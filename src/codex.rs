use crate::usage::{Limit, Limits, Record, Sample};
use serde::Deserialize;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[derive(Deserialize)]
struct Event {
    timestamp: String,
    #[serde(flatten)]
    kind: EventKind,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum EventKind {
    EventMsg(Message),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    TaskStarted {
        turn_id: String,
    },
    TaskComplete,
    TurnAborted,
    TokenCount {
        rate_limits: Option<RateLimits>,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct RateLimits {
    limit_id: Option<String>,
    primary: Option<Window>,
    secondary: Option<Window>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: f64,
    window_minutes: Option<u64>,
    resets_at: Option<u64>,
}

/// Tokens Codex spent per minute, read incrementally from its session files: each
/// `token_count` event carries the session's running total, so a call spent the difference.
#[derive(Default)]
pub struct TokenLedger {
    files: std::collections::HashMap<PathBuf, LedgerFile>,
}

/// Bytes read so far, the session's running total, and (minute, tokens) per call.
type LedgerFile = (u64, u64, Vec<(u64, u64)>);

impl TokenLedger {
    pub fn refresh(&mut self, now: u64) -> Vec<(u64, u64)> {
        use std::io::Read;
        let Some(home) = codex_home() else {
            return Vec::new();
        };
        let mut files = Vec::new();
        find_recent(&home.join("sessions"), 3, &mut files);
        for (_, path) in files {
            let (offset, total, events) = self.files.entry(path.clone()).or_default();
            let Ok(mut file) = File::open(&path) else {
                continue;
            };
            let len = file.metadata().map_or(0, |meta| meta.len());
            if len < *offset {
                (*offset, *total) = (0, 0);
                events.clear();
            }
            if len == *offset || file.seek(SeekFrom::Start(*offset)).is_err() {
                continue;
            }
            let mut bytes = Vec::new();
            if file.read_to_end(&mut bytes).is_err() {
                continue;
            }
            // Leave a half-written last line for the next refresh.
            let Some(end) = bytes.iter().rposition(|&b| b == b'\n') else {
                continue;
            };
            *offset += end as u64 + 1;
            for line in bytes[..end].split(|&b| b == b'\n') {
                if !line.windows(11).any(|w| w == b"token_count") {
                    continue;
                }
                let Ok(entry) = serde_json::from_slice::<serde_json::Value>(line) else {
                    continue;
                };
                let (Some(time), Some(running)) = (
                    entry
                        .get("timestamp")
                        .and_then(serde_json::Value::as_str)
                        .and_then(epoch),
                    entry
                        .pointer("/payload/info/total_token_usage")
                        .and_then(crate::activity::codex_tokens),
                ) else {
                    continue;
                };
                if running > *total {
                    events.push((time - time % 60, running - *total));
                    *total = running;
                }
            }
        }
        let since = now.saturating_sub(8 * 86_400);
        let mut minutes = std::collections::BTreeMap::new();
        for (time, tokens) in self.files.values().flat_map(|file| file.2.iter()) {
            if *time >= since {
                *minutes.entry(*time).or_insert(0) += tokens;
            }
        }
        minutes.into_iter().collect()
    }
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join(".codex"))
        })
}

pub fn history() -> Vec<Record> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join(".codex"))
        });
    let Some(home) = home else { return Vec::new() };
    let mut files = Vec::new();
    find_recent(&home.join("sessions"), 3, &mut files);
    files.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let mut rows: Vec<_> = files
        .into_iter()
        .take(24)
        .flat_map(|(_, path)| read_session(&path))
        .collect();
    rows.sort_by_key(|row| row.time);
    rows
}

fn find_recent(path: &Path, depth: u8, files: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && depth > 0 {
            find_recent(&entry.path(), depth - 1, files);
        } else if kind.is_file()
            && depth == 0
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
            && let Ok(modified) = entry.metadata().and_then(|meta| meta.modified())
            && modified
                .elapsed()
                .is_ok_and(|age| age < Duration::from_secs(7 * 86_400))
        {
            files.push((modified, entry.path()));
        }
    }
}

fn read_session(path: &Path) -> Vec<Record> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let Ok(meta) = file.metadata() else {
        return Vec::new();
    };
    let start = meta.len().saturating_sub(512 * 1024);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut reader = BufReader::new(file);
    if start > 0 && reader.skip_until(b'\n').is_err() {
        return Vec::new();
    }
    parse(
        reader,
        &path.file_name().unwrap_or_default().to_string_lossy(),
    )
}

