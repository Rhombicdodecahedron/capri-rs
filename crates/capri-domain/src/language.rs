//! Interface language. Her replies follow the language the user writes in; this sets the labels
//! around her (chat box, settings) and the hint given to the model for her first words.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// Follow the system locale.
    #[default]
    Auto,
    En,
    Fr,
}

impl Language {
    /// `Auto` resolved against a locale tag such as `fr-CH` or `en_US.UTF-8`.
    #[must_use]
    pub fn resolve(self, system_locale: Option<&str>) -> Language {
        match self {
            Language::Auto => match system_locale.map(str::to_lowercase) {
                Some(l) if l.starts_with("fr") => Language::Fr,
                _ => Language::En,
            },
            other => other,
        }
    }

    /// Picks the right string. `Auto` must be resolved first; it falls back to English.
    pub fn pick<'a>(self, en: &'a str, fr: &'a str) -> &'a str {
        match self {
            Language::Fr => fr,
            Language::En | Language::Auto => en,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_follows_locale() {
        assert_eq!(Language::Auto.resolve(Some("fr-CH")), Language::Fr);
        assert_eq!(Language::Auto.resolve(Some("de-CH")), Language::En);
        assert_eq!(Language::Auto.resolve(None), Language::En);
        assert_eq!(Language::Fr.resolve(Some("en-US")), Language::Fr);
    }
}
