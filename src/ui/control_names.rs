//! The window of Tools > Show Control Names: a sortable table of the game's control names.

use ctrlxref::control_names::ControlName;
use eframe::egui;

use super::attribute_table::{AttributeTable, TableRow};

/// The window title.
pub const TITLE: &str = "ctrlXref - Control Names";

/// The size of the window's content when it opens.
pub const SIZE: egui::Vec2 = egui::vec2(900., 720.);

/// The table's columns: the file's attributes, named like them, the device, and an example of
/// the control as written in the actions table's device columns.
const COLUMNS: [&str; 5] = ["name", "localizationString", "shortLocString", "Device", "EXAMPLE"];

/// The id of the window.
pub fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("control names")
}

pub struct ControlNamesWindow {
    table: AttributeTable,
    /// Index in `names` of the selected row, whatever the sort.
    selected: Option<usize>,
    count: usize,
    /// Where the window opens. Kept from then, so it stays where the user moves it.
    pub position: Option<egui::Pos2>,
}

impl ControlNamesWindow {
    pub fn new(names: &[ControlName], position: Option<egui::Pos2>) -> Self {
        let rows: Vec<TableRow> = names
            .iter()
            .map(|name| TableRow {
                tag: "Key".into(),
                cells: vec![
                    (COLUMNS[0].into(), name.name.clone()),
                    (COLUMNS[1].into(), name.localization.clone()),
                    (COLUMNS[2].into(), name.short.clone()),
                    (COLUMNS[3].into(), name.device.clone()),
                    (COLUMNS[4].into(), name.example()),
                ],
                styles: Vec::new(),
            })
            .collect();
        let mut table = AttributeTable::new();
        table.set_rows(&COLUMNS, &rows);
        Self { table, selected: None, count: names.len(), position }
    }

    /// Draws the window's content filling `ui`, the root of its window.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default_margins().show(ui, |ui| {
            super::app::panel_header(ui, format!("Control Names  -  {} rows", self.count));
            let selected = self.selected.and_then(|source_ix| self.table.row_of(source_ix));
            self.table.shortcuts(ui.ctx(), selected);
            let out = self.table.show(ui, selected, None);
            if let Some(row_ix) = out.clicked_row {
                self.selected = self.table.source_of(row_ix);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> ControlNamesWindow {
        let name = |name: &str, localization: &str, short: &str, device: &str| ControlName {
            name: name.into(),
            localization: localization.into(),
            short: short.into(),
            device: device.into(),
        };
        ControlNamesWindow::new(&[name("b", "Bravo", "", "mouse"), name("a", "Alpha", "A", "joystick")], None)
    }

    #[test]
    fn the_columns_are_name_localization_string_short_string_device_and_example() {
        let window = window();
        assert_eq!(window.table.headers(), COLUMNS);
        let rows: Vec<Vec<String>> = window.table.document_rows().map(|(_, cells)| cells.to_vec()).collect();
        assert_eq!(rows, [["b", "Bravo", "", "mouse", "mo1_b"], ["a", "Alpha", "A", "joystick", "js1_a"]]);
        assert_eq!(window.count, 2);
    }
}