fn parse(reader: impl BufRead, session: &str) -> Vec<Record> {
    let mut rows = Vec::new();
    let mut turn = None;
    let mut pending: Option<Record> = None;
    for line in reader.lines().map_while(Result::ok) {
        let Ok(Event {
            timestamp,
            kind: EventKind::EventMsg(message),
        }) = serde_json::from_str(&line)
        else {
            continue;
        };
        match message {
            Message::TaskStarted { turn_id } => {
                rows.extend(pending.take());
                turn = Some(turn_id);
            }
            Message::TaskComplete | Message::TurnAborted => {
                rows.extend(pending.take());
                turn = None;
            }
            Message::TokenCount {
                rate_limits: Some(rates),
            } if rates.limit_id.as_deref().unwrap_or("codex") == "codex" => {
                let Some(time) = epoch(&timestamp) else {
                    continue;
                };
                let mut limits = Limits::default();
                for item in [rates.primary, rates.secondary].into_iter().flatten() {
                    let Some(resets_at) = item.resets_at else {
                        continue;
                    };
                    let limit = Limit {
                        used_percentage: item.used_percent,
                        resets_at,
                    };
                    match item.window_minutes {
                        Some(300) => limits.five_hour = Some(limit),
                        Some(10_080) => limits.seven_day = Some(limit),
                        _ => {}
                    }
                }
                if limits.five_hour.is_none() && limits.seven_day.is_none() {
                    continue;
                }
                let row = Record {
                    time,
                    sample: Sample {
                        time,
                        session_id: session.to_owned(),
                        prompt_id: turn.clone(),
                        rate_limits: Some(limits),
                        cost: None,
                        context_window: None,
                        model: None,
                    },
                };
                if turn.is_some() {
                    pending = Some(row);
                } else {
                    rows.push(row);
                }
            }
            _ => {}
        }
    }
    rows.extend(pending);
    rows
}

// UTC RFC3339 (`Z` or `+00:00`), with optional fractional seconds.
pub fn epoch(value: &str) -> Option<u64> {
    let value = value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix("+00:00"))?;
    let (date, clock) = value.split_once('T')?;
    let mut date = date.split('-').map(str::parse::<u64>);
    let year = date.next()?.ok()?;
    let month = date.next()?.ok()?;
    let day = date.next()?.ok()?;
    let mut clock = clock.split('.').next()?.split(':').map(str::parse::<u64>);
    let hour = clock.next()?.ok()?;
    let minute = clock.next()?.ok()?;
    let second = clock.next()?.ok()?;
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let leaps = |year: u64| year / 4 - year / 100 + year / 400;
    let leap = leaps(year) != leaps(year - 1);
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let month = (month - 1) as usize;
    if day == 0 || day > months[month] {
        return None;
    }
    let days = (year - 1970) * 365 + leaps(year - 1) - leaps(1969)
        + months[..month].iter().sum::<u64>()
        + day
        - 1;
    Some(days * 86400 + hour * 3600 + minute * 60 + second)
}

/// A managed connection; Drop always reaps the helper, including timeout paths.
pub struct Connection {
    child: std::process::Child,
    input: std::process::ChildStdin,
    receive: std::sync::mpsc::Receiver<serde_json::Value>,
    next_id: u64,
}
impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Connection {
    pub fn open() -> Option<Self> {
        use std::process::{Command, Stdio};
        let mut command = Command::new(codex_binary()?);
        command
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().ok()?;
        let input = child.stdin.take()?;
        let output = child.stdout.take()?;
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                    && send.send(value).is_err()
                {
                    break;
                }
            }
        });
        let mut connection = Self {
            child,
            input,
            receive,
            next_id: 1,
        };
        connection.write(serde_json::json!({"method":"initialize", "id":0,
            "params":{"clientInfo":{"name":"void", "version":env!("CARGO_PKG_VERSION")}}}))?;
        connection.response(0)?.get("result")?;
        connection.write(serde_json::json!({"method":"initialized"}))?;
        Some(connection)
    }
    fn write(&mut self, value: serde_json::Value) -> Option<()> {
        use std::io::Write;
        writeln!(self.input, "{value}").ok()
    }
    fn response(&self, id: u64) -> Option<serde_json::Value> {
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        loop {
            let value = self
                .receive
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .ok()?;
            if value.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
                return Some(value);
            }
        }
    }
    pub fn snapshot(&mut self) -> Option<Record> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(serde_json::json!({"method":"account/rateLimits/read", "id":id}))?;
        snapshot_from_response(self.response(id)?, crate::usage::now())
    }
    pub fn updates(&self) -> Option<Record> {
        self.receive
            .try_iter()
            .filter_map(|value| {
                if value.get("method")?.as_str()? != "account/rateLimits/updated" {
                    return None;
                }
                snapshot_from_response(
                    serde_json::json!({"result":value.get("params")?}),
                    crate::usage::now(),
                )
            })
            .last()
    }
}
pub fn account_snapshot() -> Option<Record> {
    Connection::open()?.snapshot()
}

