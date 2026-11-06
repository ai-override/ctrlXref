//! Keyboard combinations of one or two modifiers and one other key, and which of them no
//! binding uses yet.
//!
//! Keys use the game's names, as in a profile's `keyboard="lalt+f"` or a rebind file's
//! `input="kb1_lalt+f"`.

use std::collections::BTreeSet;
use std::fmt;

use xmltree::{Element, XMLNode};

/// The keys that start a combination, in the order combinations name them. The Windows
/// keys and Caps Lock are left out on purpose.
pub const MODIFIERS: [&str; 6] = ["lalt", "ralt", "lctrl", "rctrl", "lshift", "rshift"];

/// The keys a modifier combines with, in keyboard order.
pub const KEYS: &[&str] = &[
    "escape", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
    "print", "scrolllock", "pause",
    "grave", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "minus", "equals", "backspace",
    "tab", "q", "w", "e", "r", "t", "y", "u", "i", "o", "p", "lbracket", "rbracket", "backslash",
    "a", "s", "d", "f", "g", "h", "j", "k", "l", "semicolon", "apostrophe", "enter",
    "z", "x", "c", "v", "b", "n", "m", "comma", "period", "slash",
    "space",
    "insert", "delete", "home", "end", "pgup", "pgdn",
    "up", "down", "left", "right",
    "numlock", "np_divide", "np_multiply", "np_subtract", "np_add", "np_enter", "np_period",
    "np_0", "np_1", "np_2", "np_3", "np_4", "np_5", "np_6", "np_7", "np_8", "np_9",
];

/// Two-key combinations Windows handles itself, which the game never gets or which do
/// something drastic: closing the window, switching windows, opening the Start menu or the
/// window menu, and taking a screenshot of the window. A three-key combination containing
/// one of these is reserved too (Alt+Shift+Tab, Ctrl+Alt+Tab, ...).
pub const RESERVED: [(&str, &str); 12] = [
    ("lalt", "f4"),
    ("ralt", "f4"),
    ("lalt", "tab"),
    ("ralt", "tab"),
    ("lalt", "escape"),
    ("ralt", "escape"),
    ("lalt", "space"),
    ("ralt", "space"),
    ("lalt", "print"),
    ("ralt", "print"),
    ("lctrl", "escape"),
    ("rctrl", "escape"),
];

/// One or two [`MODIFIERS`] and one of the [`KEYS`]. The modifiers are in [`MODIFIERS`]
/// order, so each combination has one value however its input was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Combo {
    pub first: &'static str,
    pub second: Option<&'static str>,
    pub key: &'static str,
}

impl Combo {
    pub fn two(modifier: &'static str, key: &'static str) -> Self {
        Self { first: modifier, second: None, key }
    }

    pub fn three(first: &'static str, second: &'static str, key: &'static str) -> Self {
        Self { first, second: Some(second), key }
    }

    /// The modifiers, like `lalt` or `lalt+lctrl`.
    pub fn modifiers(&self) -> String {
        match self.second {
            Some(second) => format!("{}+{second}", self.first),
            None => self.first.to_string(),
        }
    }

    fn has(&self, modifier: &str) -> bool {
        self.first == modifier || self.second == Some(modifier)
    }

    fn has_kind(&self, kind: &str) -> bool {
        self.first.ends_with(kind) || self.second.is_some_and(|second| second.ends_with(kind))
    }

    /// Whether Windows handles this combination itself; see [`RESERVED`].
    pub fn is_reserved(&self) -> bool {
        let contains_reserved = RESERVED
            .iter()
            .any(|&(modifier, key)| key == self.key && self.has(modifier));
        let task_manager = self.has_kind("ctrl") && self.has_kind("shift") && self.key == "escape";
        let secure_attention =
            self.has_kind("ctrl") && self.has_kind("alt") && matches!(self.key, "delete" | "np_period");
        contains_reserved || task_manager || secure_attention
    }
}

/// `lalt+f`, `lalt+lctrl+f`: modifiers first, as rebind inputs write them.
impl fmt::Display for Combo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}+{}", self.modifiers(), self.key)
    }
}

/// The modifier pairs of three-key combinations: two different kinds, so no `lalt+ralt`,
/// `lctrl+rctrl` or `lshift+rshift`, which are awkward to press (and `lctrl+ralt` is AltGr
/// on many layouts anyway, but that one has different kinds and stays).
pub fn modifier_pairs() -> Vec<(&'static str, &'static str)> {
    fn kind(modifier: &str) -> &str {
        &modifier[1..]
    }
    MODIFIERS
        .iter()
        .enumerate()
        .flat_map(|(ix, &first)| MODIFIERS[ix + 1..].iter().map(move |&second| (first, second)))
        .filter(|(first, second)| kind(first) != kind(second))
        .collect()
}

