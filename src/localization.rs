//! Text lookups from a `global.ini` localization file (`key=text` per line).
//!
//! Profile attributes such as `UILabel="@ui_CIEject"` refer to these texts; the leading
//! `@` only marks the value as a reference.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

#[derive(Default)]
pub struct Strings {
    /// Text by lowercased key; lookups ignore case like the game does.
    texts: HashMap<String, String>,
}

impl Strings {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Self::parse(&String::from_utf8_lossy(&bytes)))
    }

    /// Parses `key=text` lines. A `,P` style suffix on the key is dropped; lines without
    /// `=` are ignored. A literal `\n` in the text becomes a space, since the texts are
    /// shown on one line.
    pub fn parse(content: &str) -> Self {
        let texts = content
            .trim_start_matches('\u{feff}')
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, text)| {
                let key = key.split(',').next().unwrap_or(key).trim().to_lowercase();
                (key, text.replace("\\n", " ").trim().to_string())
            })
            .collect();
        Self { texts }
    }

    pub fn len(&self) -> usize {
        self.texts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.texts.is_empty()
    }

    /// The text for a `@key` reference, empty for a reference without a key (`"@ "`), or
    /// `value` unchanged when it is not a reference or the key is unknown.
    pub fn resolve<'a>(&'a self, value: &'a str) -> &'a str {
        let Some(key) = value.strip_prefix('@').map(str::trim) else {
            return value;
        };
        if key.is_empty() {
            return "";
        }
        self.texts.get(&key.to_lowercase()).map_or(value, String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_resolve_ignoring_case_and_key_suffix() {
        let strings = Strings::parse("\u{feff}ui_CIEject=Eject\r\nui_Desc,P=Line one\\nline two\r\nbroken\n");
        assert_eq!(strings.len(), 2);
        assert_eq!(strings.resolve("@ui_cieject"), "Eject");
        assert_eq!(strings.resolve("@ui_Desc"), "Line one line two");
        assert_eq!(strings.resolve("@ui_missing"), "@ui_missing");
        assert_eq!(strings.resolve("ui_CIEject"), "ui_CIEject");
        assert_eq!(strings.resolve("@ "), "");
    }
}
