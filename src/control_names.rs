//! The game's names for its controls, from `keybinding_localization.xml`.

use xmltree::Element;

use crate::localization::Strings;
use crate::xml_doc;

/// One `<Key>` of the file, with its `@key` references resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlName {
    pub name: String,
    pub localization: String,
    pub short: String,
}

/// Every `<Key>` under `root`, in file order, with its texts resolved through `strings`. A
/// reference `strings` does not have stays as it is; a missing attribute is empty.
pub fn read(root: &Element, strings: &Strings) -> Vec<ControlName> {
    fn walk(element: &Element, strings: &Strings, names: &mut Vec<ControlName>) {
        for child in xml_doc::child_elements(element) {
            if child.name == "Key" {
                let attribute = |name: &str| child.attributes.get(name).map_or("", String::as_str);
                names.push(ControlName {
                    name: attribute("name").to_string(),
                    localization: strings.resolve(attribute("localizationString")).to_string(),
                    short: strings.resolve(attribute("shortLocString")).to_string(),
                });
            }
            walk(child, strings, names);
        }
    }
    let mut names = Vec::new();
    walk(root, strings, &mut names);
    names
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
            <Key name="x" localizationString="@input_missing" />
            <Key name="y" localizationString="Plain text" shortLocString="@ " />
        </device>
    </LocalizedKeys>"#;

    fn name(name: &str, localization: &str, short: &str) -> ControlName {
        ControlName { name: name.into(), localization: localization.into(), short: short.into() }
    }

    #[test]
    fn keys_are_read_in_file_order_with_their_texts_resolved() {
        let strings = Strings::parse("input_a=A\ninput_lalt=Left Alt\ninput_lalt_short=LAlt\n");
        let names = read(&xml_doc::from_str(FILE).unwrap(), &strings);
        assert_eq!(
            names,
            [
                name("a", "A", ""),
                name("lalt", "Left Alt", "LAlt"),
                // Not in the texts: the reference stays, to show what is missing.
                name("x", "@input_missing", ""),
                name("y", "Plain text", ""),
            ]
        );
    }

    #[test]
    fn a_file_without_keys_has_no_names() {
        let root = xml_doc::from_str("<LocalizedKeys><device name=\"keyboard\"/></LocalizedKeys>").unwrap();
        assert!(read(&root, &Strings::default()).is_empty());
    }
}
