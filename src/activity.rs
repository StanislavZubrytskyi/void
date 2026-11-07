//! Which agent sessions are working right now, from the tails of their local transcripts.
//! Only the project folder name, the turn start and the current tool name leave this module.

use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

/// A turn with no transcript writes for this long is treated as finished or abandoned.
// shortcut: a single tool running silently longer than this drops off; upgrade if builds that long are common.
const STALE: Duration = Duration::from_secs(15 * 60);
const TAIL: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// The transcript file: stable across scans, so a turn keeps its start time.
    pub id: PathBuf,
    pub project: String,
    /// When the current turn started (or the oldest moment the tail still shows).
    pub since: u64,
    /// The tool the agent is running now, if any.
    pub tool: Option<String>,
}

/// Busy sessions for Claude and Codex, in that order, the longest-running first.
pub fn scan() -> [Vec<Session>; 2] {
    let home = || {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    };
    let mut claude = Vec::new();
    if let Some(projects) = crate::claude::config_dir().map(|dir| dir.join("projects")) {
        // Main transcripts sit directly in a project folder; subagent ones are nested deeper.
        for path in recent(&projects, 1) {
            claude.extend(
                tail_lines(&path)
                    .and_then(|lines| claude_turn(&lines))
                    .map(|session| Session {
                        id: path,
                        ..session
                    }),
            );
        }
    }
    let mut codex = Vec::new();
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join(".codex")));
    if let Some(sessions) = codex_home.map(|home| home.join("sessions")) {
        for path in recent(&sessions, 3) {
            let project = first_line(&path).and_then(|line| codex_project(&line));
            codex.extend(tail_lines(&path).and_then(|lines| codex_turn(&lines)).map(
                |mut session| {
                    session.project = project.unwrap_or_default();
                    session
                },
            ));
        }
    }
    for sessions in [&mut claude, &mut codex] {
        sessions.sort_by_key(|session| session.since);
    }
    [claude, codex]
}

/// `.jsonl` files exactly `depth` folders below `dir`, written within STALE.
fn recent(dir: &Path, depth: u8) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() && depth > 0 {
            found.extend(recent(&path, depth - 1));
        } else if kind.is_file()
            && depth == 0
            && path.extension().is_some_and(|ext| ext == "jsonl")
            && entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|at| SystemTime::now().duration_since(at).ok())
                .is_some_and(|age| age < STALE)
        {
            found.push(path);
        }
    }
    found
}

fn tail_lines(path: &Path) -> Option<Vec<String>> {
    let mut file = File::open(path).ok()?;
    let start = file.metadata().ok()?.len().saturating_sub(TAIL);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // cut mid-line
    }
    Some(lines)
}

fn first_line(path: &Path) -> Option<String> {
    let mut line = String::new();
    BufReader::new(File::open(path).ok()?)
        .take(64 * 1024)
        .read_line(&mut line)
        .ok()?;
    Some(line)
}

fn folder_name(cwd: &str) -> String {
    cwd.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(cwd)
        .to_owned()
}

fn timestamp(entry: &Value) -> Option<u64> {
    entry
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(crate::codex::epoch)
}

