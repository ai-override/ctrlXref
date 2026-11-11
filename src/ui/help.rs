//! Help pages: one Markdown document per menu, each shown in a window of its own.
//!
//! The pages are the `docs/help/*.md` files, built into the binary so they are there
//! wherever the executable is copied.

use eframe::egui::{self, Margin, ScrollArea, Ui};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

/// A help page, one per menu that has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HelpTopic {
    File,
    Source,
    Tools,
}

impl HelpTopic {
    pub const ALL: [HelpTopic; 3] = [Self::File, Self::Source, Self::Tools];

    /// The window title.
    pub fn title(self) -> &'static str {
        match self {
            Self::File => "ctrlXref Help - File Menu",
            Self::Source => "ctrlXref Help - Source Menu",
            Self::Tools => "ctrlXref Help - Tools Menu",
        }
    }

    /// The page's Markdown source.
    fn markdown(self) -> &'static str {
        match self {
            Self::File => include_str!("../../docs/help/file.md"),
            Self::Source => include_str!("../../docs/help/source.md"),
            Self::Tools => include_str!("../../docs/help/tools.md"),
        }
    }

    /// The id of the window that shows this page.
    pub fn viewport_id(self) -> egui::ViewportId {
        egui::ViewportId::from_hash_of(("help", self))
    }
}

/// The screenshots the pages show, built into the binary like the pages, by the path the pages
/// link them with. The paths are relative to `docs/help`, so GitHub shows them too.
const IMAGES: [(&str, &[u8]); 2] = [
    ("../../assets/images/orphaned_resolve1.png", include_bytes!("../../assets/images/orphaned_resolve1.png")),
    ("../../assets/images/orphaned_resolve2.png", include_bytes!("../../assets/images/orphaned_resolve2.png")),
];

/// Makes the pages' screenshots load from the binary. The viewer asks for a path without a
/// scheme as a `file://` URI, which these are kept under; no file is read.
pub fn install_images(ctx: &egui::Context) {
    egui_extras::install_image_loaders(ctx);
    for (path, bytes) in IMAGES {
        ctx.include_bytes(format!("file://{path}"), bytes);
    }
}

/// Draws the help page `topic` filling `ui`, the root of its window.
pub fn show(ui: &mut Ui, topic: HelpTopic, cache: &mut CommonMarkCache) {
    let frame = egui::Frame::central_panel(ui.style()).inner_margin(Margin::symmetric(24, 16));
    egui::CentralPanel::default_margins().frame(frame).show(ui, |ui| {
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            CommonMarkViewer::new().show(ui, cache, topic.markdown());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The image paths that `markdown` links, as in `![alt](path)`.
    fn linked_images(markdown: &str) -> Vec<&str> {
        markdown
            .match_indices("![")
            .filter_map(|(at, _)| {
                let rest = &markdown[at..];
                let start = rest.find("](")? + 2;
                Some(&rest[start..start + rest[start..].find(')')?])
            })
            .collect()
    }

    #[test]
    fn every_linked_image_is_built_in() {
        let built_in: Vec<&str> = IMAGES.iter().map(|(path, _)| *path).collect();
        for topic in HelpTopic::ALL {
            for path in linked_images(topic.markdown()) {
                assert!(built_in.contains(&path), "{path} of {topic:?} is not in IMAGES");
            }
        }
        assert_eq!(linked_images(HelpTopic::Tools.markdown()).len(), IMAGES.len());
    }
}
