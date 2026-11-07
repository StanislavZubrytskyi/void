//! Claude quota estimate for people who work in the Claude desktop app.
//!
//! The desktop app never shares quota percentages, but its transcripts keep the token usage
//! of every response and every 429 block with the exact reset time. The usage spent between
//! a window's start and its block is what 100% of that limit costs, so the current share of
//! the limit follows from the current usage. Local files only: no login, no token, no network.

use crate::usage::{Limit, Limits, Model, Record, Sample, median};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};

const FIVE_HOURS: u64 = 5 * 3600;
const WEEK: u64 = 7 * 86_400;
const HISTORY: u64 = 35 * 86_400;
pub const SESSION: &str = "claude-estimate";

#[derive(Clone)]
struct Message {
    id: String,
    time: u64,
    weight: f64,
    prompt: Option<String>,
    model: String,
}

#[derive(Clone, Copy)]
struct Block {
    time: u64,
    five_hour: bool,
    resets_at: u64,
}

#[derive(Default)]
struct FileState {
    offset: u64,
    prompt: Option<String>,
    messages: Vec<Message>,
    blocks: Vec<Block>,
}

/// Keeps parsed transcripts between refreshes and reads only the lines appended since.
#[derive(Default)]
pub struct Estimator {
    files: HashMap<PathBuf, FileState>,
}

impl Estimator {
    /// Only explicit rejected-quota events are authoritative; token costs are not account quota.
    pub fn confirmed_blocks(&mut self, now: u64) -> Vec<Record> {
        let _ = self.refresh(now);
        self.active_blocks(now)
    }

    fn active_blocks(&self, now: u64) -> Vec<Record> {
        let mut rows: Vec<_> = self
            .files
            .values()
            .flat_map(|file| file.blocks.iter())
            .filter(|block| block.time <= now && block.resets_at > now)
            .map(|block| {
                let limit = Some((100.0, block.resets_at));
                let mut row = record(
                    block.time,
                    None,
                    None,
                    if block.five_hour { limit } else { None },
                    if block.five_hour { None } else { limit },
                );
                row.sample.session_id = "claude-block".into();
                row
            })
            .collect();
        rows.sort_by_key(|row| row.time);
        rows
    }
    pub fn refresh(&mut self, now: u64) -> Vec<Record> {
        if let Some(projects) = crate::claude::config_dir().map(|dir| dir.join("projects")) {
            let mut paths = Vec::new();
            find_transcripts(&projects, 3, &mut paths);
            for path in paths {
                let state = self.files.entry(path.clone()).or_default();
                read_new_lines(&path, state);
            }
        }
        let messages: Vec<Message> = self
            .files
            .values()
            .flat_map(|file| file.messages.iter().cloned())
            .collect();
        let blocks: Vec<Block> = self
            .files
            .values()
            .flat_map(|file| file.blocks.clone())
            .collect();
        estimate(messages, &blocks, now)
    }
}

fn find_transcripts(dir: &Path, depth: u8, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() && depth > 0 {
            find_transcripts(&path, depth - 1, paths);
        } else if path.extension().is_some_and(|ext| ext == "jsonl")
            && meta
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|age| age < Duration::from_secs(HISTORY))
        {
            paths.push(path);
        }
    }
}

fn read_new_lines(path: &Path, state: &mut FileState) {
    let Ok(mut file) = File::open(path) else {
        return;
    };
    let Ok(len) = file.metadata().map(|meta| meta.len()) else {
        return;
    };
    if len < state.offset {
        *state = FileState::default(); // rewritten: parse again
    }
    if len == state.offset || file.seek(SeekFrom::Start(state.offset)).is_err() {
        return;
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return;
    }
    // Leave a partially written last line for the next refresh.
    let Some(end) = bytes.iter().rposition(|&b| b == b'\n') else {
        return;
    };
    state.offset += end as u64 + 1;
    for line in bytes[..end].split(|&b| b == b'\n') {
        parse_line(line, state);
    }
}

