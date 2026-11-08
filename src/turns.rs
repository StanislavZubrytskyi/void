//! Finished agent turns and what they predict about the next ones.
//!
//! Each turn is one local JSON line in `~/.void/turns.jsonl`: agent, project folder, start, end
//! and the prompt text (unless `VOID_NO_PROMPTS` is set). Quota cost is not stored: it is read
//! from quota measurements around the turn, so it improves as measurements do.

use crate::usage;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs::File,
    io::{self, BufRead, BufReader},
    path::PathBuf,
};

/// Turns older than this no longer describe how the user works.
const KEEP: u64 = 30 * 86_400;
/// How far a quota measurement may sit from a turn edge and still describe it.

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    /// 0 is Claude, 1 is Codex, 2 is Kimi Code.
    pub agent: usize,
    pub project: String,
    pub start: u64,
    pub end: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

fn path() -> io::Result<PathBuf> {
    Ok(usage::data_dir()?.join("turns.jsonl"))
}

pub fn save(turn: &Turn) -> Result<(), Box<dyn Error>> {
    let mut turn = turn.clone();
    if std::env::var_os("VOID_NO_PROMPTS").is_some() {
        turn.prompt = None;
    }
    usage::append_line(&serde_json::to_vec(&turn)?, &path()?)
}

pub fn load() -> Vec<Turn> {
    let Ok(file) = path().and_then(File::open) else {
        return Vec::new();
    };
    if file.lock_shared().is_err() {
        return Vec::new();
    }
    let cutoff = usage::now().saturating_sub(KEEP);
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Turn>(&line).ok())
        .filter(|turn| {
            turn.agent < crate::launch::AGENTS && turn.end >= turn.start && turn.start >= cutoff
        })
        .collect()
}
