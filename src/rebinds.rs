//! The rules of profiles and rebind files: finding actionmaps and actions, reading the bindings
//! of each device, editing `<rebind>` elements, and completing the game's blank layout.

use std::collections::{HashMap, HashSet};

use xmltree::{Element, XMLNode};

use crate::xml_doc;

/// The only elements listed in the tree; their children are the table rows.
const ACTIONMAP_TAG: &str = "actionmap";

/// Child element of an action in a rebind file; its `input` fills its device's column.
const REBIND_TAG: &str = "rebind";

/// Input devices; an action binds them in an attribute or in a child element of that name.
pub const DEVICES: [&str; 4] = ["keyboard", "mouse", "joystick", "gamepad"];

/// Rebind inputs by `(actionmap name, action name)`, in file order.
pub type Rebinds = HashMap<(String, String), Vec<String>>;

/// Element index paths of all `actionmap` elements, in document order. Actionmaps
/// nested in another actionmap are not searched for.
pub fn find_actionmaps(root: &Element) -> Vec<Vec<usize>> {
    fn walk(element: &Element, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if element.name == ACTIONMAP_TAG {
            out.push(path.clone());
            return;
        }
        for (ix, child) in xml_doc::child_elements(element).enumerate() {
            path.push(ix);
            walk(child, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(root, &mut Vec::new(), &mut out);
    out
}

/// Paths of the child elements of every actionmap in `maps`: the table rows.
pub fn action_paths(root: &Element, maps: &[Vec<usize>]) -> Vec<Vec<usize>> {
    maps.iter()
        .filter_map(|map| Some((map, xml_doc::element_at(root, map)?)))
        .flat_map(|(map, element)| {
            (0..xml_doc::child_elements(element).count()).map(move |ix| {
                let mut path = map.clone();
                path.push(ix);
                path
            })
        })
        .collect()
}

/// `(actionmap name, action name)` of the action at `path`, the key rebinds match on.
pub fn action_key(root: &Element, path: &[usize]) -> Option<(String, String)> {
    let map = xml_doc::element_at(root, path.split_last()?.1)?;
    let action = xml_doc::element_at(root, path)?;
    let name = |element: &Element| element.attributes.get("name").cloned().unwrap_or_default();
    Some((name(map), name(action)))
}

/// The profile's `device` input of `action`: its attribute, else its child element's.
pub fn profile_input(action: &Element, device: &str) -> Option<String> {
    match action.attributes.get(device) {
        Some(value) => Some(value.clone()),
        None => child_device_input(action, device),
    }
}

/// How many actions in `rows` have no keyboard input once `rebinds` are applied.
pub fn count_empty_keyboard(root: &Element, rows: &[Vec<usize>], rebinds: &Rebinds) -> usize {
    rows.iter().filter(|path| has_empty_keyboard(root, path, rebinds)).count()
}

/// Whether the action at `path` has no keyboard input. Its keyboard rebinds replace the
/// profile's binding; a blank binding like `keyboard=" "` or only empty rebinds like
/// `kb1_ ` bind nothing.
fn has_empty_keyboard(root: &Element, path: &[usize], rebinds: &Rebinds) -> bool {
    let Some(action) = xml_doc::element_at(root, path) else {
        return false;
    };
    let rebound: Vec<String> = action_key(root, path)
        .and_then(|key| rebinds.get(&key))
        .map(|inputs| inputs.iter().filter(|input| rebind_device(input) == Some(DEVICES[0])).cloned().collect())
        .unwrap_or_default();
    let value = if rebound.is_empty() {
        profile_input(action, DEVICES[0]).unwrap_or_default()
    } else {
        rebound.join(",")
    };
    !value.split(',').map(str::trim).any(|input| !input.is_empty() && !is_empty_rebind(input))
}

/// `(actionmap name, action name)` of each action in `rows`, in order, that has no
/// keyboard input once `rebinds` are applied.
pub fn unbound_keyboard_actions(root: &Element, rows: &[Vec<usize>], rebinds: &Rebinds) -> Vec<(String, String)> {
    rows.iter()
        .filter(|path| has_empty_keyboard(root, path, rebinds))
        .filter_map(|path| action_key(root, path))
        .collect()
}

/// The `<action>` named `action` in actionmap `map` of a rebind file, creating the action
/// and the actionmap when the file has none yet.
fn action_mut<'a>(root: &'a mut Element, map: &str, action: &str) -> &'a mut Element {
    if find_named_mut(root, ACTIONMAP_TAG, map).is_none() {
        let parent = actionmap_parent_mut(root);
        parent.children.push(XMLNode::Element(named_element(ACTIONMAP_TAG, map)));
    }
    let map_element = find_named_mut(root, ACTIONMAP_TAG, map).expect("the actionmap exists or was just added");
    match map_element
        .children
        .iter()
        .position(|node| node.as_element().is_some_and(|child| is_named(child, "action", action)))
    {
        Some(ix) => map_element.children[ix].as_mut_element().expect("an element"),
        None => {
            map_element.children.push(XMLNode::Element(named_element("action", action)));
            map_element.children.last_mut().and_then(XMLNode::as_mut_element).expect("an element")
        }
    }
}

/// The rebind input prefix a device column's inputs get when typed without one: the
/// device's code and its first instance.
pub fn device_prefix(device: &str) -> &'static str {
    match device {
        "keyboard" => "kb1_",
        "mouse" => "mo1_",
        "joystick" => "js1_",
        "gamepad" => "gp1_",
        _ => "",
    }
}

