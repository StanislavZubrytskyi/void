//! The projects each desktop app shows in its own sidebar, in the same order and under the same
//! names: Claude Desktop's environments (`ccd-session-presets.json`) and the Codex app's local
//! projects (`.codex-global-state.json`). Read only; Void never changes them.

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub path: PathBuf,
    /// The name the app shows for it.
    pub name: String,
    /// The app's current project: chosen by default.
    pub preferred: bool,
    /// The model the app starts new sessions with here, when it says ("Opus 5.5").
    pub model: Option<String>,
}

/// Projects of agent 0 (Claude Desktop) or 1 (Codex), as the app lists them.
pub fn list(agent: usize) -> Vec<Project> {
    if agent == 0 { claude() } else { codex() }
}

/// "claude-opus-5-5" reads as "Opus 5.5".
fn model_name(id: &str) -> String {
    let mut parts = id.trim_start_matches("claude-").split('-');
    let family = parts.next().unwrap_or_default();
    let mut name: String = family
        .chars()
        .enumerate()
        .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
        .collect();
    let version: Vec<&str> = parts.collect();
    if !version.is_empty() {
        name.push(' ');
        name.push_str(&version.join("."));
    }
    name
}

/// The repository name from a folder's git remote, which is how Claude Desktop labels a
/// project that is a git checkout ("heavy_burger" for `HeavyBurger`).
fn repository_name(folder: &Path) -> Option<String> {
    let config = std::fs::read_to_string(folder.join(".git").join("config")).ok()?;
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            in_origin = line == "[remote \"origin\"]";
        } else if in_origin && let Some(url) = line.strip_prefix("url") {
            let url = url
                .trim_start_matches([' ', '='])
                .trim()
                .trim_end_matches('/');
            let last = url.rsplit(['/', ':']).next()?;
            let name = last.strip_suffix(".git").unwrap_or(last);
            return (!name.is_empty()).then(|| name.to_owned());
        }
    }
    None
}

/// Claude Desktop keeps its data under `Claude-Data\<package>` (Store install) or `Claude`.
fn claude_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        for entry in std::fs::read_dir(local.join("Claude-Data"))
            .into_iter()
            .flatten()
            .flatten()
        {
            dirs.push(entry.path());
        }
    }
    if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
        dirs.push(roaming.join("Claude"));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join("Library/Application Support/Claude"));
    }
    dirs
}

fn claude() -> Vec<Project> {
    // Every signed-in account keeps its own environments; the most recently used one wins.
    let mut best: Option<(u64, Vec<Project>)> = None;
    for dir in claude_data_dirs() {
        let Some(state) = std::fs::read(dir.join("ccd-session-presets.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        else {
            continue;
        };
        for account in state
            .get("byAccount")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|accounts| accounts.values())
        {
            let (used, projects) = claude_presets(account);
            if best.as_ref().is_none_or(|(latest, _)| used > *latest) {
                best = Some((used, projects));
            }
        }
    }
    best.map(|(_, projects)| projects).unwrap_or_default()
}

/// One account's environments, sorted by name as the sidebar shows them; the last used one
/// is preferred. Returns the latest use too.
fn claude_presets(account: &Value) -> (u64, Vec<Project>) {
    let mut latest = (0, None::<PathBuf>);
    let mut projects: Vec<Project> = Vec::new();
    for preset in account
        .get("presets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if preset.pointer("/target/kind").and_then(Value::as_str) != Some("local") {
            continue;
        }
        let Some(cwd) = preset.pointer("/target/cwd").and_then(Value::as_str) else {
            continue;
        };
        let path = PathBuf::from(cwd);
        if !path.is_dir() || projects.iter().any(|project| project.path == path) {
            continue;
        }
        let used = preset
            .get("lastUsedAt")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if used >= latest.0 {
            latest = (used, Some(path.clone()));
        }
        let name = repository_name(&path)
            .or_else(|| {
                preset
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| crate::activity::folder_name(cwd));
        projects.push(Project {
            name,
            model: preset
                .pointer("/settings/modelId")
                .and_then(Value::as_str)
                .map(model_name),
            preferred: false,
            path,
        });
    }
    projects.sort_by_key(|project| project.name.to_lowercase());
    for project in &mut projects {
        project.preferred = latest.1.as_ref() == Some(&project.path);
    }
    (latest.0, projects)
}

