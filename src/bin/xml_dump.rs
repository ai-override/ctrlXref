use std::path::PathBuf;

use anyhow::{Result, bail};
use ctrlxref::xml_doc;
use xmltree::XMLNode;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output) = match args.as_slice() {
        [input] => (PathBuf::from(input), PathBuf::from(input).with_extension("out.xml")),
        [input, output] => (PathBuf::from(input), PathBuf::from(output)),
        _ => bail!("usage: xml_dump <input.xml> [output.xml]"),
    };

    let mut root = xml_doc::load(&input)?;
    let child_count = root.children.iter().filter_map(XMLNode::as_element).count();
    println!(
        "Loaded {}: root <{}> with {} attribute(s) and {} child element(s)",
        input.display(),
        root.name,
        root.attributes.len(),
        child_count
    );

    root.attributes.insert("modified".into(), "true".into());


    print_children(&root.children, 1);


    //xml_doc::save(&root, &output)?;
    println!("Output (not saved) {}", output.display());

    //exit with Ok result
    Ok(())
}

/// Prints every element in `children` and, recursively, all of its descendants.
/// `depth` starts at 1 for the root's direct children; each level is indented two tabs further.
fn print_children(children: &[XMLNode], depth: usize) {
    let indent = "\t".repeat((depth - 1) * 2);
    for element in children.iter().filter_map(XMLNode::as_element) {
        println!("{indent}{depth} element: {}", element.name);
        for (key, value) in &element.attributes {
            println!("{indent}\t{depth}.1 attribute: \"{key}\"=\"{value}\"");
        }
        if let Some(text) = element.get_text() {
            println!("{indent}\t{depth}.2 text: \"{}\"", text.trim());
        }
        print_children(&element.children, depth + 1);
    }
}

// D:\Devel\rust_projects\ctrlXref>cargo run --bin xml_dump -- xml\defaultProfile.xml