/// The device of an input that starts with a device prefix, like `kb1_w` or `js2_button3`.
fn prefixed_device(input: &str) -> Option<&'static str> {
    let (head, _) = input.split_once('_')?;
    let instance = head.get(2..)?;
    if instance.is_empty() || !instance.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    rebind_device(head)
}

/// The rebind inputs typed into a `device` column cell: separated by commas or vertical
/// bars (as ` | ` shows them), trimmed, blanks dropped, and given the
/// column's [`device_prefix`] when they have none, so orphaned separators leave nothing behind.
/// An input repeated in the cell is kept once. `existing` holds the action's inputs before the
/// edit: one whose prefix was damaged gets that input's prefix back, so `button1` or `s3_button1`
/// stays `js3_button1`. An input prefixed for another device, or without a control, is an error.
pub fn parse_cell_inputs(device: &str, text: &str, existing: &[String]) -> Result<Vec<String>, String> {
    let typed: Vec<&str> = text.split([',', '|']).map(str::trim).filter(|input| !input.is_empty()).collect();
    // The inputs that were in the cell and are no longer there as they were.
    let mut changed: Vec<&String> = existing
        .iter()
        .filter(|old| rebind_device(old) == Some(device) && !is_empty_rebind(old) && !typed.contains(&old.as_str()))
        .collect();
    let mut inputs: Vec<String> = Vec::new();
    for input in &typed {
        let input = match changed.iter().position(|old| restores(old, input)) {
            Some(ix) => changed.remove(ix).clone(),
            None => cell_input(device, input)?,
        };
        if !inputs.contains(&input) {
            inputs.push(input);
        }
    }
    Ok(inputs)
}

/// Whether `input` is the rebind input `old` with part of its device prefix missing.
fn restores(old: &str, input: &str) -> bool {
    let Some((head, rest)) = old.split_once('_') else {
        return false;
    };
    // What is left of the prefix must be some of its characters, in order.
    let mut prefix = head.chars().chain(['_']);
    input.strip_suffix(rest).is_some_and(|left| left.chars().all(|c| prefix.any(|p| p == c)))
}

/// The rebind input for `input` typed into a `device` column cell, as described at
/// [`parse_cell_inputs`]. An empty rebind like `js2_` is written as the game writes it, `js2_ `.
fn cell_input(device: &str, input: &str) -> Result<String, String> {
    let wrong = |found: &str| format!("\"{input}\" is a {found} input and can't go in the {device} column");
    if let Some(found) = prefixed_device(input) {
        return match found == device {
            true if is_empty_rebind(input) => Ok(format!("{input} ")),
            true => Ok(input.to_string()),
            false => Err(wrong(found)),
        };
    }
    let no_control = || format!("\"{input}\" has no control after its device prefix");
    if rebind_device(input).is_some() && input[2..].chars().all(|c| c.is_ascii_digit()) {
        return Err(no_control());
    }
    // A prefix without its number, like `js_button1`.
    let name = match input.split_once('_') {
        Some((head, rest)) if head.len() == 2 && rebind_device(head).is_some() => {
            let found = rebind_device(head).unwrap_or(device);
            if found != device {
                return Err(wrong(found));
            }
            rest
        }
        _ => input.trim_start_matches('_'),
    };
    if name.is_empty() {
        return Err(no_control());
    }
    Ok(format!("{}{name}", device_prefix(device)))
}

