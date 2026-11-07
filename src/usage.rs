use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_PAYLOAD: u64 = 1024 * 1024;
const HISTORY_BYTES: u64 = 2 * 1024 * 1024;
const FRESH_SECONDS: u64 = 30 * 60;

#[derive(Clone, Copy, Deserialize, Serialize)]
pub struct Limit {
    pub used_percentage: f64,
    pub resets_at: u64,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Limits {
    pub five_hour: Option<Limit>,
    pub seven_day: Option<Limit>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Cost {
    pub total_cost_usd: Option<f64>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Context {
    pub used_percentage: Option<f64>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Model {
    pub id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Sample {
    #[serde(default, skip_deserializing)]
    pub time: u64,
    pub session_id: String,
    pub prompt_id: Option<String>,
    pub rate_limits: Option<Limits>,
    pub cost: Option<Cost>,
    pub context_window: Option<Context>,
    pub model: Option<Model>,
}

#[derive(Clone, Deserialize)]
pub struct Record {
    pub time: u64,
    #[serde(flatten)]
    pub sample: Sample,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn data_dir() -> io::Result<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    std::env::var_os("VOID_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".void")))
        .ok_or_else(|| io::Error::other("Void data directory unavailable"))
}

fn history_path() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("usage.jsonl"))
}

// The status line must never break Claude Code: always print and exit successfully.
pub fn capture() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    let model = io::stdin()
        .lock()
        .take(MAX_PAYLOAD + 1)
        .read_to_end(&mut input)
        .ok()
        .and_then(|_| record(&input, &history_path().ok()?).ok())
        .flatten();
    // Keep the user's own status line visible when Void was connected over it.
    if let Some(output) = crate::statusline::previous_output(&input) {
        io::stdout().write_all(&output)?;
        return Ok(());
    }
    match model {
        Some(model) => println!("Void · {model}"),
        None => println!("Void"),
    }
    Ok(())
}

/// Append one sample to the history at `path`. Returns the model id, if any.
fn record(input: &[u8], path: &Path) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if input.len() as u64 > MAX_PAYLOAD {
        return Err(io::Error::other("status line payload too large").into());
    }
    let sample: Sample = serde_json::from_slice(input)?;
    let model = sample.model.as_ref().and_then(|model| model.id.clone());
    append(sample, path)?;
    Ok(model)
}

/// Append a polled account snapshot to the local history.
pub fn save(sample: Sample) -> Result<(), Box<dyn std::error::Error>> {
    append(sample, &history_path()?)
}

fn append(mut sample: Sample, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(path.parent().unwrap())?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock()?;
    sample.time = now();
    let len = file.metadata()?.len();
    // Keep a bounded local history. The exclusive lock covers compaction and append.
    if len >= HISTORY_BYTES * 2 {
        file.seek(SeekFrom::End(-(HISTORY_BYTES as i64)))?;
        let mut reader = BufReader::new(&mut file);
        reader.skip_until(b'\n')?;
        let mut tail = Vec::new();
        reader.read_to_end(&mut tail)?;
        file.rewind()?;
        file.write_all(&tail)?;
        file.set_len(tail.len() as u64)?;
    }
    file.seek(SeekFrom::End(0))?;
    let mut line = serde_json::to_vec(&sample)?;
    line.push(b'\n');
    file.write_all(&line)?;
    Ok(())
}

pub fn history() -> io::Result<Vec<Record>> {
    let file = match fs::File::open(history_path()?) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    file.lock_shared()?;
    BufReader::new(file)
        .lines()
        .filter_map(|line| match line {
            Ok(line) => serde_json::from_str(&line).ok().map(Ok),
            Err(error) => Some(Err(error)),
        })
        .collect()
}

pub type SelectLimit = fn(&Limits) -> Option<Limit>;

pub struct Forecast {
    pub limit: Limit,
    pub typical_prompts: Option<f64>,
    pub minutes: Option<f64>,
}

pub fn median(mut values: Vec<f64>) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    let upper = *values.get(mid)?;
    Some(if values.len().is_multiple_of(2) {
        (upper + values[mid - 1]) / 2.0
    } else {
        upper
    })
}

fn window(record: &Record, select: SelectLimit) -> Option<Limit> {
    let limit = select(record.sample.rate_limits.as_ref()?)?;
    ((0.0..=100.0).contains(&limit.used_percentage) && limit.resets_at > record.time)
        .then_some(limit)
}

pub fn last_window(history: &[Record], select: SelectLimit) -> Option<(Limit, u64)> {
    history.iter().rev().find_map(|record| {
        let limit = select(record.sample.rate_limits.as_ref()?)?;
        (0.0..=100.0)
            .contains(&limit.used_percentage)
            .then_some((limit, record.time))
    })
}

pub fn blocked_until(history: &[Record], select: SelectLimit, time: u64) -> Option<u64> {
    let (limit, _) = last_window(history, select)?;
    (limit.used_percentage >= 100.0 && limit.resets_at > time).then_some(limit.resets_at)
}

// Status line can render several times per prompt. Keep its last observed snapshot.
fn prompts<'a>(history: impl IntoIterator<Item = &'a Record>) -> Vec<&'a Record> {
    let mut result: Vec<&Record> = Vec::new();
    for row in history {
        if let Some(id) = &row.sample.prompt_id
            && result.last().is_some_and(|last| {
                last.sample.session_id == row.sample.session_id
                    && last.sample.prompt_id.as_ref() == Some(id)
            })
        {
            *result.last_mut().unwrap() = row;
            continue;
        }
        result.push(row);
    }
    result
}