/// The combination `input` names, like `lalt+f`, `f5+LAlt`, `kb1_ralt+1` or
/// `kb1_lshift+lctrl+x`, in any case and order; `None` for anything else, such as a single
/// key, a mouse button or the same modifier twice.
pub fn parse_combo(input: &str) -> Option<Combo> {
    let input = input.trim().to_ascii_lowercase();
    let tokens: Vec<&str> = strip_device(&input).split('+').map(str::trim).collect();
    if !(2..=3).contains(&tokens.len()) {
        return None;
    }
    let mut modifiers: Vec<usize> = tokens
        .iter()
        .filter_map(|token| MODIFIERS.iter().position(|modifier| modifier == token))
        .collect();
    let keys: Vec<&str> = tokens
        .iter()
        .copied()
        .filter(|token| !MODIFIERS.contains(token))
        .collect();
    let [key] = keys[..] else {
        return None;
    };
    let key = find(KEYS, key)?;
    modifiers.sort_unstable();
    modifiers.dedup();
    match modifiers[..] {
        [first] if tokens.len() == 2 => Some(Combo::two(MODIFIERS[first], key)),
        [first, second] => Some(Combo::three(MODIFIERS[first], MODIFIERS[second], key)),
        _ => None,
    }
}

/// `input` without a rebind file's device prefix, like the `kb1_` of `kb1_lalt+f`.
fn strip_device(input: &str) -> &str {
    match input.split_once('_') {
        Some((device, rest)) if device.starts_with("kb") && device[2..].bytes().all(|b| b.is_ascii_digit()) => rest,
        _ => input,
    }
}