/// Makes `inputs` the `device` rebinds of `action` in actionmap `map` of a rebind file, one
/// `<rebind>` each in that order, where its old `device` rebinds were. A rebind kept from
/// before keeps its other attributes. Creates the action and the actionmap when needed, and
/// removes the action when clearing leaves it empty.
pub fn set_device_rebinds(root: &mut Element, map: &str, action: &str, device: &str, inputs: &[String]) {
    let is_device_rebind = |node: &XMLNode| {
        node.as_element().is_some_and(|child| {
            child.name == REBIND_TAG && rebind_input(child).is_some_and(|input| rebind_device(&input) == Some(device))
        })
    };
    let exists = find_named_mut(root, ACTIONMAP_TAG, map).is_some_and(|map_element| {
        xml_doc::child_elements(map_element).any(|child| is_named(child, "action", action))
    });
    if inputs.is_empty() && !exists {
        return;
    }
    let action_element = action_mut(root, map, action);
    let at = action_element.children.iter().position(is_device_rebind);
    let mut old: Vec<Element> = Vec::new();
    action_element.children.retain(|node| match node.as_element() {
        Some(child) if is_device_rebind(node) => {
            old.push(child.clone());
            false
        }
        _ => true,
    });
    let new: Vec<XMLNode> = inputs
        .iter()
        .map(|input| {
            let kept = old.iter().position(|rebind| rebind_input(rebind).as_ref() == Some(input));
            XMLNode::Element(match kept {
                Some(ix) => old.remove(ix),
                None => {
                    let mut rebind = Element::new(REBIND_TAG);
                    rebind.attributes.insert("input".into(), input.clone());
                    rebind
                }
            })
        })
        .collect();
    let at = at.unwrap_or(action_element.children.len());
    action_element.children.splice(at..at, new);

    if xml_doc::child_elements(action_element).next().is_none()
        && let Some(map_element) = find_named_mut(root, ACTIONMAP_TAG, map)
    {
        map_element
            .children
            .retain(|node| !node.as_element().is_some_and(|child| is_named(child, "action", action)));
    }
}

/// Removes all `device` rebinds of `actions` (`(actionmap name, action name)`) from a rebind file
/// whose inputs are `rebinds`. Returns how many there were. Rebinds of actions not listed stay.
pub fn clear_device_rebinds(root: &mut Element, rebinds: &Rebinds, actions: &[(String, String)], device: &str) -> usize {
    let mut cleared = 0;
    for key in actions {
        let of_device = rebinds
            .get(key)
            .map_or(0, |inputs| inputs.iter().filter(|input| rebind_device(input) == Some(device)).count());
        if of_device > 0 {
            set_device_rebinds(root, &key.0, &key.1, device, &[]);
            cleared += of_device;
        }
    }
    cleared
}

/// Appends the rebinds of the rebind file `other` to `root`, after the ones each action has,
/// keeping all their attributes and creating actions and actionmaps when needed. Inputs that
/// conflict are all kept, for the user to choose from; only an input the action already has is
/// left out. Returns how many rebinds were added.
pub fn merge_rebinds(root: &mut Element, other: &Element) -> usize {
    let mut have = read_rebinds(root);
    let mut added = 0;
    for path in find_actionmaps(other) {
        let Some(map) = xml_doc::element_at(other, &path) else {
            continue;
        };
        let map_name = map.attributes.get("name").cloned().unwrap_or_default();
        for action in xml_doc::child_elements(map) {
            let Some(action_name) = action.attributes.get("name") else {
                continue;
            };
            let inputs = have.entry((map_name.clone(), action_name.clone())).or_default();
            for rebind in xml_doc::child_elements(action).filter(|child| child.name == REBIND_TAG) {
                let Some(input) = rebind_input(rebind) else {
                    continue;
                };
                if !inputs.contains(&input) {
                    inputs.push(input);
                    action_mut(root, &map_name, action_name).children.push(XMLNode::Element(rebind.clone()));
                    added += 1;
                }
            }
        }
    }
    added
}

/// Binds `input` (a `kb...` keyboard input) to `action` of actionmap `map` in a rebind
/// file: fills its empty keyboard rebind, or adds a `<rebind>`, creating the action and
/// the actionmap when the file has none yet.
pub fn set_keyboard_rebind(root: &mut Element, map: &str, action: &str, input: &str) {
    let action_element = action_mut(root, map, action);
    let empty_keyboard = action_element.children.iter_mut().filter_map(XMLNode::as_mut_element).find(|child| {
        child.name == REBIND_TAG
            && rebind_input(child).is_some_and(|value| rebind_device(&value) == Some(DEVICES[0]) && is_empty_rebind(&value))
    });
    match empty_keyboard {
        Some(rebind) => {
            rebind.attributes.insert("input".into(), input.into());
        }
        None => {
            let mut rebind = Element::new(REBIND_TAG);
            rebind.attributes.insert("input".into(), input.into());
            action_element.children.push(XMLNode::Element(rebind));
        }
    }
}

fn is_named(element: &Element, tag: &str, name: &str) -> bool {
    element.name == tag && element.attributes.get("name").is_some_and(|value| value == name)
}