fn contains(line: &[u8], needle: &[u8]) -> bool {
    line.windows(needle.len()).any(|window| window == needle)
}

fn parse_line(line: &[u8], state: &mut FileState) {
    let user = contains(line, b"\"promptId\"");
    let usage = contains(line, b"\"usage\"");
    let quota = contains(line, b"\"quotaLimits\"");
    if !(user || usage || quota) {
        return;
    }
    let Ok(entry) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    let Some(time) = entry
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(crate::codex::epoch)
    else {
        return;
    };
    // Assistant entries carry no prompt id; they belong to the last user prompt in the file.
    if entry.get("type").and_then(Value::as_str) == Some("user")
        && let Some(prompt) = entry.get("promptId").and_then(Value::as_str)
    {
        state.prompt = Some(prompt.to_owned());
    }
    if let Some(quota) = entry.get("quotaLimits")
        && quota.get("status").and_then(Value::as_str) == Some("rejected")
        && let Some(resets_at) = quota.get("resetsAt").and_then(Value::as_u64)
    {
        let five_hour = match quota.get("rateLimitType").and_then(Value::as_str) {
            Some("five_hour") => true,
            Some("seven_day") => false,
            _ => return,
        };
        state.blocks.push(Block {
            time,
            five_hour,
            resets_at,
        });
    }
    if let Some(message) = entry.get("message")
        && let Some(usage) = message.get("usage")
        && let Some(id) = message.get("id").and_then(Value::as_str)
        && let Some(model) = message.get("model").and_then(Value::as_str)
        && model != "<synthetic>"
    {
        let tokens = |key| usage.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        // Relative API prices: output 5x input, cache write 1.25x, cache read 0.1x;
        // Opus-class models 5x, Sonnet 3x, Haiku 1x. Calibration absorbs the absolute scale.
        let weight = (tokens("input_tokens")
            + 5.0 * tokens("output_tokens")
            + 1.25 * tokens("cache_creation_input_tokens")
            + 0.1 * tokens("cache_read_input_tokens"))
            * tier(model)
            / 1e6;
        state.messages.push(Message {
            id: id.to_owned(),
            time,
            weight,
            prompt: state.prompt.clone(),
            model: model.to_owned(),
        });
    }
}

fn tier(model: &str) -> f64 {
    if model.contains("haiku") {
        1.0
    } else if model.contains("sonnet") {
        3.0
    } else {
        5.0
    }
}

