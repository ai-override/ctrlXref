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

/// Whether `name` is a mouse control, which the game's lists also give for the keyboard: a
/// `mouse1`, `maxis_z` or `mwheel_up`. A combination like `lalt+mouse2` is not one.
pub fn is_mouse_control(name: &str) -> bool {
    ["mouse", "maxis", "mwheel"].iter().any(|start| name.starts_with(start))
}

/// Whether keyboard binding `input`, from the profile or a rebind, ends in a key of the keyboard.
/// In `ralt+mouse2` or `kb1_lalt+js1_button3` the key is only a modifier of another device's
/// control, so those don't; nor do a blank input and an empty rebind like `kb1_ `.
fn binds_a_key(input: &str) -> bool {
    if input.is_empty() || is_empty_rebind(input) {
        return false;
    }
    let control = input.rsplit('+').next().unwrap_or(input).trim();
    let prefixed = control.split_once('_').filter(|(device, _)| {
        rebind_device(device).is_some() && device.len() > 2 && device[2..].bytes().all(|byte| byte.is_ascii_digit())
    });
    match prefixed {
        Some((device, name)) => rebind_device(device) == Some(DEVICES[0]) && !is_mouse_control(name),
        None => !is_mouse_control(control),
    }
}

/// Whether the action at `path` has no key bound on the keyboard. Its keyboard rebinds replace
/// the profile's binding; see [`binds_a_key`] for what binds one.
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
    !value.split(',').map(str::trim).any(binds_a_key)
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

/// Removes the empty `device` rebinds, like `kb1_ ` or `js2_`, of `actions` (`(actionmap name,
/// action name)`) from a rebind file whose inputs are `rebinds`, keeping the device's other
/// rebinds. The game's control editor leaves these behind. Returns how many there were.
pub fn clear_empty_device_rebinds(
    root: &mut Element,
    rebinds: &Rebinds,
    actions: &[(String, String)],
    device: &str,
) -> usize {
    let mut cleared = 0;
    for key in actions {
        let Some(inputs) = rebinds.get(key) else {
            continue;
        };
        let of_device: Vec<&String> = inputs.iter().filter(|input| rebind_device(input) == Some(device)).collect();
        let kept: Vec<String> = of_device.iter().filter(|input| !is_empty_rebind(input)).map(|input| input.to_string()).collect();
        if kept.len() < of_device.len() {
            set_device_rebinds(root, &key.0, &key.1, device, &kept);
            cleared += of_device.len() - kept.len();
        }
    }
    cleared
}