/// Makes the blank layout `root` hold exactly `actions` (`(actionmap name, action name)`, in
/// profile order), since the profile is more up to date than the layout. Each action the
/// layout lacks is added with blank rebinds like the layout's own actions have: one per
/// device its first action clears, keeping only `device` and `input`. Missing actionmaps are
/// added after the existing ones. Layout actions not in `actions` are removed, and so are
/// actionmaps left without actions. Everything else, its `CustomisationUIHeader` with
/// `Devices` and its `modifiers` included, is kept as it is. Returns how many actions were
/// added and removed.
pub fn complete_blank_layout(root: &mut Element, actions: &[(String, String)]) -> (usize, usize) {
    let blank = blank_rebinds(root);
    let mut added = 0;
    for (map, action) in actions {
        let exists = find_named_mut(root, ACTIONMAP_TAG, map)
            .is_some_and(|map_element| xml_doc::child_elements(map_element).any(|child| is_named(child, "action", action)));
        if !exists {
            action_mut(root, map, action).children = blank.iter().cloned().map(XMLNode::Element).collect();
            added += 1;
        }
    }
    let keep: HashSet<(&str, &str)> = actions.iter().map(|(map, action)| (map.as_str(), action.as_str())).collect();
    let removed = remove_actions_not_in(root, &keep);
    (added, removed)
}

/// Removes from `element` and its descendants every action whose `(actionmap name, action
/// name)` is not in `keep`, then every actionmap left without actions. Returns how many
/// actions were removed.
fn remove_actions_not_in(element: &mut Element, keep: &HashSet<(&str, &str)>) -> usize {
    let mut removed = 0;
    if element.name == ACTIONMAP_TAG {
        let map = element.attributes.get("name").cloned().unwrap_or_default();
        element.children.retain(|node| {
            let stale = node.as_element().is_some_and(|child| {
                child.name == "action"
                    && !keep.contains(&(map.as_str(), child.attributes.get("name").map_or("", String::as_str)))
            });
            removed += usize::from(stale);
            !stale
        });
        return removed;
    }
    for child in element.children.iter_mut().filter_map(XMLNode::as_mut_element) {
        removed += remove_actions_not_in(child, keep);
    }
    element.children.retain(|node| {
        node.as_element().is_none_or(|child| {
            child.name != ACTIONMAP_TAG || xml_doc::child_elements(child).any(|action| action.name == "action")
        })
    });
    removed
}

/// The blank `<rebind>` elements for a new action of a blank layout: those of its first
/// action that has rebinds, with only `device` and `input`. Without one, a blank rebind
/// for the keyboard, mouse, gamepad and first joystick.
fn blank_rebinds(root: &Element) -> Vec<Element> {
    fn first_rebinds(element: &Element) -> Option<Vec<&Element>> {
        if element.name == "action" {
            let rebinds: Vec<&Element> = xml_doc::child_elements(element).filter(|c| c.name == REBIND_TAG).collect();
            return (!rebinds.is_empty()).then_some(rebinds);
        }
        xml_doc::child_elements(element).find_map(first_rebinds)
    }
    let rebind = |device: &str, input: &str| {
        let mut element = Element::new(REBIND_TAG);
        element.attributes.insert("device".into(), device.into());
        element.attributes.insert("input".into(), input.into());
        element
    };
    match first_rebinds(root) {
        Some(rebinds) => rebinds
            .into_iter()
            .map(|template| {
                let attribute = |name: &str| template.attributes.get(name).map_or("", String::as_str);
                rebind(attribute("device"), attribute("input"))
            })
            .collect(),
        None => vec![
            rebind("gamepad", " "),
            rebind("keyboard", " "),
            rebind("mouse", " "),
            rebind("joystick", "js1_ "),
        ],
    }
}

fn named_element(tag: &str, name: &str) -> Element {
    let mut element = Element::new(tag);
    element.attributes.insert("name".into(), name.into());
    element
}

/// The first `tag` element named `name` in `element` or its descendants.
fn find_named_mut<'a>(element: &'a mut Element, tag: &str, name: &str) -> Option<&'a mut Element> {
    if is_named(element, tag, name) {
        return Some(element);
    }
    element
        .children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .find_map(|child| find_named_mut(child, tag, name))
}

/// Where new actionmaps go in a rebind file: next to its existing ones, else into its
/// `<ActionProfiles>` element, else into the root.
fn actionmap_parent_mut(root: &mut Element) -> &mut Element {
    fn holds_actionmap(element: &Element) -> bool {
        xml_doc::child_elements(element).any(|child| child.name == ACTIONMAP_TAG)
            || xml_doc::child_elements(element).any(holds_actionmap)
    }
    fn find(element: &mut Element) -> &mut Element {
        if xml_doc::child_elements(element).any(|child| child.name == ACTIONMAP_TAG) {
            return element;
        }
        let ix = element
            .children
            .iter()
            .position(|node| node.as_element().is_some_and(holds_actionmap));
        match ix {
            Some(ix) => find(element.children[ix].as_mut_element().expect("an element")),
            None => element,
        }
    }
    if holds_actionmap(root) {
        return find(root);
    }
    let profiles = root
        .children
        .iter()
        .position(|node| node.as_element().is_some_and(|child| child.name == "ActionProfiles"));
    match profiles {
        Some(ix) => root.children[ix].as_mut_element().expect("an element"),
        None => root,
    }
}

