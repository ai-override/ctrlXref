//! Free-form regular expression search over the table and the tree, and the saved list of
//! previous searches.

use anyhow::Result;
use regex::{Regex, RegexBuilder};

/// How many previous searches are kept.
pub const MAX_HISTORY: usize = 25;

/// `pattern` as a regular expression, ignoring case unless it says otherwise with `(?-i)`.
pub fn compile(pattern: &str) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern).case_insensitive(true).build()
}

/// How many times `regex` matches in `text`. Empty matches, as `x*` has everywhere, don't count.
pub fn occurrences(regex: &Regex, text: &str) -> usize {
    regex.find_iter(text).filter(|found| !found.is_empty()).count()
}

/// Something a search can stop at, in the order searches go through them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// Its tree id.
    pub id: String,
    /// Its displayed table row; actionmaps are only in the tree.
    pub row: Option<usize>,
    /// How many matches it has.
    pub occurrences: usize,
}

/// Where a search stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    /// Index of the place.
    pub place: usize,
    /// Which of the places with matches it is, from 1.
    pub rank: usize,
    /// Whether the search went past the end or the start to get there.
    pub wrapped: bool,
}

/// The next place with matches after the one with id `current`, or the previous one when not
/// `forward`, going around the ends of `places`. It is `current` itself when only that has
/// matches. Without a current place, it starts from the first or the last place.
pub fn find(places: &[Place], current: Option<&str>, forward: bool) -> Option<Found> {
    let count = places.len();
    let current = current.and_then(|id| places.iter().position(|place| place.id == id));
    (0..count).find_map(|step| {
        let (place, wrapped) = if forward {
            let start = current.map_or(0, |ix| ix + 1);
            ((start + step) % count, start + step >= count)
        } else {
            let start = current.map_or(count - 1, |ix| ix + count - 1);
            ((start + count - step) % count, current.is_some_and(|ix| step >= ix))
        };
        (places[place].occurrences > 0).then(|| Found {
            place,
            rank: places[..=place].iter().filter(|place| place.occurrences > 0).count(),
            wrapped,
        })
    })
}

/// Moves `pattern` to the front of `history`, which keeps the latest [`MAX_HISTORY`] searches.
pub fn remember(history: &mut Vec<String>, pattern: &str) {
    history.retain(|old| old != pattern);
    history.insert(0, pattern.to_string());
    history.truncate(MAX_HISTORY);
}

#[cfg(windows)]
const REG_KEY: &str = r"Software\ctrlXref";
#[cfg(windows)]
const REG_VALUE: &str = "SearchHistory";

/// The previous searches saved by [`save_history`], latest first.
#[cfg(windows)]
pub fn load_history() -> Vec<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(REG_KEY)
        .and_then(|key| key.get_value(REG_VALUE))
        .unwrap_or_default()
}

/// Saves `history` under `HKEY_CURRENT_USER\Software\ctrlXref` for [`load_history`].
#[cfg(windows)]
pub fn save_history(history: &[String]) -> Result<()> {
    use anyhow::Context;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(REG_KEY)
        .with_context(|| format!(r"cannot open registry key HKCU\{REG_KEY}"))?;
    key.set_value(REG_VALUE, &history.to_vec())
        .with_context(|| format!(r"cannot write HKCU\{REG_KEY}\{REG_VALUE}"))
}

#[cfg(not(windows))]
pub fn load_history() -> Vec<String> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn save_history(_history: &[String]) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Places `a`, `b`, ... with the given numbers of matches.
    fn places(occurrences: &[usize]) -> Vec<Place> {
        occurrences
            .iter()
            .enumerate()
            .map(|(ix, occurrences)| Place {
                id: ((b'a' + ix as u8) as char).to_string(),
                row: Some(ix),
                occurrences: *occurrences,
            })
            .collect()
    }

    fn stop(places: &[Place], current: Option<&str>, forward: bool) -> Option<(usize, usize, bool)> {
        find(places, current, forward).map(|found| (found.place, found.rank, found.wrapped))
    }

    #[test]
    fn searches_ignore_case_unless_told_otherwise() {
        assert_eq!(occurrences(&compile("eject").unwrap(), "Eject, EJECT"), 2);
        assert_eq!(occurrences(&compile("(?-i)eject").unwrap(), "Eject, eject"), 1);
        assert!(compile("(unclosed").is_err());
    }

    #[test]
    fn empty_matches_do_not_count() {
        assert_eq!(occurrences(&compile("x*").unwrap(), "axxb"), 1);
        assert_eq!(occurrences(&compile("^$").unwrap(), ""), 0);
    }

    #[test]
    fn find_next_goes_around_the_end() {
        let places = places(&[0, 2, 0, 1, 0]);
        assert_eq!(stop(&places, None, true), Some((1, 1, false)));
        assert_eq!(stop(&places, Some("b"), true), Some((3, 2, false)));
        assert_eq!(stop(&places, Some("c"), true), Some((3, 2, false)));
        assert_eq!(stop(&places, Some("d"), true), Some((1, 1, true)));
        assert_eq!(stop(&places, Some("e"), true), Some((1, 1, true)));
    }

    #[test]
    fn find_previous_goes_around_the_start() {
        let places = places(&[0, 2, 0, 1, 0]);
        assert_eq!(stop(&places, None, false), Some((3, 2, false)));
        assert_eq!(stop(&places, Some("d"), false), Some((1, 1, false)));
        assert_eq!(stop(&places, Some("c"), false), Some((1, 1, false)));
        assert_eq!(stop(&places, Some("b"), false), Some((3, 2, true)));
        assert_eq!(stop(&places, Some("a"), false), Some((3, 2, true)));
    }

    #[test]
    fn a_single_match_is_found_again_as_a_wrap() {
        let places = places(&[0, 1, 0]);
        assert_eq!(stop(&places, Some("b"), true), Some((1, 1, true)));
        assert_eq!(stop(&places, Some("b"), false), Some((1, 1, true)));
    }

    #[test]
    fn nothing_to_find() {
        assert_eq!(stop(&places(&[0, 0]), Some("a"), true), None);
        assert_eq!(stop(&[], None, true), None);
        assert_eq!(stop(&[], None, false), None);
    }

    #[test]
    fn history_keeps_the_latest_once() {
        let mut history = vec!["a".to_string(), "b".to_string()];
        remember(&mut history, "b");
        assert_eq!(history, ["b", "a"]);
        remember(&mut history, "c");
        assert_eq!(history, ["c", "b", "a"]);
        for ix in 0..MAX_HISTORY {
            remember(&mut history, &ix.to_string());
        }
        assert_eq!(history.len(), MAX_HISTORY);
        assert_eq!(history[0], (MAX_HISTORY - 1).to_string());
    }
}