pub fn forecast(history: &[Record], select: SelectLimit, time: u64) -> Option<Forecast> {
    let (limit, latest) = last_window(history, select)?;
    if time >= limit.resets_at || time.saturating_sub(latest) > FRESH_SECONDS {
        return None;
    }
    // Account polls and parallel sessions interleave in one file: pair rows within each session.
    let mut rows: Vec<&Record> = history.iter().collect();
    rows.sort_by(|a, b| a.sample.session_id.cmp(&b.sample.session_id)); // stable: keeps time order
    let prompts = prompts(rows);
    let mut pairs: Vec<_> = prompts
        .windows(2)
        .map(|pair| (pair[0], pair[1]))
        .filter(|(before, after)| before.sample.session_id == after.sample.session_id)
        .collect();
    pairs.sort_by_key(|(before, _)| before.time);
    let mut changes = Vec::new();
    let mut pace = Vec::new();
    for &(before, after) in pairs.iter().rev() {
        if time.saturating_sub(before.time) > 7 * 86400 || pace.len().max(changes.len()) >= 40 {
            break;
        }
        let elapsed = after.time.saturating_sub(before.time);
        // Polled account snapshots carry no prompt id: they give the pace, not a per-prompt cost.
        let snapshots = before.sample.prompt_id.is_none() && after.sample.prompt_id.is_none();
        if (after.sample.prompt_id.is_none() && !snapshots) || elapsed > FRESH_SECONDS {
            continue;
        }
        let Some((a, b)) = window(before, select).zip(window(after, select)) else {
            continue;
        };
        // API reset times can jitter by a second between polls of the same window.
        if a.resets_at.abs_diff(b.resets_at) > 60 || b.used_percentage < a.used_percentage {
            continue;
        }
        let delta = b.used_percentage - a.used_percentage;
        if !snapshots {
            changes.push(delta);
        }
        if elapsed > 0 {
            pace.push((delta, elapsed as f64 / 60.0));
        }
    }
    let remaining = 100.0 - limit.used_percentage;
    let typical_prompts = (changes.len() >= 5)
        .then(|| changes.iter().sum::<f64>() / changes.len() as f64)
        .filter(|rate| *rate > 0.0)
        .map(|rate| (remaining / rate).floor());
    let pace = (pace.len() >= 5)
        .then(|| pace.iter().map(|p| p.0).sum::<f64>() / pace.iter().map(|p| p.1).sum::<f64>())
        .filter(|rate| *rate > 0.0);
    Some(Forecast {
        limit,
        typical_prompts,
        minutes: if remaining == 0.0 {
            Some(0.0)
        } else {
            pace.map(|rate| remaining / rate)
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_rejects_garbage_and_appends_valid_samples() {
        let dir = std::env::temp_dir().join(format!("void-test-{}", std::process::id()));
        let path = dir.join("usage.jsonl");
        assert!(record(b"garbage", &path).is_err());
        assert!(record(&vec![b' '; MAX_PAYLOAD as usize + 1], &path).is_err());
        let model = record(br#"{"session_id":"s","model":{"id":"m"}}"#, &path).unwrap();
        assert_eq!(model.as_deref(), Some("m"));
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn polled_snapshots_give_pace_without_prompt_counts() {
        let time = 1_800_000_000;
        let rows: Vec<Record> = (0_u64..=8)
            .map(|i| {
                serde_json::from_value(serde_json::json!({
                    "time": time - (8 - i) * 120,
                    "session_id": "claude-account",
                    // reset time jitters by a second between polls
                    "rate_limits": { "five_hour": {
                        "used_percentage": 50.0 + i as f64 * 0.5,
                        "resets_at": time + 3600 + i % 2
                    } }
                }))
                .unwrap()
            })
            .collect();
        let estimate = forecast(&rows, |r| r.five_hour, time).unwrap();
        assert_eq!(estimate.typical_prompts, None);
        // 46 pp left at 0.25 pp/min
        assert!((estimate.minutes.unwrap() - 184.0).abs() < 0.01);
    }

    #[test]
    fn interleaved_account_polls_do_not_break_prompt_pairs() {
        let time = 1_800_000_000;
        let row = |at: u64, session: &str, prompt: Option<String>, used: f64| -> Record {
            serde_json::from_value(serde_json::json!({
                "time": at,
                "session_id": session,
                "prompt_id": prompt,
                "rate_limits": { "five_hour": { "used_percentage": used, "resets_at": time + 7200 } }
            }))
            .unwrap()
        };
        let rows: Vec<Record> = (0_u64..=8)
            .flat_map(|i| {
                let at = time - 300 - (8 - i) * 120;
                let used = 50.0 + i as f64;
                [
                    row(at, "cli", Some(i.to_string()), used),
                    row(at + 30, "claude-account", None, used),
                ]
            })
            .collect();
        let estimate = forecast(&rows, |r| r.five_hour, time).unwrap();
        // 42 pp left at 1 pp per prompt, and 1 pp per 2 minutes in both sessions.
        assert_eq!(estimate.typical_prompts, Some(42.0));
        assert!((estimate.minutes.unwrap() - 84.0).abs() < 0.01);
    }
}