/// The device column of a rebind input, from its prefix: `kb` keyboard, `mo` mouse,
/// `js` joystick and `gp` gamepad.
pub fn rebind_device(input: &str) -> Option<&'static str> {
    match input.get(..2)? {
        "kb" => Some(DEVICES[0]),
        "mo" => Some(DEVICES[1]),
        "js" => Some(DEVICES[2]),
        "gp" => Some(DEVICES[3]),
        _ => None,
    }
}

/// Whether `input` names a device but no input, like `kb1_ ` or `js2_`. The game leaves
/// these behind once a binding has been edited, as it cannot remove the element.
pub fn is_empty_rebind(input: &str) -> bool {
    input.split_once('_').is_some_and(|(_, rest)| rest.trim().is_empty())
}

/// The input of a `<rebind>` element, with a device prefix. The game's blank layout writes
/// some without one, like `<rebind device="keyboard" input=" "/>`; those get the prefix of
/// their `device` attribute, so that one reads as the empty rebind `kb1_ `.
fn rebind_input(rebind: &Element) -> Option<String> {
    let input = rebind.attributes.get("input")?;
    if rebind_device(input).is_some() {
        return Some(input.clone());
    }
    let device = rebind.attributes.get("device").map(|device| device.to_ascii_lowercase());
    let device = match device.as_deref() {
        Some("xboxpad") => DEVICES[3],
        Some(device) => DEVICES.into_iter().find(|known| *known == device)?,
        None => return Some(input.clone()),
    };
    let input = input.trim();
    Some(format!("{}{}", device_prefix(device), if input.is_empty() { " " } else { input }))
}

/// The input of a device bound in a child element instead of an attribute: either
/// `<gamepad input="..."/>` or several `<keyboard><inputdata input="..."/></keyboard>`.
fn child_device_input(action: &Element, device: &str) -> Option<String> {
    let element = xml_doc::child_elements(action).find(|child| child.name == device)?;
    if let Some(input) = element.attributes.get("input") {
        return Some(input.clone());
    }
    let inputs: Vec<&str> = xml_doc::child_elements(element)
        .filter_map(|data| data.attributes.get("input"))
        .map(String::as_str)
        .collect();
    (!inputs.is_empty()).then(|| inputs.join(", "))
}