fn estimate(messages: Vec<Message>, blocks: &[Block], now: u64) -> Vec<Record> {
    // Streaming writes one response several times; keep one entry per message id.
    let mut unique: HashMap<String, Message> = HashMap::new();
    for message in messages {
        unique.insert(message.id.clone(), message);
    }
    let mut messages: Vec<Message> = unique.into_values().collect();
    messages.sort_by_key(|message| message.time);
    let mut distinct: Vec<Block> = Vec::new();
    for block in blocks {
        if !distinct
            .iter()
            .any(|seen| seen.five_hour == block.five_hour && seen.resets_at == block.resets_at)
        {
            distinct.push(*block);
        }
    }
    let span = |five_hour: bool| if five_hour { FIVE_HOURS } else { WEEK };
    let capacity = |five_hour: bool| {
        median(
            distinct
                .iter()
                .filter(|block| block.five_hour == five_hour)
                .map(|block| {
                    let start = block.resets_at.saturating_sub(span(five_hour));
                    messages
                        .iter()
                        .filter(|m| m.time >= start && m.time <= block.time)
                        .map(|m| m.weight)
                        .sum::<f64>()
                })
                .filter(|used| *used > 0.0)
                .collect(),
        )
    };
    let (five_capacity, week_capacity) = (capacity(true), capacity(false));
    // The weekly limit resets at the same moment every week.
    let anchor = distinct
        .iter()
        .filter(|block| !block.five_hour)
        .map(|block| block.resets_at)
        .max();
    let week_end = |time: u64| {
        anchor.map(|anchor| {
            let k = (time as i64 - anchor as i64).div_euclid(WEEK as i64) + 1;
            (anchor as i64 + k * WEEK as i64) as u64
        })
    };
    let percent = |used: f64, capacity: f64| (used / capacity * 100.0).min(99.9);

    let mut records = Vec::new();
    let (mut five_end, mut five_used) = (0, 0.0);
    let (mut week, mut week_used) = (None, 0.0);
    let mut model = None;
    for message in messages
        .iter()
        .filter(|m| m.time + WEEK + FIVE_HOURS >= now && m.time <= now)
    {
        if message.time >= five_end {
            // A five-hour window opens with the first message, on a 10-minute boundary.
            let start = message.time - message.time % 600;
            five_end = distinct
                .iter()
                .filter(|block| {
                    block.five_hour
                        && block.resets_at > message.time
                        && block.resets_at <= start + FIVE_HOURS + 600
                })
                .map(|block| block.resets_at)
                .next()
                .unwrap_or(start + FIVE_HOURS);
            five_used = 0.0;
        }
        five_used += message.weight;
        let end = week_end(message.time);
        if end != week {
            week = end;
            week_used = 0.0;
        }
        week_used += message.weight;
        model = Some(message.model.clone());
        if message.time + WEEK < now || (five_capacity.is_none() && week_capacity.is_none()) {
            continue;
        }
        records.push(record(
            message.time,
            message.prompt.clone(),
            model.clone(),
            five_capacity.map(|c| (percent(five_used, c), five_end)),
            week_capacity
                .zip(week)
                .map(|(c, end)| (percent(week_used, c), end)),
        ));
    }
    // A snapshot for now keeps the data fresh while idle; active blocks are exact.
    let active = |five_hour: bool| {
        distinct
            .iter()
            .filter(|b| b.five_hour == five_hour && b.resets_at > now)
            .map(|b| (100.0, b.resets_at))
            .max_by_key(|(_, reset)| *reset)
    };
    let five_now = active(true).or_else(|| {
        five_capacity
            .filter(|_| five_end > now)
            .map(|c| (percent(five_used, c), five_end))
    });
    let week_now = active(false).or_else(|| {
        week_capacity
            .zip(week.filter(|end| *end > now))
            .map(|(c, end)| (percent(week_used, c), end))
    });
    if five_now.is_some() || week_now.is_some() {
        // The latest model lets the advice suggest a cheaper one.
        records.push(record(now, None, model, five_now, week_now));
    }
    records
}

