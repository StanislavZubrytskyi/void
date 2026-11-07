//! Starting a task: opens the agent's own interactive CLI in a new terminal window, in the
//! chosen project, with the prompt as its first message. The user sees the work and approves
//! the agent's actions there, exactly as if they had typed the command themselves.

use std::path::{Path, PathBuf};

/// Long enough for a real task description, short enough to stay a command-line argument.
const MAX_PROMPT: usize = 8000;

#[derive(Debug, PartialEq)]
pub enum Failure {
    Empty,
    TooLong,
    NoProject,
    NoAgent,
    Start,
}

impl Failure {
    pub fn label(&self, agent: usize) -> String {
        use crate::i18n::s;
        let name = ["Claude", "Codex"][agent];
        match self {
            Self::Empty => s(
                "Напиши, що зробити",
                "Напиши, что сделать",
                "Write what to do",
            )
            .into(),
            Self::TooLong => s(
                "Задача задовга",
                "Задача слишком длинная",
                "The task is too long",
            )
            .into(),
            Self::NoProject => s(
                "Папку проєкту не знайдено",
                "Папка проекта не найдена",
                "Project folder not found",
            )
            .into(),
            Self::NoAgent => t!(
                "{name} не встановлено",
                "{name} не установлен",
                "{name} is not installed"
            ),
            Self::Start => t!(
                "Не вдалося запустити {name}",
                "Не удалось запустить {name}",
                "Couldn't start {name}"
            ),
        }
    }
}

/// The checks that do not touch the system: a prompt worth sending and a folder that exists.
fn check(project: &Path, prompt: &str) -> Result<String, Failure> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err(Failure::Empty);
    }
    if prompt.chars().count() > MAX_PROMPT {
        return Err(Failure::TooLong);
    }
    if !project.is_dir() {
        return Err(Failure::NoProject);
    }
    Ok(prompt.to_owned())
}

/// Claude Code's executable: an explicit override, then PATH, then the native and npm installs.
fn claude_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("VOID_CLAUDE_BIN").map(PathBuf::from)
        && path.is_absolute()
        && path.is_file()
    {
        return Some(path);
    }
    let name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let on_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .filter(|entry| entry.is_absolute())
            .map(|entry| entry.join(name))
            .find(|path| path.is_file())
    });
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let native = home.map(|home| home.join(".local").join("bin").join(name));
    let npm = std::env::var_os("APPDATA").map(|appdata| {
        PathBuf::from(appdata)
            .join("npm/node_modules/@anthropic-ai/claude-code/bin")
            .join(name)
    });
    on_path
        .or(native.filter(|path| path.is_file()))
        .or(npm.filter(|path| path.is_file()))
}

/// Opens agent 0 (Claude) or 1 (Codex) in a new terminal in `project`, starting with `prompt`.
pub fn start(agent: usize, project: &Path, prompt: &str) -> Result<(), Failure> {
    let prompt = check(project, prompt)?;
    let binary = if agent == 0 {
        claude_binary()
    } else {
        crate::codex::codex_binary()
    }
    .ok_or(Failure::NoAgent)?;
    let mut command = std::process::Command::new(binary);
    // The prompt is one argument handed straight to the executable, never through a shell.
    command.arg(prompt).current_dir(project);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE: the agent gets its own window
    }
    // shortcut: outside Windows the agent starts without a terminal of its own; give it one
    // (a terminal emulator command) when the macOS and Linux builds ship.
    command.spawn().map(|_| ()).map_err(|_| Failure::Start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_needs_words_and_a_real_folder() {
        let here = std::env::temp_dir();
        assert_eq!(check(&here, "   "), Err(Failure::Empty));
        assert_eq!(
            check(&here, &"x".repeat(MAX_PROMPT + 1)),
            Err(Failure::TooLong)
        );
        assert_eq!(
            check(&here.join("no-such-void-folder"), "fix it"),
            Err(Failure::NoProject)
        );
        assert_eq!(
            check(&here, "  fix the login \n"),
            Ok("fix the login".to_owned())
        );
    }
}