fn codex() -> Vec<Project> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|home| PathBuf::from(home).join(".codex"))
        });
    home.and_then(|home| std::fs::read(home.join(".codex-global-state.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|state| codex_projects(&state))
        .unwrap_or_default()
}

/// The Codex sidebar: projects in `project-order`, then the rest by creation; the selected
/// project is preferred.
fn codex_projects(state: &Value) -> Vec<Project> {
    let Some(projects) = state.get("local-projects").and_then(Value::as_object) else {
        return Vec::new();
    };
    let order: Vec<&str> = state
        .get("project-order")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let selected = state
        .pointer("/selected-project/projectId")
        .and_then(Value::as_str);
    let mut listed: Vec<(usize, u64, Project)> = projects
        .iter()
        .filter_map(|(id, project)| {
            let path = PathBuf::from(project.pointer("/rootPaths/0")?.as_str()?);
            if !path.is_dir() {
                return None;
            }
            let rank = order
                .iter()
                .position(|known| known == id)
                .unwrap_or(usize::MAX);
            let created = project
                .get("createdAt")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let name = project.get("name").and_then(Value::as_str).map_or_else(
                || crate::activity::folder_name(&path.to_string_lossy()),
                str::to_owned,
            );
            Some((
                rank,
                created,
                Project {
                    name,
                    preferred: selected == Some(id.as_str()),
                    model: None,
                    path,
                },
            ))
        })
        .collect();
    listed.sort_by_key(|(rank, created, _)| (*rank, *created));
    listed.into_iter().map(|(_, _, project)| project).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_read_like_the_app() {
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("claude-haiku-5-5"), "Haiku 5.5");
    }

    #[test]
    fn claude_environments_sort_by_name_and_prefer_the_last_used() {
        let here = std::env::temp_dir();
        let path = here.to_string_lossy().to_string();
        let account = serde_json::json!({ "presets": [
            { "name": "zeta", "lastUsedAt": 5, "target": { "kind": "local", "cwd": path },
              "settings": { "modelId": "claude-sonnet-5-5" } },
            { "name": "remote", "lastUsedAt": 9, "target": { "kind": "ssh", "cwd": path } },
        ]});
        let (used, projects) = claude_presets(&account);
        assert_eq!(used, 5);
        assert_eq!(projects.len(), 1);
        assert!(projects[0].preferred);
        assert_eq!(projects[0].model.as_deref(), Some("Sonnet 5.5"));
    }

    #[test]
    fn codex_projects_follow_the_sidebar_order() {
        let here = std::env::temp_dir().to_string_lossy().to_string();
        let other = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let state = serde_json::json!({
            "local-projects": {
                "a": { "name": "Alpha", "rootPaths": [here], "createdAt": 1 },
                "b": { "name": "Beta", "rootPaths": [other], "createdAt": 2 },
                "gone": { "name": "Gone", "rootPaths": ["Z:\\no\\such\\void\\folder"], "createdAt": 3 }
            },
            "project-order": ["b"],
            "selected-project": { "projectId": "a" }
        });
        let projects = codex_projects(&state);
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Beta", "Alpha"]);
        assert!(projects[1].preferred);
    }

    #[test]
    fn repository_names_come_from_the_origin_remote() {
        let dir = std::env::temp_dir().join(format!("void-repo-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join(".git").join("config"),
            "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = https://github.com/me/heavy_burger.git\n",
        )
        .unwrap();
        assert_eq!(repository_name(&dir).as_deref(), Some("heavy_burger"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