fn find(names: &[&'static str], name: &str) -> Option<&'static str> {
    names.iter().copied().find(|candidate| *candidate == name)
}

/// The keyboard inputs a profile binds: every `keyboard` attribute, and the `input` of
/// each `<keyboard>` element and its children.
pub fn profile_keyboard_inputs(root: &Element) -> Vec<String> {
    let mut inputs = Vec::new();
    walk(root, &mut |element| {
        if let Some(input) = element.attributes.get("keyboard") {
            inputs.push(input.clone());
        }
        if element.name == "keyboard" {
            inputs.extend(element.attributes.get("input").cloned());
            for child in child_elements(element) {
                inputs.extend(child.attributes.get("input").cloned());
            }
        }
    });
    inputs
}

/// The keyboard inputs (`kb...`) of every `<rebind>` element in a rebind file.
pub fn rebind_keyboard_inputs(root: &Element) -> Vec<String> {
    let mut inputs = Vec::new();
    walk(root, &mut |element| {
        if element.name == "rebind"
            && let Some(input) = element.attributes.get("input")
            && input.starts_with("kb")
        {
            inputs.push(input.clone());
        }
    });
    inputs
}

fn walk(element: &Element, visit: &mut impl FnMut(&Element)) {
    visit(element);
    for child in child_elements(element) {
        walk(child, visit);
    }
}

fn child_elements(element: &Element) -> impl Iterator<Item = &Element> {
    element.children.iter().filter_map(|node| match node {
        XMLNode::Element(child) => Some(child),
        _ => None,
    })
}

/// The combinations among `inputs`.
pub fn bound_combos<'a>(inputs: impl IntoIterator<Item = &'a str>) -> BTreeSet<Combo> {
    inputs.into_iter().filter_map(parse_combo).collect()
}

/// Every combination that is not reserved, best first: the two-key ones by [`MODIFIERS`]
/// and then [`KEYS`] order, then the three-key ones by [`modifier_pairs`] and key.
pub fn candidates() -> Vec<Combo> {
    let two = MODIFIERS
        .iter()
        .flat_map(|&modifier| KEYS.iter().map(move |&key| Combo::two(modifier, key)));
    let three = modifier_pairs()
        .into_iter()
        .flat_map(|(first, second)| KEYS.iter().map(move |&key| Combo::three(first, second, key)));
    two.chain(three).filter(|combo| !combo.is_reserved()).collect()
}

/// The [`candidates`] that are not `bound`, in the same order.
pub fn unused_combos(bound: &BTreeSet<Combo>) -> Vec<Combo> {
    candidates().into_iter().filter(|combo| !bound.contains(combo)).collect()
}

/// `combos` as text, one line per modifier or modifier pair: `lalt (3): a, b, c`.
pub fn format_by_modifier(combos: &[Combo]) -> String {
    let mut lines: Vec<(String, Vec<&str>)> = Vec::new();
    for combo in combos {
        let modifiers = combo.modifiers();
        match lines.last_mut() {
            Some((last, keys)) if *last == modifiers => keys.push(combo.key),
            _ => lines.push((modifiers, vec![combo.key])),
        }
    }
    lines
        .iter()
        .map(|(modifiers, keys)| format!("{modifiers} ({}): {}", keys.len(), keys.join(", ")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_and_three_key_combinations() {
        assert_eq!(parse_combo("lalt+f"), Some(Combo::two("lalt", "f")));
        assert_eq!(parse_combo("f5+LAlt"), Some(Combo::two("lalt", "f5")));
        assert_eq!(parse_combo("kb1_rctrl+lbracket"), Some(Combo::two("rctrl", "lbracket")));
        assert_eq!(parse_combo("kb2_rshift+np_1"), Some(Combo::two("rshift", "np_1")));
        assert_eq!(parse_combo("kb1_lshift+lalt+x"), Some(Combo::three("lalt", "lshift", "x")));
        assert_eq!(parse_combo("lctrl+x+lalt"), Some(Combo::three("lalt", "lctrl", "x")));
        assert_eq!(parse_combo("capslock+a"), None);
        assert_eq!(parse_combo("f"), None);
        assert_eq!(parse_combo("kb1_ "), None);
        assert_eq!(parse_combo("lalt+mouse1"), None);
        assert_eq!(parse_combo("lalt+lshift"), None);
        assert_eq!(parse_combo("lalt+lalt+x"), None);
        assert_eq!(parse_combo("lctrl+lalt+lshift+x"), None);
        assert_eq!(Combo::three("lalt", "lctrl", "a").to_string(), "lalt+lctrl+a");
    }

    #[test]
    fn collects_profile_and_rebind_inputs() {
        let profile = crate::xml_doc::from_str(
            r#"<profile><actionmap name="m">
                <action name="a" keyboard="lalt+f"/>
                <action name="b"><keyboard input="lctrl+g"/></action>
                <action name="c"><keyboard><inputdata input="rshift+h"/></keyboard></action>
            </actionmap></profile>"#,
        )
        .unwrap();
        assert_eq!(profile_keyboard_inputs(&profile), ["lalt+f", "lctrl+g", "rshift+h"]);

        let rebinds = crate::xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="a">
                <rebind input="kb1_ralt+1"/><rebind input="js1_button3"/>
            </action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        assert_eq!(rebind_keyboard_inputs(&rebinds), ["kb1_ralt+1"]);
    }

    #[test]
    fn pairs_mix_modifier_kinds() {
        let pairs = modifier_pairs();
        assert_eq!(pairs.len(), 12);
        assert!(pairs.contains(&("lalt", "lctrl")));
        assert!(pairs.contains(&("rctrl", "lshift")));
        assert!(!pairs.contains(&("lalt", "ralt")));
        assert!(!pairs.contains(&("lshift", "rshift")));
    }

    #[test]
    fn windows_combinations_are_reserved() {
        assert!(Combo::two("lalt", "f4").is_reserved());
        assert!(Combo::three("lalt", "lshift", "tab").is_reserved());
        assert!(Combo::three("ralt", "lctrl", "tab").is_reserved());
        assert!(Combo::three("lctrl", "lshift", "escape").is_reserved());
        assert!(Combo::three("lalt", "rctrl", "delete").is_reserved());
        assert!(Combo::three("lalt", "lctrl", "np_period").is_reserved());
        assert!(!Combo::two("lshift", "escape").is_reserved());
        assert!(!Combo::three("lalt", "lshift", "delete").is_reserved());
    }

    #[test]
    fn two_key_combinations_come_first() {
        let candidates = candidates();
        let first_three = candidates.iter().position(|combo| combo.second.is_some()).unwrap();
        assert_eq!(first_three, MODIFIERS.len() * KEYS.len() - RESERVED.len());
        assert!(candidates[first_three..].iter().all(|combo| combo.second.is_some()));
        assert_eq!(candidates[first_three], Combo::three("lalt", "lctrl", "f1"));
    }

    #[test]
    fn unused_leaves_out_bound_and_reserved() {
        let bound = bound_combos(["lalt+a", "kb1_lalt+b", "c", "kb1_lctrl+lalt+d"]);
        let unused = unused_combos(&bound);
        assert!(!unused.contains(&Combo::two("lalt", "a")));
        assert!(!unused.contains(&Combo::two("lalt", "b")));
        assert!(!unused.contains(&Combo::three("lalt", "lctrl", "d")));
        assert!(!unused.contains(&Combo::two("lalt", "f4")));
        assert!(unused.contains(&Combo::two("lalt", "c")));
        assert!(unused.contains(&Combo::three("lalt", "lctrl", "a")));
        assert_eq!(unused.len(), candidates().len() - 3);
        let text = format_by_modifier(&unused);
        assert!(text.starts_with("lalt ("), "{text}");
        assert!(text.contains("\nlalt+lctrl ("), "{text}");
    }
}
