//! Loading and saving XML documents as an editable element tree.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};
use xmltree::{Element, EmitterConfig};

/// Parses the XML file at `path` into its root element.
pub fn load(path: &Path) -> Result<Element> {
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    Element::parse(BufReader::new(file))
        .with_context(|| format!("cannot parse XML in {}", path.display()))
}

/// Writes `root` to `path` as indented UTF-8 XML, replacing any existing file.
pub fn save(root: &Element, path: &Path) -> Result<()> {
    let file = File::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    root.write_with_config(&mut writer, emitter_config())
        .with_context(|| format!("cannot write XML to {}", path.display()))?;
    writer.flush()?;
    Ok(())
}

/// Parses XML from a string.
pub fn from_str(xml: &str) -> Result<Element> {
    Element::parse(xml.as_bytes()).context("cannot parse XML")
}

/// Serializes `root` to an indented XML string.
pub fn to_string(root: &Element) -> Result<String> {
    let mut buf = Vec::new();
    root.write_with_config(&mut buf, emitter_config())
        .context("cannot serialize XML")?;
    Ok(String::from_utf8(buf)?)
}

/// Iterates over the child elements of `element`, skipping text, comments and other nodes.
pub fn child_elements(element: &Element) -> impl Iterator<Item = &Element> {
    element.children.iter().filter_map(|node| node.as_element())
}

/// Finds the descendant of `root` reached by following `path`, where each entry is
/// an index into [`child_elements`]. An empty path returns `root` itself.
pub fn element_at<'a>(root: &'a Element, path: &[usize]) -> Option<&'a Element> {
    path.iter()
        .try_fold(root, |element, &ix| child_elements(element).nth(ix))
}

fn emitter_config() -> EmitterConfig {
    EmitterConfig::new()
        .perform_indent(true)
        .write_document_declaration(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xmltree::XMLNode;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<config version="1"><item id="a">first</item></config>"#;

    #[test]
    fn round_trip_in_memory() {
        let mut root = from_str(SAMPLE).unwrap();
        root.attributes.insert("version".into(), "2".into());
        let mut child = Element::new("item");
        child.attributes.insert("id".into(), "b".into());
        child.children.push(XMLNode::Text("second".into()));
        root.children.push(XMLNode::Element(child));

        let reparsed = from_str(&to_string(&root).unwrap()).unwrap();
        assert_eq!(reparsed.attributes["version"], "2");
        let items: Vec<_> = reparsed.children.iter().filter_map(XMLNode::as_element).collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].get_text().unwrap(), "second");
    }

    #[test]
    fn element_at_follows_element_indices() {
        let root = from_str("<a>text<b/><!-- c --><c><d x=\"1\"/></c></a>").unwrap();
        assert_eq!(element_at(&root, &[]).unwrap().name, "a");
        assert_eq!(element_at(&root, &[1]).unwrap().name, "c");
        assert_eq!(element_at(&root, &[1, 0]).unwrap().attributes["x"], "1");
        assert!(element_at(&root, &[2]).is_none());
    }

    #[test]
    fn save_and_load_file() {
        let path = std::env::temp_dir().join("ctrlxref_save_and_load_test.xml");
        let root = from_str(SAMPLE).unwrap();
        save(&root, &path).unwrap();
        let loaded = load(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(loaded, root);
    }
}
