//! Interface language: Ukrainian, Russian or English, chosen in the panel's settings.
//!
//! The language lives on the UI thread, which draws every word Void shows (panel, tray menu,
//! notifications). Tests run on their own threads and so always start in Ukrainian.

use serde::{Deserialize, Serialize};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Uk,
    Ru,
    En,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Uk, Lang::Ru, Lang::En];

    /// The short label on the language switch.
    pub fn label(self) -> &'static str {
        match self {
            Lang::Uk => "УКР",
            Lang::Ru => "РУС",
            Lang::En => "ENG",
        }
    }
}

thread_local! {
    static LANG: Cell<Lang> = const { Cell::new(Lang::Uk) };
}

pub fn set(lang: Lang) {
    LANG.with(|cell| cell.set(lang));
}

pub fn lang() -> Lang {
    LANG.with(Cell::get)
}

/// One fixed phrase in the current language.
pub fn s(uk: &'static str, ru: &'static str, en: &'static str) -> &'static str {
    match lang() {
        Lang::Uk => uk,
        Lang::Ru => ru,
        Lang::En => en,
    }
}

/// `format!` in the current language: `t!("uk {x}", "ru {x}", "en {x}", args...)`.
macro_rules! t {
    ($uk:literal, $ru:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::Uk => format!($uk $(, $arg)*),
            $crate::i18n::Lang::Ru => format!($ru $(, $arg)*),
            $crate::i18n::Lang::En => format!($en $(, $arg)*),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrases_follow_the_chosen_language() {
        let name = "Codex";
        assert_eq!(
            t!("{name} працює", "{name} работает", "{name} is working"),
            "Codex працює"
        );
        set(Lang::En);
        assert_eq!(
            t!("{name} працює", "{name} работает", "{name} is working"),
            "Codex is working"
        );
        assert_eq!(s("Звук", "Звук", "Sound"), "Sound");
        set(Lang::Uk);
    }
}