fn codex_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("VOID_CODEX_BIN").map(PathBuf::from)
        && path.is_absolute()
        && path.is_file()
    {
        return Some(path);
    }
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    if let Some(path) = std::env::var_os("PATH")
        && let Some(binary) = std::env::split_paths(&path)
            .filter(|entry| entry.is_absolute())
            .map(|entry| entry.join(name))
            .find(|path| path.is_file())
    {
        return Some(binary);
    }
    #[cfg(windows)]
    {
        let directory = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("OpenAI/Codex/bin");
        fs::read_dir(directory)
            .ok()?
            .flatten()
            .map(|entry| entry.path().join(name))
            .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
            .max_by_key(|(time, _)| *time)
            .map(|(_, path)| path)
    }
    #[cfg(not(windows))]
    None
}

fn snapshot_from_response(response: serde_json::Value, time: u64) -> Option<Record> {
    let result = response.get("result")?;
    let snapshot = result
        .pointer("/rateLimitsByLimitId/codex")
        .or_else(|| result.get("rateLimits"))?;
    if snapshot
        .get("limitId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|id| id != "codex")
    {
        return None;
    }
    let mut limits = Limits::default();
    for key in ["primary", "secondary"] {
        let Some(window) = snapshot.get(key) else {
            continue;
        };
        let Some(used_percentage) = window
            .get("usedPercent")
            .and_then(serde_json::Value::as_f64)
        else {
            continue;
        };
        let Some(resets_at) = window.get("resetsAt").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        let limit = Limit {
            used_percentage,
            resets_at,
        };
        match window
            .get("windowDurationMins")
            .and_then(serde_json::Value::as_u64)
        {
            Some(300) => limits.five_hour = Some(limit),
            Some(10_080) => limits.seven_day = Some(limit),
            _ => {}
        }
    }
    if limits.five_hour.is_none() && limits.seven_day.is_none() {
        return None;
    }
    Some(Record {
        time,
        sample: Sample {
            time,
            session_id: "codex-account".into(),
            prompt_id: None,
            rate_limits: Some(limits),
            cost: None,
            context_window: None,
            model: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_codex_quota_and_groups_the_turn() {
        let input = r#"
{"timestamp":"2024-02-29T00:00:00.025Z","type":"event_msg","payload":{"type":"task_started","turn_id":"a"}}
{"timestamp":"2024-02-29T00:00:01Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","primary":{"window_minutes":300,"used_percent":30,"resets_at":1709169900}}}}
{"timestamp":"2024-02-29T00:00:02Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"codex","secondary":{"window_minutes":10080,"used_percent":100,"resets_at":1709769600}}}}
{"timestamp":"2024-02-29T00:00:03Z","type":"event_msg","payload":{"type":"token_count","rate_limits":{"limit_id":"other","primary":{"window_minutes":300,"used_percent":90,"resets_at":1709169900}}}}
"#;
        let rows = parse(input.as_bytes(), "test");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].time, 1709164802);
        assert_eq!(rows[0].sample.prompt_id.as_deref(), Some("a"));
        assert_eq!(
            crate::usage::blocked_until(&rows, |limits| limits.seven_day, 1709251200),
            Some(1709769600)
        );
        assert_eq!(
            crate::usage::blocked_until(&rows, |limits| limits.seven_day, 1709769600),
            None
        );
    }
}
