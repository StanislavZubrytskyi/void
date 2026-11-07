//! Which agent sessions are working right now, and how their last turn went, from the tails
//! of their local transcripts: project folder, turn start and end, current tool, prompt text.

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
/// How far back to look, once, for the prompt of a turn that began before the usual tail.
const DEEP_TAIL: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// The transcript file: stable across scans, so a turn keeps its start time.
    pub id: PathBuf,
    pub project: String,
    /// When the current turn started (or the oldest moment the tail still shows).
    pub since: u64,
    /// The tool the agent is running now, if any.
    pub tool: Option<String>,
    /// Still working; otherwise this describes the last finished turn.
    pub busy: bool,
    /// The last transcript moment of the turn.
    pub end: u64,
    /// What the user asked for in this turn, cut to PROMPT_CHARS.
    pub prompt: Option<String>,
    /// Tokens the turn has spent so far: fresh input, cache writes and output, without cache reads.
    pub tokens: u64,
}

const PROMPT_CHARS: usize = 1000;

/// Tokens one Claude response spent: fresh input, cache writes and output. Cache reads repeat the
/// whole conversation on every call and would dwarf the real work, so they are left out.
fn claude_tokens(usage: &Value) -> u64 {
    [
        "input_tokens",
        "cache_creation_input_tokens",
        "output_tokens",
    ]
    .iter()
    .filter_map(|key| usage.get(*key)?.as_u64())
    .sum()
}

/// The same measure from a Codex token total: uncached input plus output.
pub fn codex_tokens(total: &Value) -> Option<u64> {
    let field = |key: &str| total.get(key).and_then(Value::as_u64).unwrap_or(0);
    total.get("input_tokens")?;
    Some(
        field("input_tokens").saturating_sub(field("cached_input_tokens")) + field("output_tokens"),
    )
}

fn prompt_text(text: &str) -> String {
    text.trim().chars().take(PROMPT_CHARS).collect()
}

/// The last turn of one transcript: 0 is Claude, 1 is Codex.
pub fn read_turn(agent: usize, path: &Path) -> Option<Session> {
    read_turn_from(agent, path, TAIL)
}

/// The same, reading much further back: for a long turn whose prompt left the usual tail.
pub fn read_turn_deep(agent: usize, path: &Path) -> Option<Session> {
    read_turn_from(agent, path, DEEP_TAIL)
}

fn read_turn_from(agent: usize, path: &Path, tail: u64) -> Option<Session> {
    let lines = tail_lines(path, tail)?;
    let mut turn = if agent == 0 {
        claude_turn(&lines)?
    } else {
        let mut turn = codex_turn(&lines)?;
        turn.project = first_line(path)
            .and_then(|line| codex_project(&line))
            .unwrap_or_default();
        turn
    };
    turn.id = path.to_path_buf();
    Some(turn)
}

/// Busy sessions for Claude and Codex, in that order, the longest-running first.
pub fn scan() -> [Vec<Session>; 2] {
    let home = || {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    };
    let busy = |agent: usize, paths: Vec<PathBuf>| -> Vec<Session> {
        paths
            .iter()
            .filter_map(|path| read_turn(agent, path))
            .filter(|turn| turn.busy)
            .collect()
    };
    // Main Claude transcripts sit directly in a project folder; subagent ones are nested deeper.
    let mut claude = crate::claude::config_dir()
        .map(|dir| busy(0, recent(&dir.join("projects"), 1)))
        .unwrap_or_default();
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join(".codex")));
    let mut codex = codex_home
        .map(|home| busy(1, recent(&home.join("sessions"), 3)))
        .unwrap_or_default();
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

