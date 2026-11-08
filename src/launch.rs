//! Starting a task in the desktop app the user works in: a new Claude Code session in Claude
//! Desktop, or a new thread in the Codex app, in the chosen project, with the text filled in.
//! Void only opens the app's own link (`claude://code/new`, `codex://threads/new`); the app
//! takes it from there, so its usual review and approvals apply.

use std::path::Path;

/// The apps trim long prompts themselves; keep the link well inside command-line limits.
const MAX_PROMPT: usize = 4000;

#[derive(Debug, PartialEq)]
pub enum Failure {
    Empty,
    TooLong,
    NoProject,
    NoApp,
    NoSession,
}

/// The desktop app each agent runs in.
pub const APPS: [&str; 2] = ["Claude", "Codex"];

impl Failure {
    pub fn label(&self, agent: usize) -> String {
        use crate::i18n::s;
        let app = APPS[agent.min(1)];
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
            Self::NoSession => s(
                "Сесію не знайдено",
                "Сессия не найдена",
                "Session not found",
            )
            .into(),
            Self::NoApp => t!(
                "Не вдалося відкрити {app}",
                "Не удалось открыть {app}",
                "Couldn't open {app}"
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

/// Percent-encodes everything but the unreserved URL characters, as UTF-8.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The app link that opens a new task for agent 0 (Claude) or 1 (Codex).
fn link(agent: usize, project: &Path, prompt: &str) -> String {
    let (text, folder) = (encode(prompt), encode(&project.to_string_lossy()));
    if agent == 0 {
        format!("claude://code/new?q={text}&folder={folder}")
    } else {
        format!("codex://threads/new?prompt={text}&path={folder}")
    }
}

/// The app link that opens an existing Claude session or Codex thread. None for an id that
/// is not the shape the app itself uses.
fn session_link(agent: usize, id: &str) -> Option<String> {
    let body = if agent == 0 {
        id.strip_prefix("local_")?
    } else {
        id
    };
    if body.is_empty() || !body.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some(if agent == 0 {
        format!("claude://code/continue?session={id}")
    } else {
        format!("codex://threads/{id}")
    })
}

/// Opens an existing conversation in its app. The apps' links take no text for it.
pub fn open_session(agent: usize, id: &str) -> Result<(), Failure> {
    open(&session_link(agent, id).ok_or(Failure::NoSession)?)
}

/// Opens a new task for agent 0 (Claude) or 1 (Codex) in its desktop app.
pub fn start(agent: usize, project: &Path, prompt: &str) -> Result<(), Failure> {
    let prompt = check(project, prompt)?;
    open(&link(agent, project, &prompt))
}

/// Hands the link to the system, which passes it to the app registered for it. No shell is
/// involved, so nothing in the prompt can be read as a command.
#[cfg(windows)]
fn open(url: &str) -> Result<(), Failure> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, target) = (wide("open"), wide(url));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute reports success with a value above 32.
    if result as isize > 32 {
        Ok(())
    } else {
        Err(Failure::NoApp)
    }
}

#[cfg(not(windows))]
fn open(url: &str) -> Result<(), Failure> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|_| Failure::NoApp)
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

    #[test]
    fn session_links_accept_only_the_apps_own_ids() {
        assert_eq!(
            session_link(0, "local_0a81-e30a").as_deref(),
            Some("claude://code/continue?session=local_0a81-e30a")
        );
        assert_eq!(
            session_link(1, "01a1b232-f6cb").as_deref(),
            Some("codex://threads/01a1b232-f6cb")
        );
        assert_eq!(session_link(0, "01a1b232"), None);
        assert_eq!(session_link(1, "x&y=1"), None);
    }

    #[test]
    fn links_carry_the_text_and_folder_safely() {
        assert_eq!(encode("a b/ä&q=1"), "a%20b%2F%C3%A4%26q%3D1");
        let folder = Path::new(r"D:\work\site");
        assert_eq!(
            link(0, folder, "fix it"),
            "claude://code/new?q=fix%20it&folder=D%3A%5Cwork%5Csite"
        );
        assert_eq!(
            link(1, folder, "fix it"),
            "codex://threads/new?prompt=fix%20it&path=D%3A%5Cwork%5Csite"
        );
    }
}