/// Whether the whole of rebind `input` matches `pattern`, where `*` stands for any text, none
/// included, and case doesn't matter: `*button*` matches `js4_button24`, and `*_lalt` matches
/// `kb1_lalt` but not `kb1_lalt+i`. The input's surrounding spaces are ignored, so `kb1_`
/// matches the empty rebind `kb1_ `. A pattern without `*` must match the input exactly.
pub fn wildcard_match(pattern: &str, input: &str) -> bool {
    let pattern = pattern.trim().to_lowercase();
    let input = input.trim().to_lowercase();
    let parts: Vec<&str> = pattern.split('*').collect();
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if parts.len() == 1 {
        return input == first;
    }
    if input.len() < first.len() + last.len() || !input.starts_with(first) || !input.ends_with(last) {
        return false;
    }
    // The middle parts, in order, in what is between the first and the last part.
    let mut rest = &input[first.len()..input.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// The `device` rebinds of `actions` (`(actionmap name, action name)`) in `rebinds` that
/// [`wildcard_match`] `pattern`, as `(action key, input)`.
pub fn matching_rebinds<'a>(
    rebinds: &'a Rebinds,
    actions: &'a [(String, String)],
    device: &'a str,
    pattern: &'a str,
) -> impl Iterator<Item = (&'a (String, String), &'a String)> {
    actions.iter().flat_map(move |key| {
        rebinds
            .get(key)
            .into_iter()
            .flatten()
            .filter(move |input| rebind_device(input) == Some(device) && wildcard_match(pattern, input))
            .map(move |input| (key, input))
    })
}

/// Removes the `device` rebinds of `actions` that [`wildcard_match`] `pattern` from a rebind
/// file whose inputs are `rebinds`, keeping the device's others. Returns how many there were.
pub fn remove_matching_rebinds(
    root: &mut Element,
    rebinds: &Rebinds,
    actions: &[(String, String)],
    device: &str,
    pattern: &str,
) -> usize {
    let mut removed = 0;
    for key in actions {
        let Some(inputs) = rebinds.get(key) else {
            continue;
        };
        let of_device: Vec<&String> = inputs.iter().filter(|input| rebind_device(input) == Some(device)).collect();
        let kept: Vec<String> =
            of_device.iter().filter(|input| !wildcard_match(pattern, input)).map(|input| input.to_string()).collect();
        if kept.len() < of_device.len() {
            set_device_rebinds(root, &key.0, &key.1, device, &kept);
            removed += of_device.len() - kept.len();
        }
    }
    removed
}

/// Copies the profile's default `device` bindings of the actions at `rows` (paths in `profile`)
/// into a rebind file whose inputs are `rebinds`, as `device` rebinds with the device prefix,
/// like `kb1_w`. A joystick's defaults don't say which stick they are for, so they go to
/// stick `instance` (`js2_` for 2), or to `js1_` without one. An action that already has a
/// `device` rebind, an empty one like `kb1_ ` and another stick's included, keeps it; so does
/// one without a default for `device`. Returns how many actions got their defaults.
pub fn clone_default_rebinds(
    root: &mut Element,
    profile: &Element,
    rows: &[Vec<usize>],
    rebinds: &Rebinds,
    device: &str,
    instance: Option<u32>,
) -> usize {
    let mut cloned = 0;
    for path in rows {
        let (Some(key), Some(action)) = (action_key(profile, path), xml_doc::element_at(profile, path)) else {
            continue;
        };
        if rebinds.get(&key).is_some_and(|inputs| inputs.iter().any(|input| rebind_device(input) == Some(device))) {
            continue;
        }
        let default = default_inputs(action, device);
        // A blank default binds nothing, and one for another device can't be a rebind of this one.
        match parse_cell_inputs(device, &default.join(","), &[]) {
            Ok(inputs) if !inputs.is_empty() => {
                let inputs: Vec<String> = match instance {
                    Some(instance) => inputs.iter().map(|input| with_instance(input, instance)).collect(),
                    None => inputs,
                };
                set_device_rebinds(root, &key.0, &key.1, device, &inputs);
                cloned += 1;
            }
            _ => {}
        }
    }
    cloned
}

/// The profile's default `device` inputs of `action`, for cloning into rebinds. The profile
/// gives some mouse controls as keyboard bindings, like `keyboard="mouse1"`: those are left
/// out of the keyboard's and added to the mouse's, after the mouse's own.
fn default_inputs(action: &Element, device: &str) -> Vec<String> {
    let split = |device: &str| -> Vec<String> {
        profile_input(action, device)
            .map(|value| value.split(',').map(|input| input.trim().to_string()).filter(|input| !input.is_empty()).collect())
            .unwrap_or_default()
    };
    match device {
        "keyboard" => split(device).into_iter().filter(|input| !ends_in_mouse_control(input)).collect(),
        "mouse" => {
            let mut inputs = split(device);
            inputs.extend(split(DEVICES[0]).into_iter().filter(|input| ends_in_mouse_control(input)));
            inputs
        }
        _ => split(device),
    }
}

/// The device instance of a prefixed rebind `input`: 2 for `js2_button1`.
fn instance_of(input: &str) -> Option<u32> {
    prefixed_device(input)?;
    input.split_once('_')?.0.get(2..)?.parse().ok()
}

/// The device instances that the `device` rebinds of `actions` use, sorted: `[1, 2]` for
/// `js1_x` and `js2_button3`.
pub fn device_instances(rebinds: &Rebinds, actions: &[(String, String)], device: &str) -> Vec<u32> {
    let mut instances: Vec<u32> = actions
        .iter()
        .filter_map(|key| rebinds.get(key))
        .flatten()
        .filter(|input| rebind_device(input) == Some(device))
        .filter_map(|input| instance_of(input))
        .collect();
    instances.sort_unstable();
    instances.dedup();
    instances
}

/// Moves the `device` rebinds of `actions` from device instance `from` to `to` in a rebind
/// file whose inputs are `rebinds`: `js1_button3` becomes `js2_button3` for 1 to 2. Each
/// `<rebind>` is changed in place, keeping its other attributes. A moved rebind that the action
/// already has on `to` is removed, so it is there once. Returns how many rebinds were moved.
pub fn change_device_instance(
    root: &mut Element,
    rebinds: &Rebinds,
    actions: &[(String, String)],
    device: &str,
    from: u32,
    to: u32,
) -> usize {
    if from == to {
        return 0;
    }
    let is_moved = |input: &str| rebind_device(input) == Some(device) && instance_of(input) == Some(from);
    let mut moved = 0;
    for key in actions {
        if !rebinds.get(key).is_some_and(|inputs| inputs.iter().any(|input| is_moved(input))) {
            continue;
        }
        let Some(map) = find_named_mut(root, ACTIONMAP_TAG, &key.0) else {
            continue;
        };
        let Some(action) = map.children.iter_mut().filter_map(XMLNode::as_mut_element).find(|child| is_named(child, "action", &key.1))
        else {
            continue;
        };
        for rebind in action.children.iter_mut().filter_map(XMLNode::as_mut_element).filter(|child| child.name == REBIND_TAG) {
            if let Some(input) = rebind.attributes.get_mut("input")
                && is_moved(input)
            {
                *input = with_instance(input, to);
                moved += 1;
            }
        }
        // Keep the first of rebinds that now have the same input.
        let mut seen: Vec<String> = Vec::new();
        action.children.retain(|node| {
            let Some(input) = node.as_element().filter(|child| child.name == REBIND_TAG).and_then(rebind_input) else {
                return true;
            };
            if rebind_device(&input) != Some(device) || is_empty_rebind(&input) {
                return true;
            }
            if seen.contains(&input) {
                return false;
            }
            seen.push(input);
            true
        });
    }
    moved
}

/// The prefixed rebind `input` with its device instance set to `instance`: `js1_x` for 3 is `js3_x`.
fn with_instance(input: &str, instance: u32) -> String {
    match input.split_once('_') {
        Some((head, rest)) if head.len() >= 2 => format!("{}{instance}_{rest}", &head[..2]),
        _ => input.to_string(),
    }
}

/// Whether `input` ends in a mouse control, like `mouse1`, `lalt+mouse2` or `kb1_mwheel_up`.
fn ends_in_mouse_control(input: &str) -> bool {
    let control = input.rsplit('+').next().unwrap_or(input).trim();
    let control = match prefixed_device(control) {
        Some(_) => control.split_once('_').map_or(control, |(_, name)| name),
        None => control,
    };
    is_mouse_control(control)
}

/// How the empty rebinds of a device column are written in its Clear menu item: `kb1_ `,
/// `mo1_ `, `jsx_ ` (any joystick) and `gp1_ `.
pub fn empty_rebind_label(device: &str) -> &'static str {
    match device {
        "keyboard" => "kb1_",
        "mouse" => "mo1_",
        "joystick" => "jsx_",
        "gamepad" => "gp1_",
        _ => "",
    }
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

/// Why a profile action is suggested for the rebinds of an action the profile lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchReason {
    /// The profile has an action of the same name in another actionmap: the action moved.
    SameName,
    /// The action names are this similar, in percent: the action was probably renamed.
    Similar(u8),
}

