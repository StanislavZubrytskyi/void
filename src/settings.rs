//! What the user chose in the panel's settings, kept in `~/.void/settings.json`.

use crate::i18n::Lang;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Lang,
    /// A short chime when an agent finishes a turn.
    pub sound: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Lang::Uk,
            sound: true,
        }
    }
}

fn path() -> io::Result<PathBuf> {
    Ok(crate::usage::data_dir()?.join("settings.json"))
}

/// Missing or unreadable settings fall back to the defaults.
pub fn load() -> Settings {
    path()
        .and_then(fs::read)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> io::Result<()> {
    let path = path()?;
    fs::create_dir_all(path.parent().unwrap())?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(settings)?)?;
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_or_partial_files_keep_defaults() {
        let partial: Settings = serde_json::from_str(r#"{"language":"en"}"#).unwrap();
        assert_eq!(
            partial,
            Settings {
                language: Lang::En,
                sound: true
            }
        );
        assert!(serde_json::from_str::<Settings>(r#"{"language":"de"}"#).is_err());
    }
}