/// Replays a Claude Code transcript tail: a prompt or a pending tool call means the turn is
/// running; `end_turn` (or an interrupt) ends it.
fn claude_turn(lines: &[String]) -> Option<Session> {
    let (mut busy, mut since, mut tool, mut cwd) = (false, 0, None, None);
    for line in lines {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(dir) = entry.get("cwd").and_then(Value::as_str) {
            cwd = Some(dir.to_owned());
        }
        let Some(time) = timestamp(&entry) else {
            continue;
        };
        let content = entry.pointer("/message/content");
        match entry.get("type").and_then(Value::as_str) {
            Some("user") if entry.get("isMeta").and_then(Value::as_bool) != Some(true) => {
                let text = match content {
                    Some(Value::String(text)) => Some(text.as_str()),
                    Some(Value::Array(blocks)) => blocks
                        .iter()
                        .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                        .and_then(|block| block.get("text")?.as_str()),
                    _ => None,
                };
                match text {
                    Some(text) if text.starts_with("[Request interrupted") => busy = false,
                    // Slash commands like /exit run locally, without a model turn.
                    Some(text)
                        if text.starts_with("<command-") || text.starts_with("<local-command") => {}
                    // A tool result continues the turn; a new prompt starts one.
                    None => tool = None,
                    Some(_) => {
                        if !busy {
                            since = time;
                        }
                        busy = true;
                        tool = None;
                    }
                }
            }
            Some("assistant") => {
                match entry
                    .pointer("/message/stop_reason")
                    .and_then(Value::as_str)
                {
                    Some("end_turn" | "max_tokens" | "stop_sequence" | "refusal") => busy = false,
                    _ => {
                        if !busy {
                            since = time;
                        }
                        busy = true;
                        if let Some(name) = content
                            .and_then(Value::as_array)
                            .and_then(|blocks| {
                                blocks.iter().rev().find(|block| {
                                    block.get("type").and_then(Value::as_str) == Some("tool_use")
                                })
                            })
                            .and_then(|block| block.get("name")?.as_str())
                        {
                            tool = Some(name.to_owned());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    busy.then(|| Session {
        id: PathBuf::new(),
        project: cwd.as_deref().map(folder_name).unwrap_or_default(),
        since,
        tool,
    })
}

fn codex_project(line: &str) -> Option<String> {
    let entry: Value = serde_json::from_str(line).ok()?;
    (entry.get("type")?.as_str()? == "session_meta")
        .then(|| entry.pointer("/payload/cwd")?.as_str().map(folder_name))
        .flatten()
}

/// Replays a Codex rollout tail: `task_started` opens a turn, `task_complete` or
/// `turn_aborted` closes it.
fn codex_turn(lines: &[String]) -> Option<Session> {
    let (mut busy, mut since, mut tool) = (false, 0, None);
    for line in lines {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = entry.pointer("/payload/type").and_then(Value::as_str);
        match (entry.get("type").and_then(Value::as_str), kind) {
            (Some("event_msg"), Some("task_started")) => {
                busy = true;
                since = timestamp(&entry).unwrap_or(0);
                tool = None;
            }
            (Some("event_msg"), Some("task_complete" | "turn_aborted")) => busy = false,
            (Some("response_item"), Some("function_call" | "custom_tool_call")) => {
                tool = entry
                    .pointer("/payload/name")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            (Some("response_item"), Some("function_call_output" | "custom_tool_call_output")) => {
                tool = None;
            }
            _ => {}
        }
    }
    busy.then(|| Session {
        id: PathBuf::new(),
        project: String::new(),
        since,
        tool,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(rows: &[Value]) -> Vec<String> {
        rows.iter().map(Value::to_string).collect()
    }

    #[test]
    fn claude_turn_runs_from_prompt_through_tools_until_end_turn() {
        let user = |t: &str, content: Value| serde_json::json!({"type":"user","timestamp":t,"cwd":"D:\\work\\Void","message":{"content":content}});
        let assistant = |t: &str, stop: &str, tool: Option<&str>| {
            let content = match tool {
                Some(name) => serde_json::json!([{"type":"tool_use","name":name}]),
                None => serde_json::json!([{"type":"text","text":"done"}]),
            };
            serde_json::json!({"type":"assistant","timestamp":t,"message":{"stop_reason":stop,"content":content}})
        };
        let prompt = user("2026-11-07T10:00:00Z", "fix it".into());
        let call = assistant("2026-11-07T10:00:05Z", "tool_use", Some("Bash"));
        let result = user(
            "2026-11-07T10:01:00Z",
            serde_json::json!([{"type":"tool_result"}]),
        );
        let done = assistant("2026-11-07T10:02:00Z", "end_turn", None);

        let running = claude_turn(&lines(&[prompt.clone(), call.clone()])).unwrap();
        assert_eq!(running.project, "Void");
        assert_eq!(running.tool.as_deref(), Some("Bash"));
        assert_eq!(
            running.since,
            crate::codex::epoch("2026-11-07T10:00:00Z").unwrap()
        );
        // The tool returned: still the same turn, now thinking.
        let thinking =
            claude_turn(&lines(&[prompt.clone(), call.clone(), result.clone()])).unwrap();
        assert_eq!((thinking.since, thinking.tool), (running.since, None));
        assert!(claude_turn(&lines(&[prompt.clone(), call.clone(), result, done])).is_none());
        let stop = user(
            "2026-11-07T10:00:09Z",
            "[Request interrupted by user]".into(),
        );
        assert!(claude_turn(&lines(&[prompt, call, stop])).is_none());
    }

    #[test]
    fn codex_turn_follows_task_events_and_tool_calls() {
        let event = |t: &str, kind: &str| serde_json::json!({"timestamp":t,"type":"event_msg","payload":{"type":kind}});
        let call = serde_json::json!({"timestamp":"2026-11-07T10:00:03Z","type":"response_item",
            "payload":{"type":"function_call","name":"shell"}});
        let started = event("2026-11-07T10:00:00Z", "task_started");
        let running = codex_turn(&lines(&[started.clone(), call.clone()])).unwrap();
        assert_eq!(running.tool.as_deref(), Some("shell"));
        let done = event("2026-11-07T10:05:00Z", "task_complete");
        assert!(codex_turn(&lines(&[started, call, done])).is_none());
        let meta = r#"{"type":"session_meta","payload":{"cwd":"/home/me/api/"}}"#;
        assert_eq!(codex_project(meta).as_deref(), Some("api"));
    }
}