fn record(
    time: u64,
    prompt: Option<String>,
    model: Option<String>,
    five_hour: Option<(f64, u64)>,
    seven_day: Option<(f64, u64)>,
) -> Record {
    let limit = |value: Option<(f64, u64)>| {
        value.map(|(used_percentage, resets_at)| Limit {
            used_percentage,
            resets_at,
        })
    };
    Record {
        time,
        sample: Sample {
            time,
            session_id: SESSION.into(),
            prompt_id: prompt,
            rate_limits: Some(Limits {
                five_hour: limit(five_hour),
                seven_day: limit(seven_day),
            }),
            cost: None,
            context_window: None,
            model: model.map(|id| Model { id: Some(id) }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, time: u64, weight: f64, prompt: &str) -> Message {
        Message {
            id: id.into(),
            time,
            weight,
            prompt: Some(prompt.into()),
            model: "claude-opus-5-5".into(),
        }
    }

    #[test]
    fn confirmed_data_keeps_event_time_and_excludes_expired_blocks() {
        let mut estimator = Estimator::default();
        estimator.files.insert(
            PathBuf::from("fixture"),
            FileState {
                blocks: vec![
                    Block {
                        time: 100,
                        five_hour: true,
                        resets_at: 500,
                    },
                    Block {
                        time: 50,
                        five_hour: false,
                        resets_at: 150,
                    },
                ],
                ..Default::default()
            },
        );
        let rows = estimator.active_blocks(200);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].time, 100);
        assert_eq!(rows[0].sample.session_id, "claude-block");
        assert!(estimator.active_blocks(501).is_empty());
    }

    #[test]
    fn calibrates_on_past_blocks_and_estimates_the_current_window() {
        let day = 86_400;
        let now = 100 * day;
        let mut messages = Vec::new();
        // Past window: 10 messages x 2.0 hit the five-hour limit (capacity 20).
        for i in 0..10 {
            messages.push(message(
                &format!("old{i}"),
                now - 2 * day + i * 60,
                2.0,
                "old",
            ));
        }
        // Current window: 12 prompts x 1.0, two minutes apart, the duplicate is ignored.
        for i in 0..12 {
            messages.push(message(
                &format!("new{i}"),
                now - 3600 + i * 120,
                1.0,
                &format!("p{i}"),
            ));
        }
        messages.push(message("new0", now - 3600, 1.0, "p0"));
        let past_start = now - 2 * day;
        let blocks = [Block {
            time: past_start + 9 * 60,
            five_hour: true,
            resets_at: past_start + FIVE_HOURS,
        }];

        let records = estimate(messages, &blocks, now);
        let latest = records.last().unwrap();
        assert_eq!(latest.time, now);
        assert_eq!(
            latest.sample.model.as_ref().and_then(|m| m.id.as_deref()),
            Some("claude-opus-5-5")
        );
        let five = latest
            .sample
            .rate_limits
            .as_ref()
            .unwrap()
            .five_hour
            .unwrap();
        assert!((five.used_percentage - 60.0).abs() < 1e-9); // 12 of 20
        assert_eq!(five.resets_at, now - 3600 + FIVE_HOURS);

        let forecast = crate::usage::forecast(&records, |r| r.five_hour, now).unwrap();
        // 5 pp per prompt, 2.5 pp per minute, 40 pp left.
        assert_eq!(forecast.typical_prompts, Some(8.0));
        assert!((forecast.minutes.unwrap() - 16.0).abs() < 1e-6);
    }

    #[test]
    fn unknown_without_blocks_but_exact_while_blocked() {
        let now = 1_000_000;
        let messages = vec![message("a", now - 60, 1.0, "p")];
        assert!(estimate(messages.clone(), &[], now).is_empty());
        let blocks = [Block {
            time: now - 30,
            five_hour: false,
            resets_at: now + 3600,
        }];
        let records = estimate(messages, &blocks, now);
        let week = records
            .last()
            .unwrap()
            .sample
            .rate_limits
            .as_ref()
            .unwrap()
            .seven_day
            .unwrap();
        assert_eq!((week.used_percentage, week.resets_at), (100.0, now + 3600));
        assert_eq!(
            crate::usage::blocked_until(&records, |r| r.seven_day, now),
            Some(now + 3600)
        );
    }

    #[test]
    fn reads_prompt_ids_usage_and_blocks_from_lines() {
        let mut state = FileState::default();
        for line in [
            r#"{"type":"user","promptId":"p1","timestamp":"2026-11-07T10:00:00Z","message":{"content":"text mentions \"usage\""}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-07T10:00:05Z","message":{"id":"m1","model":"claude-sonnet-5-5","usage":{"input_tokens":1000000,"output_tokens":0}}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-07T10:00:06Z","quotaLimits":{"status":"rejected","resetsAt":1794045600,"rateLimitType":"seven_day"}}"#,
        ] {
            parse_line(line.as_bytes(), &mut state);
        }
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].prompt.as_deref(), Some("p1"));
        assert!((state.messages[0].weight - 3.0).abs() < 1e-9);
        assert_eq!(state.blocks.len(), 1);
        assert!(!state.blocks[0].five_hour);
    }
}