/// A profile action, `(actionmap name, action name)`, that the rebinds of an action the profile
/// lacks may belong to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub target: (String, String),
    pub reason: MatchReason,
}

/// How similar, in percent, action names must be for [`suggest_targets`] to suggest one.
const MIN_SIMILARITY: u8 = 70;

/// The keys of `rebinds` that are not among the profile's `actions`, sorted. The game ignores
/// their rebinds, which usually name an action renamed or moved since the file was made.
pub fn unmatched_actions(rebinds: &Rebinds, actions: &[(String, String)]) -> Vec<(String, String)> {
    let known: HashSet<&(String, String)> = actions.iter().collect();
    let mut unmatched: Vec<(String, String)> = rebinds.keys().filter(|key| !known.contains(key)).cloned().collect();
    unmatched.sort();
    unmatched
}

/// `name` in lower case without underscores, as names are compared.
fn normalized(name: &str) -> Vec<char> {
    name.chars().filter(|c| *c != '_').flat_map(char::to_lowercase).collect()
}

/// How many characters must be inserted, removed or replaced to turn `a` into `b`.
fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != cb)).min(above + 1).min(row[j] + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

/// How similar two [`normalized`] names are, in percent, or `None` when their lengths alone
/// make them less than `at_least` percent similar, which spares comparing them.
fn percent_similar(a: &[char], b: &[char], at_least: u8) -> Option<u8> {
    let longest = a.len().max(b.len());
    if longest == 0 {
        return Some(100);
    }
    if (a.len().min(b.len())) * 100 < usize::from(at_least) * longest {
        return None;
    }
    let same = longest - edit_distance(a, b);
    Some(u8::try_from(same * 100 / longest).unwrap_or(100))
}

/// How similar two names are, in percent, ignoring case and underscores.
fn similarity(a: &str, b: &str) -> u8 {
    percent_similar(&normalized(a), &normalized(b), 0).unwrap_or(0)
}

/// At most `max` profile `actions` that the rebinds of `orphan`, an action the profile lacks, may
/// belong to, the likeliest first: an action of the same name in another actionmap (it moved),
/// the one in the most similar actionmap first; then actions with a name at least
/// [`MIN_SIMILARITY`] percent similar (it was renamed), the same actionmap's first when as similar.
pub fn suggest_targets(orphan: &(String, String), actions: &[(String, String)], max: usize) -> Vec<Suggestion> {
    let (map, action) = orphan;
    let name = normalized(action);
    // Sorted by (same name, name similarity, same actionmap, actionmap similarity), best first.
    let mut scored: Vec<((bool, u8, bool, u8), Suggestion)> = actions
        .iter()
        .filter(|target| *target != orphan)
        .filter_map(|target| {
            let same_name = target.1 == *action;
            let name_score = match same_name {
                true => 100,
                false => percent_similar(&name, &normalized(&target.1), MIN_SIMILARITY)?,
            };
            if name_score < MIN_SIMILARITY {
                return None;
            }
            let reason = if same_name { MatchReason::SameName } else { MatchReason::Similar(name_score) };
            let key = (same_name, name_score, target.0 == *map, similarity(map, &target.0));
            Some((key, Suggestion { target: target.clone(), reason }))
        })
        .collect();
    // A stable sort keeps the profile's order among equals.
    scored.sort_by_key(|(key, _)| std::cmp::Reverse(*key));
    scored.into_iter().take(max).map(|(_, suggestion)| suggestion).collect()
}

/// Takes the `<action>` named `action` out of actionmap `map` in `element` or its descendants,
/// removing that actionmap too when it is left without actions.
fn take_action(element: &mut Element, map: &str, action: &str) -> Option<Element> {
    for ix in 0..element.children.len() {
        let Some(child) = element.children[ix].as_mut_element() else {
            continue;
        };
        if !is_named(child, ACTIONMAP_TAG, map) {
            if child.name != ACTIONMAP_TAG
                && let Some(taken) = take_action(child, map, action)
            {
                return Some(taken);
            }
            continue;
        }
        let Some(at) = child
            .children
            .iter()
            .position(|node| node.as_element().is_some_and(|a| is_named(a, "action", action)))
        else {
            continue;
        };
        let XMLNode::Element(taken) = child.children.remove(at) else {
            unreachable!("the position of an element");
        };
        if !xml_doc::child_elements(child).any(|a| a.name == "action") {
            element.children.remove(ix);
        }
        return Some(taken);
    }
    None
}

/// Removes the action `key` (`(actionmap name, action name)`) from a rebind file, and its
/// actionmap when that is left without actions. Returns how many rebinds it had.
pub fn remove_action(root: &mut Element, key: &(String, String)) -> usize {
    take_action(root, &key.0, &key.1)
        .map_or(0, |action| xml_doc::child_elements(&action).filter(|child| child.name == REBIND_TAG).count())
}

/// Removes from a rebind file every action not among the profile's `actions`, and the
/// actionmaps left without actions. Returns how many actions were removed.
pub fn remove_unmatched(root: &mut Element, actions: &[(String, String)]) -> usize {
    let keep: HashSet<(&str, &str)> = actions.iter().map(|(map, action)| (map.as_str(), action.as_str())).collect();
    remove_actions_not_in(root, &keep)
}

/// Moves the rebinds of action `from` to action `to` (both `(actionmap name, action name)`) in a
/// rebind file, keeping their other attributes, and removes `from` (and its actionmap when that
/// is left without actions). A rebind fills an empty rebind of its device that `to` has, like
/// `kb1_ `. Inputs `to` already has are left out, and so is an empty rebind of a device `to` already
/// rebinds. Creates `to` and its actionmap when needed. Returns how many rebinds were moved.
pub fn move_action_rebinds(root: &mut Element, from: &(String, String), to: &(String, String)) -> usize {
    let Some(source) = take_action(root, &from.0, &from.1) else {
        return 0;
    };
    let mut have: Vec<String> = read_rebinds(root).remove(to).unwrap_or_default();
    let rebinds: Vec<(Element, String)> = xml_doc::child_elements(&source)
        .filter(|child| child.name == REBIND_TAG)
        .filter_map(|rebind| Some((rebind.clone(), rebind_input(rebind)?)))
        .filter(|(_, input)| !have.contains(input))
        .collect();
    let mut moved = 0;
    for (rebind, input) in rebinds {
        let device = rebind_device(&input);
        if is_empty_rebind(&input) && have.iter().any(|old| rebind_device(old) == device) {
            continue;
        }
        let target = action_mut(root, &to.0, &to.1);
        let empty = target.children.iter().position(|node| {
            node.as_element().is_some_and(|child| {
                child.name == REBIND_TAG
                    && rebind_input(child).is_some_and(|old| is_empty_rebind(&old) && rebind_device(&old) == device)
            })
        });
        match empty {
            Some(ix) => {
                have.retain(|old| !(is_empty_rebind(old) && rebind_device(old) == device));
                target.children[ix] = XMLNode::Element(rebind);
            }
            None => target.children.push(XMLNode::Element(rebind)),
        }
        have.push(input);
        moved += 1;
    }
    moved
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

/// The rebind file's header, whose `label` names the file in the game's list of control profiles.
pub const HEADER_TAG: &str = "CustomisationUIHeader";

/// The most characters a header label can have, so it fits the game's list of control profiles.
pub const MAX_HEADER_LABEL: usize = 32;

/// The first [`HEADER_TAG`] element under `root`, in document order, outside the actionmaps.
/// Exported layouts keep it directly under the root; files in the game's profile format
/// (`<ActionMaps><ActionProfiles>`) usually have none.
fn find_header(root: &Element) -> Option<&Element> {
    xml_doc::child_elements(root).filter(|child| child.name != ACTIONMAP_TAG).find_map(|child| {
        if child.name == HEADER_TAG { Some(child) } else { find_header(child) }
    })
}

fn find_header_mut(root: &mut Element) -> Option<&mut Element> {
    root.children
        .iter_mut()
        .filter_map(XMLNode::as_mut_element)
        .filter(|child| child.name != ACTIONMAP_TAG)
        .find_map(|child| if child.name == HEADER_TAG { Some(child) } else { find_header_mut(child) })
}

/// The `label` of the rebind file's [`HEADER_TAG`], if it has one.
pub fn header_label(root: &Element) -> Option<&str> {
    find_header(root).and_then(|header| header.attributes.get("label")).map(String::as_str)
}

/// Sets the `label` of the rebind file's [`HEADER_TAG`], keeping its other attributes and
/// children. Without a header, one with only the label is added as the root's first child,
/// where the game's exported layouts have it.
pub fn set_header_label(root: &mut Element, label: &str) {
    if let Some(header) = find_header_mut(root) {
        header.attributes.insert("label".into(), label.into());
        return;
    }
    let mut header = Element::new(HEADER_TAG);
    header.attributes.insert("label".into(), label.into());
    root.children.insert(0, XMLNode::Element(header));
}

/// Why `label` cannot be a header label, or `None` when it can: it must not be blank and must
/// have at most [`MAX_HEADER_LABEL`] characters, leading and trailing spaces left out.
pub fn header_label_error(label: &str) -> Option<&'static str> {
    let label = label.trim();
    if label.is_empty() {
        Some("The label can't be empty")
    } else if label.chars().count() > MAX_HEADER_LABEL {
        Some("The label can be at most 32 characters")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_label_is_read_and_set_keeping_the_rest() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><CustomisationUIHeader label="Old" description="d" image=""><devices><keyboard instance="1"/></devices></CustomisationUIHeader><modifiers/><actionmap name="m"/></ActionMaps>"#,
        )
        .unwrap();
        assert_eq!(header_label(&root), Some("Old"));
        set_header_label(&mut root, "New name");
        assert_eq!(header_label(&root), Some("New name"));
        let children: Vec<_> = xml_doc::child_elements(&root).map(|child| child.name.as_str()).collect();
        assert_eq!(children, [HEADER_TAG, "modifiers", "actionmap"]);
        let header = xml_doc::child_elements(&root).next().unwrap();
        assert_eq!(header.attributes["description"], "d");
        assert_eq!(xml_doc::child_elements(header).next().unwrap().name, "devices");
    }

    #[test]
    fn header_label_is_found_inside_action_profiles() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><ActionProfiles profileName="default"><CustomisationUIHeader label="Game"/><actionmap name="m"/></ActionProfiles></ActionMaps>"#,
        )
        .unwrap();
        assert_eq!(header_label(&root), Some("Game"));
        set_header_label(&mut root, "Copy");
        assert_eq!(header_label(&root), Some("Copy"));
        let xml = xml_doc::to_string(&root).unwrap();
        assert_eq!(xml.matches(HEADER_TAG).count(), 1, "{xml}");
    }

    #[test]
    fn header_is_added_under_the_root_of_a_profile_format_file() {
        let mut root =
            xml_doc::from_str(r#"<ActionMaps><ActionProfiles profileName="default"><actionmap name="m"/></ActionProfiles></ActionMaps>"#)
                .unwrap();
        assert_eq!(header_label(&root), None);
        set_header_label(&mut root, "Copy");
        let children: Vec<_> = xml_doc::child_elements(&root).map(|child| child.name.as_str()).collect();
        assert_eq!(children, [HEADER_TAG, "ActionProfiles"]);
        assert_eq!(header_label(&root), Some("Copy"));
    }

    #[test]
    fn label_is_added_to_a_header_without_one() {
        let mut root =
            xml_doc::from_str(r#"<ActionMaps><CustomisationUIHeader description="d"/><actionmap name="m"/></ActionMaps>"#).unwrap();
        assert_eq!(header_label(&root), None);
        set_header_label(&mut root, "Copy");
        assert_eq!(header_label(&root), Some("Copy"));
        assert_eq!(xml_doc::child_elements(&root).filter(|child| child.name == HEADER_TAG).count(), 1);
        assert_eq!(xml_doc::child_elements(&root).next().unwrap().attributes["description"], "d");
    }

    #[test]
    fn header_is_added_when_missing() {
        let mut root = xml_doc::from_str(r#"<ActionMaps><actionmap name="m"/></ActionMaps>"#).unwrap();
        assert_eq!(header_label(&root), None);
        set_header_label(&mut root, "Copy");
        assert_eq!(header_label(&root), Some("Copy"));
        assert_eq!(xml_doc::child_elements(&root).next().unwrap().name, HEADER_TAG);
        assert_eq!(xml_doc::child_elements(&root).count(), 2);
    }

    #[test]
    fn header_label_must_be_one_to_32_characters() {
        assert!(header_label_error("").is_some());
        assert!(header_label_error("   ").is_some());
        assert_eq!(header_label_error("a"), None);
        assert_eq!(header_label_error(&"a".repeat(32)), None);
        assert_eq!(header_label_error(&format!("  {}  ", "a".repeat(32))), None);
        assert!(header_label_error(&"a".repeat(33)).is_some());
        // Characters are counted, not bytes.
        assert_eq!(header_label_error(&"ä".repeat(32)), None);
        assert!(header_label_error(&"ä".repeat(33)).is_some());
    }

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
    fn autofill_targets_actions_with_only_mouse_controls_as_keyboard_default() {
        let profile = xml_doc::from_str(
            r#"<p><actionmap name="m">
                <action name="axis" keyboard="maxis_z"/>
                <action name="button" keyboard="mouse2"/>
                <action name="wheels" keyboard="mwheel_up, mwheel_down"/>
                <action name="combo" keyboard="ralt+mouse2"/>
                <action name="wheel_combo" keyboard="lalt+mwheel_up"/>
                <action name="mixed" keyboard="mouse1, f"/>
                <action name="key_combo" keyboard="lalt+k"/>
                <action name="numpad" keyboard="np_5"/>
                <action name="modifier" keyboard="lshift"/>
                <action name="rebound" keyboard="mouse1"/>
                <action name="rebound_to_mouse" keyboard="f"/>
                <action name="rebound_to_mouse_combo" keyboard="f"/>
                <action name="rebound_to_joystick_combo" keyboard="f"/>
                <action name="rebound_to_gamepad_combo" keyboard="f"/>
                <action name="joystick_rebind_only" keyboard=" "/>
            </actionmap></p>"#,
        )
        .unwrap();
        let rebind_file = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="rebound"><rebind input="kb1_g"/></action>
                <action name="rebound_to_mouse"><rebind input="kb1_mouse3"/></action>
                <action name="rebound_to_mouse_combo"><rebind input="kb1_lctrl+mouse3"/></action>
                <action name="rebound_to_joystick_combo"><rebind input="kb1_lalt+js1_button3"/></action>
                <action name="rebound_to_gamepad_combo"><rebind input="kb1_lalt+gp1_a"/></action>
                <action name="joystick_rebind_only"><rebind input="js1_lalt+button3"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let unbound = unbound_keyboard_actions(&profile, &rows, &read_rebinds(&rebind_file));
        let expected = [
            "axis",
            "button",
            "wheels",
            "combo",
            "wheel_combo",
            "rebound_to_mouse",
            "rebound_to_mouse_combo",
            "rebound_to_joystick_combo",
            "rebound_to_gamepad_combo",
            "joystick_rebind_only",
        ]
        .map(|name| ("m".to_string(), name.to_string()));
        assert_eq!(unbound, expected);
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
    fn clearing_empty_rebinds_keeps_real_ones_and_other_devices() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="kb1_ "/><rebind input="kb1_w" activationMode="hold"/><rebind input="js1_"/></action><action name="b"><rebind input="kb1_ "/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let have = read_rebinds(&root);
        let actions = [("m".to_string(), "a".to_string()), ("m".to_string(), "b".to_string())];
        assert_eq!(clear_empty_device_rebinds(&mut root, &have, &actions, "keyboard"), 2);
        let after = read_rebinds(&root);
        assert_eq!(after[&actions[0]], ["kb1_w", "js1_"]);
        // An action left with no rebinds is removed.
        assert!(!after.contains_key(&actions[1]));
        assert!(xml_doc::to_string(&root).unwrap().contains(r#"activationMode="hold""#));
        assert_eq!(clear_empty_device_rebinds(&mut root, &after, &actions, "keyboard"), 0);
        assert_eq!(empty_rebind_label("joystick"), "jsx_");
    }

    #[test]
    fn cloning_defaults_fills_only_actions_without_rebinds_of_the_device() {
        let profile = xml_doc::from_str(
            r#"<profile><actionmap name="m">
                <action name="a" keyboard="w" mouse="mouse1"/>
                <action name="b" keyboard="lalt+s"/>
                <action name="c" keyboard=" "/>
                <action name="d"><keyboard><inputdata input="x"/><inputdata input="y"/></keyboard></action>
                <action name="e" keyboard="e"/>
            </actionmap></profile>"#,
        )
        .unwrap();
        let rows: Vec<Vec<usize>> = (0..5).map(|ix| vec![0, ix]).collect();
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="mo1_mouse2"/></action><action name="b"><rebind input="kb1_q"/></action><action name="e"><rebind input="kb1_ "/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let have = read_rebinds(&root);
        // b keeps its own binding, e its empty one, and c has no default.
        assert_eq!(clone_default_rebinds(&mut root, &profile, &rows, &have, "keyboard", None), 2);
        let after = read_rebinds(&root);
        let key = |action: &str| ("m".to_string(), action.to_string());
        assert_eq!(after[&key("a")], ["mo1_mouse2", "kb1_w"]);
        assert_eq!(after[&key("b")], ["kb1_q"]);
        assert!(!after.contains_key(&key("c")));
        assert_eq!(after[&key("d")], ["kb1_x", "kb1_y"]);
        assert_eq!(after[&key("e")], ["kb1_ "]);
        // a already has a mouse rebind.
        assert_eq!(clone_default_rebinds(&mut root, &profile, &rows, &after, "mouse", None), 0);
    }

    #[test]
    fn changing_the_joystick_number_moves_rebinds_in_place() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m">
                <action name="a"><rebind input="js1_button3" activationMode="hold"/><rebind input="kb1_w"/><rebind input="js2_x"/></action>
                <action name="b"><rebind input="js1_x"/><rebind input="js2_x"/></action>
                <action name="c"><rebind input="js3_button1"/></action>
            </actionmap></ActionMaps>"#,
        )
        .unwrap();
        let have = read_rebinds(&root);
        let key = |action: &str| ("m".to_string(), action.to_string());
        let actions = [key("a"), key("b"), key("c")];
        assert_eq!(device_instances(&have, &actions, "joystick"), [1, 2, 3]);
        assert_eq!(change_device_instance(&mut root, &have, &actions, "joystick", 1, 2), 2);
        let after = read_rebinds(&root);
        assert_eq!(after[&key("a")], ["js2_button3", "kb1_w", "js2_x"]);
        // b had x on both sticks; it is there once.
        assert_eq!(after[&key("b")], ["js2_x"]);
        assert_eq!(after[&key("c")], ["js3_button1"]);
        assert!(xml_doc::to_string(&root).unwrap().contains(r#"<rebind input="js2_button3" activationMode="hold""#));
        assert_eq!(device_instances(&after, &actions, "joystick"), [2, 3]);
        assert_eq!(change_device_instance(&mut root, &after, &actions, "joystick", 3, 3), 0);
    }

    #[test]
    fn wildcards_match_the_whole_input() {
        assert!(wildcard_match("*button*", "js4_button24"));
        assert!(wildcard_match("*_lalt", "kb1_lalt"));
        assert!(!wildcard_match("*_lalt", "kb1_lalt+i"));
        assert!(wildcard_match("kb1_*+i", "kb1_lalt+i"));
        assert!(wildcard_match("js*_button*", "js2_button3"));
        assert!(!wildcard_match("js*_button*", "js2_x"));
        assert!(wildcard_match("*", "kb1_w"));
        assert!(wildcard_match("KB1_W", "kb1_w"));
        assert!(!wildcard_match("kb1_w", "kb1_wx"));
        assert!(wildcard_match("kb1_", "kb1_ "));
        // The first and last parts can't overlap: `a*a` needs two a's.
        assert!(!wildcard_match("a*a", "a"));
        assert!(wildcard_match("a*a", "aa"));
    }

    #[test]
    fn removing_by_wildcards_keeps_non_matching_and_other_devices() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="kb1_lalt"/><rebind input="kb1_lalt+i" multiTap="2"/><rebind input="mo1_lalt"/></action><action name="b"><rebind input="kb1_ralt"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let have = read_rebinds(&root);
        let actions = [("m".to_string(), "a".to_string()), ("m".to_string(), "b".to_string())];
        assert_eq!(matching_rebinds(&have, &actions, "keyboard", "*alt").count(), 2);
        assert_eq!(remove_matching_rebinds(&mut root, &have, &actions, "keyboard", "*alt"), 2);
        let after = read_rebinds(&root);
        assert_eq!(after[&actions[0]], ["kb1_lalt+i", "mo1_lalt"]);
        assert!(!after.contains_key(&actions[1]));
        assert!(xml_doc::to_string(&root).unwrap().contains(r#"multiTap="2""#));
    }

    #[test]
    fn cloning_joystick_defaults_goes_to_the_chosen_stick() {
        let profile = xml_doc::from_str(
            r#"<profile><actionmap name="m">
                <action name="a" joystick="button1"/>
                <action name="b" joystick="js1_x, lalt+button2"/>
                <action name="c" joystick="button3"/>
            </actionmap></profile>"#,
        )
        .unwrap();
        let rows = vec![vec![0, 0], vec![0, 1], vec![0, 2]];
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="c"><rebind input="js1_button9"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let have = read_rebinds(&root);
        // c has a joystick rebind of its own, on another stick.
        assert_eq!(clone_default_rebinds(&mut root, &profile, &rows, &have, "joystick", Some(2)), 2);
        let after = read_rebinds(&root);
        let key = |action: &str| ("m".to_string(), action.to_string());
        assert_eq!(after[&key("a")], ["js2_button1"]);
        assert_eq!(after[&key("b")], ["js2_x", "js2_lalt+button2"]);
        assert_eq!(after[&key("c")], ["js1_button9"]);
    }

    #[test]
    fn cloning_moves_keyboard_mouse_defaults_to_the_mouse() {
        let profile = xml_doc::from_str(
            r#"<profile><actionmap name="m">
                <action name="q" activationMode="delayed_press" keyboard="mouse1"/>
                <action name="r" keyboard="r, lalt+mouse2" mouse="mwheel_up"/>
            </actionmap></profile>"#,
        )
        .unwrap();
        let rows = vec![vec![0, 0], vec![0, 1]];
        let mut root = xml_doc::from_str("<ActionMaps/>").unwrap();
        assert_eq!(clone_default_rebinds(&mut root, &profile, &rows, &Rebinds::new(), "keyboard", None), 1);
        let have = read_rebinds(&root);
        assert_eq!(clone_default_rebinds(&mut root, &profile, &rows, &have, "mouse", None), 2);
        let after = read_rebinds(&root);
        let key = |action: &str| ("m".to_string(), action.to_string());
        assert_eq!(after[&key("q")], ["mo1_mouse1"]);
        assert_eq!(after[&key("r")], ["kb1_r", "mo1_mwheel_up", "mo1_lalt+mouse2"]);
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

    fn keys(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(map, action)| (map.to_string(), action.to_string())).collect()
    }

    #[test]
    fn unmatched_actions_are_the_rebinds_the_profile_lacks_sorted() {
        let root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="z"><rebind input="kb1_z"/></action><action name="a"><rebind input="kb1_a"/></action><action name="b"><rebind input="kb1_b"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let unmatched = unmatched_actions(&read_rebinds(&root), &keys(&[("m", "a")]));
        assert_eq!(unmatched, keys(&[("m", "b"), ("m", "z")]));
    }

    #[test]
    fn an_action_moved_to_another_actionmap_is_suggested_first() {
        let actions = keys(&[
            ("spaceship_general", "v_mining_mode_x"),
            ("other", "v_mining_mode"),
            ("spaceship_mining", "v_mining_mode"),
        ]);
        let suggestions = suggest_targets(&("spaceship_mining_old".into(), "v_mining_mode".into()), &actions, 3);
        let targets: Vec<_> = suggestions.iter().map(|s| (s.target.0.as_str(), s.reason)).collect();
        assert_eq!(targets[0], ("spaceship_mining", MatchReason::SameName));
        assert_eq!(targets[1], ("other", MatchReason::SameName));
        assert!(matches!(targets[2].1, MatchReason::Similar(_)));
    }

    #[test]
    fn a_renamed_action_is_suggested_with_its_similarity() {
        let actions = keys(&[("spaceship_general", "v_toggle_landing"), ("spaceship_general", "v_flight_ready")]);
        let suggestions = suggest_targets(&("spaceship_general".into(), "v_flightready".into()), &actions, 3);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].target.1, "v_flight_ready");
        assert_eq!(suggestions[0].reason, MatchReason::Similar(100));

        let suggestions = suggest_targets(&("spaceship_general".into(), "v_flight_readyy".into()), &actions, 3);
        assert!(matches!(suggestions[0].reason, MatchReason::Similar(percent) if (90..100).contains(&percent)));
    }

    #[test]
    fn an_unrelated_name_gets_no_suggestion() {
        let actions = keys(&[("m", "v_toggle_landing"), ("m", "v_flight_ready")]);
        assert!(suggest_targets(&("m".into(), "pc_interaction_mode".into()), &actions, 3).is_empty());
    }

    #[test]
    fn moving_rebinds_keeps_attributes_skips_duplicates_and_removes_the_source() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="old"><action name="gone"><rebind input="kb1_g" multiTap="2"/><rebind input="kb1_x"/><rebind input="js1_ "/></action></actionmap><actionmap name="new"><action name="here"><rebind input="kb1_x"/><rebind input="js1_button1"/><rebind input="gp1_ "/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let moved = move_action_rebinds(&mut root, &("old".into(), "gone".into()), &("new".into(), "here".into()));
        // kb1_x is there already, and the empty js1_ would unbind js1_button1.
        assert_eq!(moved, 1);
        let rebinds = read_rebinds(&root);
        assert_eq!(rebinds.len(), 1);
        assert_eq!(rebinds[&("new".into(), "here".into())], ["kb1_x", "js1_button1", "gp1_ ", "kb1_g"]);
        let xml = xml_doc::to_string(&root).unwrap();
        assert!(xml.contains(r#"input="kb1_g" multiTap="2""#) || xml.contains(r#"multiTap="2" input="kb1_g""#), "{xml}");
        assert!(!xml.contains("name=\"old\""), "{xml}");
    }

    #[test]
    fn a_moved_rebind_fills_an_empty_one_and_creates_the_target() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="gone"><rebind input="gp1_a"/></action><action name="blank"><rebind input="gp1_ "/></action><action name="keep"><rebind input="kb1_k"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        assert_eq!(move_action_rebinds(&mut root, &("m".into(), "gone".into()), &("m".into(), "blank".into())), 1);
        let rebinds = read_rebinds(&root);
        assert_eq!(rebinds[&("m".into(), "blank".into())], ["gp1_a"]);
        assert!(!rebinds.contains_key(&("m".into(), "gone".into())));

        assert_eq!(move_action_rebinds(&mut root, &("m".into(), "keep".into()), &("n".into(), "new".into())), 1);
        assert_eq!(read_rebinds(&root)[&("n".into(), "new".into())], ["kb1_k"]);
    }

    #[test]
    fn removing_unmatched_actions_keeps_the_matched_ones() {
        let mut root = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="kb1_a"/></action><action name="gone"><rebind input="kb1_g"/></action></actionmap><actionmap name="old"><action name="x"><rebind input="kb1_x"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let mut single = root.clone();
        assert_eq!(remove_action(&mut single, &("old".into(), "x".into())), 1);
        assert!(!xml_doc::to_string(&single).unwrap().contains("name=\"old\""));

        assert_eq!(remove_unmatched(&mut root, &keys(&[("m", "a")])), 2);
        let rebinds = read_rebinds(&root);
        assert_eq!(rebinds.keys().collect::<Vec<_>>(), [&("m".to_string(), "a".to_string())]);
        assert!(!xml_doc::to_string(&root).unwrap().contains("name=\"old\""));
    }
}
