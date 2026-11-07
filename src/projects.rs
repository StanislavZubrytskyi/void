//! Folders each agent recently worked in, newest first: the choices for starting a new task.
//! Read from the agents' own local transcripts; nothing is stored.

use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

/// Older work is unlikely to be what the user wants to continue.
const RECENT: Duration = Duration::from_secs(30 * 86_400);
const LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub path: PathBuf,
    /// The folder name, for the picker.
    pub name: String,
    /// When the agent last wrote a transcript there.
    pub last: SystemTime,
}

/// Recent projects for agent 0 (Claude) or 1 (Codex), newest first.
pub fn recent(agent: usize) -> Vec<Project> {
    let mut found: Vec<Project> = Vec::new();
    let mut add = |cwd: &str, last: SystemTime| {
        let path = PathBuf::from(cwd);
        if !path.is_dir() || is_scratch(&path) {
            return;
        }
        match found.iter_mut().find(|project| project.path == path) {
            Some(project) => project.last = project.last.max(last),
            None => found.push(Project {
                name: crate::activity::folder_name(cwd),
                path,
                last,
            }),
        }
    };
    if agent == 0 {
        // One folder per project; its newest transcript knows the working directory.
        let projects = crate::claude::config_dir().map(|dir| dir.join("projects"));
        for folder in projects
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .into_iter()
            .flatten()
            .flatten()
        {
            if let Some((last, path)) = newest_jsonl(&folder.path())
                && let Some(cwd) = claude_cwd(&path)
            {
                add(&cwd, last);
            }
        }
    } else {
        let home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .map(|home| PathBuf::from(home).join(".codex"))
            });
        let mut sessions = Vec::new();
        if let Some(home) = home {
            collect_jsonl(&home.join("sessions"), 3, &mut sessions);
        }
        for (last, path) in sessions {
            if let Some(cwd) = crate::activity::first_line(&path)
                .and_then(|line| serde_json::from_str::<Value>(&line).ok())
                .and_then(|entry| Some(entry.pointer("/payload/cwd")?.as_str()?.to_owned()))
            {
                add(&cwd, last);
            }
        }
    }
    found.sort_by(|a, b| b.last.cmp(&a.last));
    found.truncate(LIMIT);
    found
}

/// Claude Desktop's throwaway scratch workspaces are not projects to come back to.
fn is_scratch(path: &Path) -> bool {
    let text = path.to_string_lossy().to_ascii_lowercase();
    text.contains("claude-data") || text.contains("\\scratch\\") || text.contains("/scratch/")
}

fn recent_enough(path: &Path) -> Option<SystemTime> {
    let modified = path.metadata().ok()?.modified().ok()?;
    (SystemTime::now().duration_since(modified).ok()? < RECENT).then_some(modified)
}

fn newest_jsonl(dir: &Path) -> Option<(SystemTime, PathBuf)> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|path| Some((recent_enough(&path)?, path)))
        .max_by_key(|(last, _)| *last)
}

fn collect_jsonl(dir: &Path, depth: u8, found: &mut Vec<(SystemTime, PathBuf)>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() && depth > 0 {
            collect_jsonl(&path, depth - 1, found);
        } else if path.extension().is_some_and(|ext| ext == "jsonl")
            && let Some(last) = recent_enough(&path)
        {
            found.push((last, path));
        }
    }
}

/// The working directory a Claude Code transcript was recorded in: the first `cwd` field.
fn claude_cwd(path: &Path) -> Option<String> {
    BufReader::new(File::open(path).ok()?)
        .lines()
        .take(200)
        .map_while(Result::ok)
        .filter(|line| line.contains("\"cwd\""))
        .find_map(|line| {
            let entry: Value = serde_json::from_str(&line).ok()?;
            Some(entry.get("cwd")?.as_str()?.to_owned())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_workspaces_are_not_projects() {
        assert!(is_scratch(Path::new(
            r"C:\Users\me\AppData\Local\Claude-Data\claude_x\scratch\2026-10-23"
        )));
        assert!(!is_scratch(Path::new(r"D:\work\site")));
    }

    #[test]
    fn reads_the_working_directory_from_a_transcript() {
        let dir = std::env::temp_dir().join(format!("void-projects-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.jsonl");
        std::fs::write(
            &file,
            "{\"type\":\"summary\"}\n{\"type\":\"user\",\"cwd\":\"D:\\\\work\\\\site\"}\n",
        )
        .unwrap();
        assert_eq!(claude_cwd(&file).as_deref(), Some(r"D:\work\site"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