/// The `<rebind input="..."/>` inputs of every action in a rebind file's actionmaps, as
/// [`rebind_input`] reads them.
pub fn read_rebinds(root: &Element) -> Rebinds {
    let mut rebinds = Rebinds::new();
    for path in find_actionmaps(root) {
        let Some(map) = xml_doc::element_at(root, &path) else {
            continue;
        };
        let map_name = map.attributes.get("name").cloned().unwrap_or_default();
        for action in xml_doc::child_elements(map) {
            let inputs: Vec<String> = xml_doc::child_elements(action)
                .filter(|child| child.name == REBIND_TAG)
                .filter_map(rebind_input)
                .collect();
            if let (false, Some(name)) = (inputs.is_empty(), action.attributes.get("name")) {
                rebinds.insert((map_name.clone(), name.clone()), inputs);
            }
        }
    }
    rebinds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_appends_conflicts_and_leaves_out_what_the_action_has() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><ActionProfiles profileName="default">
                <actionmap name="m"><action name="a"><rebind input="kb1_x"/><rebind input="js1_button1"/></action></actionmap>
            </ActionProfiles></ActionMaps>"#,
        )
        .unwrap();
        let other = xml_doc::from_str(
            r#"<ActionMaps>
                <actionmap name="m">
                    <action name="a"><rebind input="kb1_x"/><rebind input="kb1_y" multiTap="2"/></action>
                    <action name="b"><rebind input="mo1_mouse4"/></action>
                </actionmap>
                <actionmap name="new"><action name="c"><rebind device="keyboard" input=" "/></action></actionmap>
            </ActionMaps>"#,
        )
        .unwrap();
        assert_eq!(merge_rebinds(&mut root, &other), 3);
        let rebinds = read_rebinds(&root);
        assert_eq!(rebinds[&("m".into(), "a".into())], ["kb1_x", "js1_button1", "kb1_y"]);
        assert_eq!(rebinds[&("m".into(), "b".into())], ["mo1_mouse4"]);
        assert_eq!(rebinds[&("new".into(), "c".into())], ["kb1_ "]);
        assert!(xml_doc::to_string(&root).unwrap().contains(r#"<rebind input="kb1_y" multiTap="2""#));
        // New actionmaps go next to the existing ones, and merging again adds nothing.
        assert_eq!(xml_doc::child_elements(xml_doc::child_elements(&root).next().unwrap()).count(), 2);
        assert_eq!(merge_rebinds(&mut root, &other), 0);
    }

    #[test]
    fn blank_keyboard_cells_count_as_empty() {
        let profile = xml_doc::from_str(
            r#"<p><actionmap name="m">
                <action name="a" keyboard=" "/><action name="b"/><action name="c" keyboard="f"/>
                <action name="d" keyboard=" "/>
            </actionmap></p>"#,
        )
        .unwrap();
        let rebind_file = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="d"><rebind input="kb1_g"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        // a and b are unbound; d's blank keyboard is filled by its rebind.
        assert_eq!(count_empty_keyboard(&profile, &rows, &read_rebinds(&rebind_file)), 2);
    }

    #[test]
    fn autofill_targets_actions_without_keyboard() {
        let profile = xml_doc::from_str(
            r#"<p><actionmap name="m">
                <action name="a" keyboard="f"/>
                <action name="b" keyboard=" "/>
                <action name="c"/>
                <action name="d" keyboard=" "/>
            </actionmap></p>"#,
        )
        .unwrap();
        let rebind_file = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="b"><rebind input="kb1_g"/></action>
                <action name="d"><rebind input="kb1_ "/><rebind input="js1_button1"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let unbound = unbound_keyboard_actions(&profile, &rows, &read_rebinds(&rebind_file));
        // a is bound by the profile and b by its rebind; d's only keyboard rebind is empty.
        assert_eq!(unbound, [("m".into(), "c".into()), ("m".into(), "d".into())]);
    }

    #[test]
    fn keyboard_rebinds_fill_empty_ones_or_are_added() {
        let mut rebind_file = xml_doc::from_str(
            r#"<ActionMaps><ActionProfiles profileName="default">
                <actionmap name="m"><action name="d"><rebind input="kb1_ "/><rebind input="js1_button1"/></action></actionmap>
            </ActionProfiles></ActionMaps>"#,
        )
        .unwrap();
        set_keyboard_rebind(&mut rebind_file, "m", "d", "kb1_lctrl+a");
        set_keyboard_rebind(&mut rebind_file, "m", "v_set_x", "kb1_lctrl+b");
        set_keyboard_rebind(&mut rebind_file, "new_map", "v_set_y", "kb1_ralt+c");
        let rebinds = read_rebinds(&rebind_file);
        assert_eq!(rebinds[&("m".into(), "d".into())], ["kb1_lctrl+a", "js1_button1"]);
        assert_eq!(rebinds[&("m".into(), "v_set_x".into())], ["kb1_lctrl+b"]);
        assert_eq!(rebinds[&("new_map".into(), "v_set_y".into())], ["kb1_ralt+c"]);
        // The new actionmap sits next to the existing one, inside ActionProfiles.
        let xml = xml_doc::to_string(&rebind_file).unwrap();
        assert!(xml.contains("<action name=\"v_set_x\">\n"), "{xml}");
        let profiles = xml_doc::child_elements(&rebind_file).next().unwrap();
        assert_eq!(xml_doc::child_elements(profiles).count(), 2);
    }

    #[test]
    fn blank_layout_gets_the_missing_profile_actions() {
        let mut layout = xml_doc::from_str(
            r#"<ActionMaps version="1">
                <CustomisationUIHeader label="@ui_CMBlank"><Devices><Keyboard instance="1"/><Joystick instance="2"/></Devices></CustomisationUIHeader>
                <modifiers><mod input=" "/></modifiers>
                <actionmap name="m1">
                    <action name="a">
                        <rebind device="keyboard" activationMode="press" input=" "/>
                        <rebind device="joystick" input="js1_ "/>
                        <rebind device="joystick" input="js2_ "/>
                    </action>
                    <action name="stale"><rebind device="keyboard" input=" "/></action>
                </actionmap>
                <actionmap name="old"><action name="gone"><rebind device="keyboard" input=" "/></action></actionmap>
            </ActionMaps>"#,
        )
        .unwrap();
        let actions: Vec<(String, String)> =
            [("m1", "a"), ("m1", "b"), ("m2", "c"), ("m2", "c")].map(|(m, a)| (m.into(), a.into())).into();
        assert_eq!(complete_blank_layout(&mut layout, &actions), (2, 2));

        let xml = xml_doc::to_string(&layout).unwrap();
        // The header, devices, modifiers and the layout's actions in the profile are kept.
        for kept in ["@ui_CMBlank", "<Joystick instance=\"2\"", "<modifiers>", "activationMode=\"press\""] {
            assert!(xml.contains(kept), "{kept} missing in {xml}");
        }
        // Layout actions not in the profile go, and so does the actionmap they leave empty.
        for gone in ["name=\"stale\"", "name=\"gone\"", "name=\"old\""] {
            assert!(!xml.contains(gone), "{gone} still in {xml}");
        }
        let names: Vec<&str> = xml_doc::child_elements(&layout)
            .filter(|child| child.name == ACTIONMAP_TAG)
            .map(|map| map.attributes["name"].as_str())
            .collect();
        assert_eq!(names, ["m1", "m2"]);
        // New actions get the first action's blank rebinds, without activationMode.
        let rebinds = read_rebinds(&layout);
        let expected: Vec<String> = ["kb1_ ", "js1_ ", "js2_ "].map(String::from).into();
        assert_eq!(rebinds[&("m1".into(), "b".into())], expected);
        assert_eq!(rebinds[&("m2".into(), "c".into())], expected);
        assert_eq!(xml.matches("activationMode").count(), 1);
    }

    #[test]
    fn cell_text_splits_on_commas_and_gets_the_column_prefix() {
        assert_eq!(
            parse_cell_inputs("keyboard", " up , kb2_numpad8,, lalt+k ", &[]).unwrap(),
            ["kb1_up", "kb2_numpad8", "kb1_lalt+k"]
        );
        // A conflict cell is shown with " | " between the inputs; editing it keeps them apart.
        assert_eq!(parse_cell_inputs("keyboard", "kb1_x | kb1_y", &[]).unwrap(), ["kb1_x", "kb1_y"]);
        assert_eq!(parse_cell_inputs("mouse", "mouse4", &[]).unwrap(), ["mo1_mouse4"]);
        assert_eq!(parse_cell_inputs("joystick", " ", &[]).unwrap(), Vec::<String>::new());
        assert!(parse_cell_inputs("keyboard", "w, js1_button2", &[]).is_err());
    }

    #[test]
    fn orphaned_separators_leave_valid_inputs() {
        for text in ["kb1_x |", "kb1_x | ", "| kb1_x", " | kb1_x | ", "kb1_x || ", "|kb1_x|"] {
            assert_eq!(parse_cell_inputs("keyboard", text, &[]).unwrap(), ["kb1_x"], "{text}");
        }
        assert_eq!(parse_cell_inputs("keyboard", " | ", &[]).unwrap(), Vec::<String>::new());
        // Several controls stay, in their order, and a repeated one once.
        assert_eq!(parse_cell_inputs("keyboard", "kb1_y | kb1_x |", &[]).unwrap(), ["kb1_y", "kb1_x"]);
        assert_eq!(parse_cell_inputs("keyboard", "kb1_x | kb1_x", &[]).unwrap(), ["kb1_x"]);
    }

    #[test]
    fn a_damaged_prefix_comes_back_from_what_the_cell_had() {
        let existing = ["js3_button1".to_string(), "js4_x".to_string(), "kb1_f".to_string()];
        for text in ["button1", "s3_button1", "3_button1", "js_button1", "js3button1", "_button1"] {
            assert_eq!(parse_cell_inputs("joystick", text, &existing).unwrap(), ["js3_button1"], "{text}");
        }
        assert_eq!(parse_cell_inputs("joystick", "button1 | x", &existing).unwrap(), ["js3_button1", "js4_x"]);
        // Another control gets the column's own prefix, and so does a modifier added to a key.
        assert_eq!(parse_cell_inputs("joystick", "button2", &existing).unwrap(), ["js1_button2"]);
        assert_eq!(parse_cell_inputs("keyboard", "lalt+f", &existing).unwrap(), ["kb1_lalt+f"]);
        // The same control of another joystick that is still there is not taken for it.
        let both = ["js1_x".to_string(), "js2_x".to_string()];
        assert_eq!(parse_cell_inputs("joystick", "js1_x | x", &both).unwrap(), ["js1_x", "js2_x"]);
    }

    #[test]
    fn a_prefix_without_a_control_is_not_made_into_one() {
        for text in ["kb1", "kb", "_", "kb_"] {
            assert!(parse_cell_inputs("keyboard", text, &[]).is_err(), "{text}");
        }
        assert!(parse_cell_inputs("keyboard", "js_x", &[]).is_err());
        assert_eq!(parse_cell_inputs("joystick", "js_x", &[]).unwrap(), ["js1_x"]);
        assert_eq!(parse_cell_inputs("keyboard", "np_1", &[]).unwrap(), ["kb1_np_1"]);
    }

    #[test]
    fn empty_rebinds_stay_as_the_game_writes_them_when_a_cell_is_edited() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="a">
                <rebind device="joystick" input="js1_ " tag="kept"/><rebind input="js2_x"/><rebind input="js2_y"/>
            </action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let existing = read_rebinds(&root)[&("m".to_string(), "a".to_string())].clone();
        let inputs = parse_cell_inputs("joystick", "js1_ | js2_x", &existing).unwrap();
        assert_eq!(inputs, ["js1_ ", "js2_x"]);
        set_device_rebinds(&mut root, "m", "a", "joystick", &inputs);
        assert!(xml_doc::to_string(&root).unwrap().contains(r#"tag="kept""#));
    }

    #[test]
    fn edited_cells_replace_that_devices_rebinds_only() {
        let mut rebind_file = xml_doc::from_str(
            r#"<ActionMaps><ActionProfiles profileName="default">
                <actionmap name="m"><action name="a"><rebind input="kb1_x" multiTap="2"/><rebind input="js1_button1"/><rebind input="kb1_y"/></action></actionmap>
            </ActionProfiles></ActionMaps>"#,
        )
        .unwrap();
        let inputs = |list: &[&str]| list.iter().map(|input| input.to_string()).collect::<Vec<_>>();

        // Each input gets a <rebind> of its own; a kept one keeps its attributes.
        set_device_rebinds(&mut rebind_file, "m", "a", "keyboard", &inputs(&["kb1_up", "kb1_x"]));
        let rebinds = read_rebinds(&rebind_file);
        assert_eq!(rebinds[&("m".into(), "a".into())], ["kb1_up", "kb1_x", "js1_button1"]);
        let xml = xml_doc::to_string(&rebind_file).unwrap();
        assert!(xml.contains(r#"<rebind input="kb1_x" multiTap="2""#), "{xml}");

        // An action only in the profile gets an <action> element.
        set_device_rebinds(&mut rebind_file, "m", "b", "mouse", &inputs(&["mo1_mouse4"]));
        assert_eq!(read_rebinds(&rebind_file)[&("m".into(), "b".into())], ["mo1_mouse4"]);

        // Clearing removes the device's rebinds, and the action once nothing is left.
        set_device_rebinds(&mut rebind_file, "m", "a", "keyboard", &[]);
        assert_eq!(read_rebinds(&rebind_file)[&("m".into(), "a".into())], ["js1_button1"]);
        set_device_rebinds(&mut rebind_file, "m", "b", "mouse", &[]);
        let xml = xml_doc::to_string(&rebind_file).unwrap();
        assert!(!xml.contains("name=\"b\""), "{xml}");

        // Clearing an action the file doesn't have adds nothing.
        set_device_rebinds(&mut rebind_file, "m", "c", "keyboard", &[]);
        assert!(!xml_doc::to_string(&rebind_file).unwrap().contains("name=\"c\""));
    }

    #[test]
    fn blank_layout_rebinds_read_and_edit_by_their_device() {
        const BLANK: &str = r#"<ActionMaps><actionmap name="m1">
                <action name="a">
                    <rebind device="gamepad" input=" "/>
                    <rebind device="keyboard" activationMode="press" input=" "/>
                    <rebind device="joystick" input="js1_ "/>
                </action>
            </actionmap></ActionMaps>"#;
        let mut rebind_file = xml_doc::from_str(BLANK).unwrap();
        // Unprefixed blank inputs read as their device's empty rebind, and are all kept.
        assert_eq!(read_rebinds(&rebind_file)[&("m1".into(), "a".into())], ["gp1_ ", "kb1_ ", "js1_ "]);

        // Editing the keyboard cell replaces the blank keyboard rebind in its place.
        set_device_rebinds(&mut rebind_file, "m1", "a", "keyboard", &["kb1_f".into()]);
        assert_eq!(read_rebinds(&rebind_file)[&("m1".into(), "a".into())], ["gp1_ ", "kb1_f", "js1_ "]);
        assert_eq!(xml_doc::to_string(&rebind_file).unwrap().matches("<rebind").count(), 3);

        // Autofill fills a blank keyboard rebind too, keeping its other attributes.
        let mut rebind_file = xml_doc::from_str(BLANK).unwrap();
        set_keyboard_rebind(&mut rebind_file, "m1", "a", "kb1_g");
        assert_eq!(read_rebinds(&rebind_file)[&("m1".into(), "a".into())], ["gp1_ ", "kb1_g", "js1_ "]);
        let xml = xml_doc::to_string(&rebind_file).unwrap();
        assert!(xml.contains(r#"activationMode="press" input="kb1_g""#), "{xml}");
    }
}
