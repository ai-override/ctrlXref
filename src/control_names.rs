//! The game's names for its controls, from `keybinding_localization.xml`.

use xmltree::Element;

use crate::keys;
use crate::localization::Strings;
use crate::rebinds;
use crate::xml_doc;

/// One `<Key>` of the file, with its `@key` references resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlName {
    pub name: String,
    pub localization: String,
    pub short: String,
    /// The name of its `<device>`, with `joystick` for any of the numbered joysticks.
    pub device: String,
}

impl ControlName {
    /// An example of this control as a rebind input, as the actions table's device columns show
    /// it. A keyboard modifier is shown with a key, and the mouse names of the keyboard's list
    /// with a modifier, as the game only combines those. Empty for a device without a prefix.
    pub fn example(&self) -> String {
        let name = self.name.as_str();
        let device = if self.device == "control_pad" { "gamepad" } else { self.device.as_str() };
        let prefix = rebinds::device_prefix(device);
        if prefix.is_empty() || name.is_empty() {
            return String::new();
        }
        if device == "keyboard" && keys::MODIFIERS.contains(&name) {
            format!("{prefix}{name}+o")
        } else if device == "keyboard" && ["mouse", "maxis", "mwheel"].iter().any(|start| name.starts_with(start)) {
            format!("{prefix}lctrl+{name}")
        } else {
            format!("{prefix}{name}")
        }
    }
}

/// The one joystick device that is read. The file repeats the same list for `joystick1` to
/// `joystick4`, and `joystick1` has a typo in it: `at3_up` for `hat3_up`.
const SHOWN_JOYSTICK: &str = "joystick2";

/// Whether `element` is a `<device>` that only repeats the list of [`SHOWN_JOYSTICK`].
fn is_repeated_joystick(element: &Element) -> bool {
    element.name == "device"
        && element
            .attributes
            .get("name")
            .is_some_and(|name| name.starts_with("joystick") && name != SHOWN_JOYSTICK)
}

/// Every `<Key>` under `root`, in file order, with its texts resolved through `strings`, except
/// those of the joysticks that repeat [`SHOWN_JOYSTICK`]. A reference `strings` does not have stays
/// as it is; a missing attribute is empty.
pub fn read(root: &Element, strings: &Strings) -> Vec<ControlName> {
    fn walk(element: &Element, device: &str, strings: &Strings, names: &mut Vec<ControlName>) {
        for child in xml_doc::child_elements(element).filter(|child| !is_repeated_joystick(child)) {
            let device = match child.attributes.get("name") {
                Some(name) if child.name == "device" => device_label(name),
                _ => device,
            };
            if child.name == "Key" {
                let attribute = |name: &str| child.attributes.get(name).map_or("", String::as_str);
                names.push(ControlName {
                    name: attribute("name").to_string(),
                    localization: strings.resolve(attribute("localizationString")).to_string(),
                    short: strings.resolve(attribute("shortLocString")).to_string(),
                    device: device.to_string(),
                });
            }
            walk(child, device, strings, names);
        }
    }
    let mut names = Vec::new();
    walk(root, "", strings, &mut names);
    names
}

/// The device name as shown: `joystick` for `joystick1` and so on, since they all have the same
/// keys; other names as they are.
fn device_label(name: &str) -> &str {
    if name.starts_with("joystick") { "joystick" } else { name }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"<LocalizedKeys>
        <device name="keyboard">
            <Key name="a" localizationString="@input_a" shortLocString="" iconName="" />
            <Key name="lalt" localizationString="@input_lalt" shortLocString="@input_lalt_short" iconName="" />
        </device>
        <device name="joystick1">
            <Key name="at3_up" localizationString="@input_j1" />
        </device>
        <device name="joystick2">
            <Key name="x" localizationString="@input_missing" />
            <Key name="y" localizationString="Plain text" shortLocString="@ " />
        </device>
        <device name="joystick3">
            <Key name="x" localizationString="@input_j3" />
        </device>
        <device name="joystick4">
            <Key name="x" localizationString="@input_j4" />
        </device>
    </LocalizedKeys>"#;

    fn name(name: &str, localization: &str, short: &str, device: &str) -> ControlName {
        ControlName { name: name.into(), localization: localization.into(), short: short.into(), device: device.into() }
    }

    #[test]
    fn keys_are_read_in_file_order_with_their_texts_resolved() {
        let strings = Strings::parse("input_a=A\ninput_lalt=Left Alt\ninput_lalt_short=LAlt\n");
        let names = read(&xml_doc::from_str(FILE).unwrap(), &strings);
        assert_eq!(
            names,
            [
                name("a", "A", "", "keyboard"),
                name("lalt", "Left Alt", "LAlt", "keyboard"),
                // Not in the texts: the reference stays, to show what is missing.
                name("x", "@input_missing", "", "joystick"),
                name("y", "Plain text", "", "joystick"),
            ]
        );
    }

    #[test]
    fn of_the_joysticks_only_the_second_is_read_and_other_devices_all_are() {
        let root = xml_doc::from_str(
            r#"<LocalizedKeys>
                <device name="mouse"><Key name="m"/></device>
                <device name="control_pad"><Key name="p"/></device>
                <device name="joystick1"><Key name="at3_up"/></device>
                <device name="joystick2"><Key name="hat3_up"/></device>
                <device name="joystick3"><Key name="hat3_up"/></device>
            </LocalizedKeys>"#,
        )
        .unwrap();
        let names: Vec<(String, String)> =
            read(&root, &Strings::default()).into_iter().map(|name| (name.name, name.device)).collect();
        let expected = [("m", "mouse"), ("p", "control_pad"), ("hat3_up", "joystick")];
        assert_eq!(names, expected.map(|(name, device)| (name.to_string(), device.to_string())));
    }

    #[test]
    fn examples_are_written_like_rebind_inputs() {
        let example = |key: &str, device: &str| name(key, "", "", device).example();
        assert_eq!(example("button21", "joystick"), "js1_button21");
        assert_eq!(example("maxis_x", "mouse"), "mo1_maxis_x");
        assert_eq!(example("a", "control_pad"), "gp1_a");
        assert_eq!(example("f1", "keyboard"), "kb1_f1");
        assert_eq!(example("ralt", "keyboard"), "kb1_ralt+o");
        assert_eq!(example("mouse3", "keyboard"), "kb1_lctrl+mouse3");
        assert_eq!(example("mwheel_up", "keyboard"), "kb1_lctrl+mwheel_up");
        assert_eq!(example("x", "unknown"), "");
        assert_eq!(example("", "keyboard"), "");
    }

    #[test]
    fn a_file_without_keys_has_no_names() {
        let root = xml_doc::from_str("<LocalizedKeys><device name=\"keyboard\"/></LocalizedKeys>").unwrap();
        assert!(read(&root, &Strings::default()).is_empty());
    }
}