fn tail_lines(path: &Path, tail: u64) -> Option<Vec<String>> {
    let mut file = File::open(path).ok()?;
    let start = file.metadata().ok()?.len().saturating_sub(tail);
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
    let (mut end, mut prompt) = (0, None);
    // Streaming writes one response several times: count each message id once.
    let mut spent: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
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
                    // An interrupted turn says nothing about how long such work takes.
                    Some(text) if text.starts_with("[Request interrupted") => {
                        busy = false;
                        since = 0;
                    }
                    // Slash commands like /exit run locally, without a model turn.
                    Some(text)
                        if text.starts_with("<command-") || text.starts_with("<local-command") => {}
                    // A tool result continues the turn; a new prompt starts one.
                    None => {
                        tool = None;
                        end = time;
                    }
                    Some(text) => {
                        if !busy {
                            since = time;
                            prompt = Some(prompt_text(text));
                            spent.clear();
                        }
                        busy = true;
                        tool = None;
                        end = time;
                    }
                }
            }
            Some("assistant") => {
                if let Some(id) = entry.pointer("/message/id").and_then(Value::as_str)
                    && let Some(usage) = entry.pointer("/message/usage")
                {
                    spent.insert(id.to_owned(), claude_tokens(usage));
                }
                match entry
                    .pointer("/message/stop_reason")
                    .and_then(Value::as_str)
                {
                    Some("end_turn" | "max_tokens" | "stop_sequence" | "refusal") => {
                        busy = false;
                        end = time;
                    }
                    _ => {
                        if !busy {
                            since = time;
                            prompt = None; // the tail starts mid-turn: the prompt is out of view
                            spent.retain(|id, _| {
                                entry.pointer("/message/id").and_then(Value::as_str) == Some(id)
                            });
                        }
                        busy = true;
                        end = time;
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
    (since > 0).then(|| Session {
        id: PathBuf::new(),
        project: cwd.as_deref().map(folder_name).unwrap_or_default(),
        since,
        tool,
        busy,
        end,
        prompt,
        tokens: spent.values().sum(),
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
    let (mut end, mut prompt) = (0, None);
    // The session keeps a running token total; a turn spent the difference since it started.
    let (mut total, mut base) = (None::<u64>, None::<u64>);
    for line in lines {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = entry.pointer("/payload/type").and_then(Value::as_str);
        if busy && let Some(time) = timestamp(&entry) {
            end = time;
        }
        match (entry.get("type").and_then(Value::as_str), kind) {
            (Some("event_msg"), Some("task_started")) => {
                busy = true;
                since = timestamp(&entry).unwrap_or(0);
                end = since;
                tool = None;
                prompt = None;
                base = total;
            }
            (Some("event_msg"), Some("token_count")) => {
                if let Some(now) = entry
                    .pointer("/payload/info/total_token_usage")
                    .and_then(codex_tokens)
                {
                    // A tail that starts mid-turn: count from this call on.
                    let last = entry
                        .pointer("/payload/info/last_token_usage")
                        .and_then(codex_tokens)
                        .unwrap_or(0);
                    base.get_or_insert(now.saturating_sub(last));
                    total = Some(now);
                }
            }
            (Some("event_msg"), Some("task_complete")) => busy = false,
            (Some("event_msg"), Some("turn_aborted")) => {
                busy = false;
                since = 0;
            }
            // The first user text that is not injected context (AGENTS.md, environment) is the ask.
            (Some("response_item"), Some("message"))
                if busy
                    && prompt.is_none()
                    && entry.pointer("/payload/role").and_then(Value::as_str) == Some("user") =>
            {
                prompt = entry
                    .pointer("/payload/content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| item.get("text")?.as_str())
                    .find(|text| !text.starts_with('<') && !text.starts_with("# AGENTS.md"))
                    .map(prompt_text);
            }
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
    (since > 0).then(|| Session {
        id: PathBuf::new(),
        project: String::new(),
        since,
        tool,
        busy,
        end,
        prompt,
        tokens: total
            .zip(base)
            .map_or(0, |(total, base)| total.saturating_sub(base)),
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
            serde_json::json!({"type":"assistant","timestamp":t,"message":{"id":t,"stop_reason":stop,"content":content,
                "usage":{"input_tokens":10,"cache_creation_input_tokens":100,"cache_read_input_tokens":5000,"output_tokens":40}}})
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
        assert_eq!(running.prompt.as_deref(), Some("fix it"));
        let finished = claude_turn(&lines(&[prompt.clone(), call.clone(), result, done])).unwrap();
        assert!(!finished.busy);
        // Two responses of 150 each; cache reads are not counted.
        assert_eq!((running.tokens, finished.tokens), (150, 300));
        assert_eq!(finished.end - finished.since, 120);
        assert_eq!(finished.prompt.as_deref(), Some("fix it"));
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
        let ask = serde_json::json!({"timestamp":"2026-11-07T10:00:01Z","type":"response_item",
            "payload":{"type":"message","role":"user","content":[
                {"type":"input_text","text":"# AGENTS.md instructions"},
                {"type":"input_text","text":"<environment_context>"},
                {"type":"input_text","text":"add a login page"}]}});
        let running = codex_turn(&lines(&[started.clone(), ask.clone(), call.clone()])).unwrap();
        assert!(running.busy);
        assert_eq!(running.tool.as_deref(), Some("shell"));
        assert_eq!(running.prompt.as_deref(), Some("add a login page"));
        let count = |input: u64, output: u64| {
            serde_json::json!({"timestamp":"2026-11-07T10:00:04Z",
            "type":"event_msg","payload":{"type":"token_count","info":{
                "total_token_usage":{"input_tokens":input,"cached_input_tokens":input / 2,"output_tokens":output},
                "last_token_usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0}}}})
        };
        let before = count(1000, 100); // the previous turn's running total: 600
        let done = event("2026-11-07T10:05:00Z", "task_complete");
        let finished = codex_turn(&lines(&[
            before,
            started.clone(),
            ask,
            call.clone(),
            count(3000, 300), // 1800 now
            done,
        ]))
        .unwrap();
        assert_eq!(finished.tokens, 1200);
        assert_eq!((finished.busy, finished.end - finished.since), (false, 300));
        let aborted = event("2026-11-07T10:01:00Z", "turn_aborted");
        assert!(codex_turn(&lines(&[started, call, aborted])).is_none());
        let meta = r#"{"type":"session_meta","payload":{"cwd":"/home/me/api/"}}"#;
        assert_eq!(codex_project(meta).as_deref(), Some("api"));
    }
}
