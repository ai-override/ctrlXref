//! Table that lists XML elements as rows, one column per attribute name.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use eframe::egui::{self, Align, Color32, FontFamily, Key, Label, RichText, ScrollArea, Sense, TextEdit, Ui};
use egui_extras::TableBuilder;

use super::theme;

/// Column key for the element name; `#` cannot start an XML attribute name, so it never
/// clashes with attribute columns.
const ELEMENT_KEY: &str = "#element";

/// The input for one table row: an element's tag name and its cells as
/// `(column name, display value)`, usually its attributes.
pub struct TableRow {
    pub tag: String,
    pub cells: Vec<(String, String)>,
    /// `(column name, style)` of the cells drawn differently from plain text.
    pub styles: Vec<(String, CellStyle)>,
}

/// How a cell is drawn, when not as plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellStyle {
    /// The default profile's bindings (the read-only Default column): bold, light blue.
    Profile,
    /// A conflict: dark red background.
    Conflict,
}

/// Approximate width of one character in the table font, used to size columns to their content.
const CHAR_WIDTH: f32 = 7.5;

/// Column width limits; longer cells are clipped and the column can be resized by dragging.
const MIN_WIDTH: f32 = 48.;
const MAX_WIDTH: f32 = 280.;

/// Share of a column's non-empty cells that its width fits, so a few very long cells
/// (like an action binding five mouse inputs) do not widen the whole column.
const FIT_SHARE: f32 = 0.98;

/// Background of a cell that shows a conflict.
const CONFLICT_BG: Color32 = Color32::from_rgb(0x4a, 0x08, 0x08);

/// Text color of profile bindings in the dark and the light theme; the light theme
/// uses a deeper blue so the text stays readable on white.
const PROFILE_FG_DARK: Color32 = theme::ACCENT;
const PROFILE_FG_LIGHT: Color32 = Color32::from_rgb(0x1f, 0x6f, 0xd1);

/// A column: its key (the cell name in [`TableRow::cells`]), header text and initial width.
#[derive(Clone)]
struct Column {
    key: String,
    name: String,
    width: f32,
}

/// A column's sort order; clicking its header cycles through them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColumnSort {
    Default,
    Ascending,
    Descending,
}

#[derive(Clone)]
struct Row {
    /// Index of the row's element in the slice given to `set_rows`, so a row
    /// can be traced back to its element whatever the sort order.
    source_ix: usize,
    /// One cell per column.
    cells: Vec<String>,
    /// Each cell's style, `None` for plain text.
    styles: Vec<Option<CellStyle>>,
}

/// A cell's new text, entered in its editor, for the owner to apply with [`take_edits`].
///
/// [`take_edits`]: AttributeTable::take_edits
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellEdit {
    /// Index of the row's element in the last `set_rows` slice.
    pub source_ix: usize,
    /// The column's key, the cell name given in [`TableRow::cells`].
    pub column: String,
    pub value: String,
}

/// The cell being edited and its text.
struct Editing {
    source_ix: usize,
    col_ix: usize,
    /// The cell's text when editing started; committing it unchanged is no edit.
    original: String,
    text: String,
    /// Set until the editor has taken the keyboard focus.
    focus: bool,
}

/// What happened in the table this frame.
#[derive(Default)]
pub struct TableOutput {
    /// The displayed row the user clicked.
    pub clicked_row: Option<usize>,
    /// Whether a header click re-sorted the rows.
    pub resorted: bool,
    /// Where each column's header cell is.
    pub header_rects: Vec<egui::Rect>,
}

pub struct AttributeTable {
    columns: Vec<Column>,
    /// Rows in document order; `ColumnSort::Default` shows them like this.
    original: Vec<Row>,
    /// `original` reordered by the active sort.
    rows: Vec<Row>,
    sort: Option<(usize, ColumnSort)>,
    /// Keys of the columns whose cells can be edited with a double-click.
    editable: HashSet<String>,
    /// Keys of the columns whose titles keep their case in the dark theme's uppercase headers.
    exact_titles: HashSet<String>,
    editing: Option<Editing>,
    /// Committed edits not yet taken by the owner.
    edits: Vec<CellEdit>,
}

impl AttributeTable {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            original: Vec::new(),
            rows: Vec::new(),
            sort: None,
            editable: HashSet::new(),
            exact_titles: HashSet::new(),
            editing: None,
            edits: Vec::new(),
        }
    }

    /// Makes the cells of the columns with these keys editable with a double-click.
    pub fn set_editable(&mut self, columns: &[&str]) {
        self.editable = columns.iter().map(|name| name.to_string()).collect();
    }

    /// Makes the titles of the columns with these keys keep their case in the dark theme,
    /// whose headers are otherwise uppercase.
    pub fn set_exact_titles(&mut self, columns: &[&str]) {
        self.exact_titles = columns.iter().map(|name| name.to_string()).collect();
    }

    /// The edits committed since the last call, oldest first.
    pub fn take_edits(&mut self) -> Vec<CellEdit> {
        std::mem::take(&mut self.edits)
    }

    /// Whether the cell at displayed `row_ix`, `col_ix` can be edited: its column is
    /// editable and it is not a [`CellStyle::Profile`] binding, which comes from the game
    /// and is read-only.
    fn is_editable(&self, row_ix: usize, col_ix: usize) -> bool {
        self.columns.get(col_ix).is_some_and(|column| self.editable.contains(&column.key))
            && self.rows.get(row_ix).is_some_and(|row| row.styles[col_ix] != Some(CellStyle::Profile))
    }

    /// Opens a text editor over the cell at displayed `row_ix`, `col_ix`, holding its text.
    /// Enter or leaving the editor commits it, Escape cancels.
    fn start_edit(&mut self, row_ix: usize, col_ix: usize) {
        if !self.is_editable(row_ix, col_ix) {
            return;
        }
        let Some(row) = self.rows.get(row_ix) else {
            return;
        };
        let original = row.cells[col_ix].clone();
        self.editing = Some(Editing {
            source_ix: row.source_ix,
            col_ix,
            text: original.clone(),
            original,
            focus: true,
        });
    }

    /// Closes the cell editor, recording its text as an edit when `commit` and changed.
    fn finish_edit(&mut self, commit: bool) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        if commit && editing.text != editing.original {
            self.edits.push(CellEdit {
                source_ix: editing.source_ix,
                column: self.columns[editing.col_ix].key.clone(),
                value: editing.text,
            });
        }
    }

    /// Shows one row per [`TableRow`]. The columns are an "Element" column when the rows
    /// have different tags, the `leading` columns in that order (always present, even when
    /// no row has them), then the other cell names in first-seen order. Missing cells stay
    /// empty. Keeps the sort while the columns stay the same.
    pub fn set_rows(&mut self, leading: &[&str], rows: &[TableRow]) {
        let mixed_tags = rows.windows(2).any(|pair| pair[0].tag != pair[1].tag);

        let mut seen: HashSet<&str> = leading.iter().copied().collect();
        let others = rows
            .iter()
            .flat_map(|row| row.cells.iter().map(|(name, _)| name.as_str()))
            .filter(|name| seen.insert(*name));
        let names: Vec<&str> = leading.iter().copied().chain(others).collect();

        let mut headers: Vec<(String, String)> = Vec::new();
        if mixed_tags {
            headers.push((ELEMENT_KEY.into(), "Element".into()));
        }
        headers.extend(names.iter().map(|name| (name.to_string(), name.to_string())));

        self.original = rows
            .iter()
            .enumerate()
            .map(|(source_ix, row)| {
                let tag = mixed_tags.then(|| row.tag.clone());
                let value = |name: &str| {
                    row.cells
                        .iter()
                        .find(|(cell_name, _)| cell_name == name)
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default()
                };
                let cells = tag.into_iter().chain(names.iter().map(|name| value(name))).collect();
                let style = |name: &str| {
                    row.styles
                        .iter()
                        .find(|(cell_name, _)| cell_name == name)
                        .map(|(_, style)| *style)
                };
                let styles = mixed_tags
                    .then_some(None)
                    .into_iter()
                    .chain(names.iter().map(|name| style(name)))
                    .collect();
                Row { source_ix, cells, styles }
            })
            .collect();

        let same_columns = self.columns.len() == headers.len()
            && self.columns.iter().zip(&headers).all(|(column, (key, _))| column.key == *key);
        self.columns = headers
            .into_iter()
            .enumerate()
            .map(|(col_ix, (key, name))| {
                let lengths = self.original.iter().map(|row| row.cells[col_ix].chars().count());
                // +2 leaves room for the sort arrow next to the header text.
                let chars = typical_length(lengths).max(name.chars().count() + 2);
                let width = (chars as f32 * CHAR_WIDTH + 20.).clamp(MIN_WIDTH, MAX_WIDTH);
                Column { key, name, width }
            })
            .collect();

        if !same_columns {
            self.sort = None;
        }
        self.apply_sort();
        // The rows the editor was opened on are gone.
        self.editing = None;
    }

    /// The displayed row index of the element at `source_ix` in the last `set_rows` slice.
    pub fn row_of(&self, source_ix: usize) -> Option<usize> {
        self.rows.iter().position(|row| row.source_ix == source_ix)
    }

    /// The index in the last `set_rows` slice of the element displayed at `row_ix`.
    pub fn source_of(&self, row_ix: usize) -> Option<usize> {
        self.rows.get(row_ix).map(|row| row.source_ix)
    }

    /// How many rows are displayed.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The column headers, as shown.
    pub fn headers(&self) -> Vec<String> {
        self.columns.iter().map(|column| column.name.clone()).collect()
    }

    /// Each row's source index (into the last `set_rows` slice) and cells, in the current
    /// sort order.
    pub fn displayed_rows(&self) -> impl Iterator<Item = (usize, &[String])> {
        self.rows.iter().map(|row| (row.source_ix, row.cells.as_slice()))
    }

    fn apply_sort(&mut self) {
        self.rows = self.original.clone();
        let Some((col_ix, sort)) = self.sort else {
            return;
        };
        let cell = |row: &Row| row.cells[col_ix].clone();
        match sort {
            ColumnSort::Default => {}
            ColumnSort::Ascending => self.rows.sort_by(|a, b| compare_values(&cell(a), &cell(b))),
            ColumnSort::Descending => self.rows.sort_by(|a, b| compare_values(&cell(b), &cell(a))),
        }
    }

    /// Moves column `col_ix` to its next sort order: ascending, descending, document order.
    fn cycle_sort(&mut self, col_ix: usize) {
        let next = match self.sort {
            Some((ix, ColumnSort::Ascending)) if ix == col_ix => ColumnSort::Descending,
            Some((ix, ColumnSort::Descending)) if ix == col_ix => ColumnSort::Default,
            _ => ColumnSort::Ascending,
        };
        self.set_sort(col_ix, next);
    }

    /// Sorts the rows by column `col_ix` in order `sort`.
    fn set_sort(&mut self, col_ix: usize, sort: ColumnSort) {
        self.sort = Some((col_ix, sort));
        self.apply_sort();
    }

    /// Draws the table filling `ui`, with displayed row `selected` highlighted and, when
    /// given, row `scroll_to` scrolled into the middle of the view.
    pub fn show(&mut self, ui: &mut Ui, selected: Option<usize>, scroll_to: Option<usize>) -> TableOutput {
        let mut out = TableOutput::default();
        if self.columns.is_empty() {
            return out;
        }
        // Column widths are kept per set of columns.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for column in &self.columns {
            column.key.hash(&mut hasher);
        }
        let table_id = hasher.finish();
        let row_height = ui.text_style_height(&egui::TextStyle::Body) + 6.;

        // The table is wider than the window, so it scrolls sideways with its bar in sight.
        ScrollArea::horizontal()
            .auto_shrink(false)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                let mut builder = TableBuilder::new(ui)
                    .id_salt(table_id)
                    .striped(true)
                    .resizable(true)
                    .sense(Sense::click())
                    .auto_shrink(false)
                    .cell_layout(egui::Layout::left_to_right(Align::Center));
                for column in &self.columns {
                    builder = builder.column(
                        egui_extras::Column::initial(column.width).at_least(MIN_WIDTH / 2.).clip(true),
                    );
                }
                if let Some(row) = scroll_to {
                    builder = builder.scroll_to_row(row, Some(Align::Center));
                }
                let sort = self.sort;
                // A header click cycles that column's sort; its context menu picks one.
                let mut sort_picked: Option<(usize, Option<ColumnSort>)> = None;
                builder
                    .header(row_height + 2., |mut header| {
                        for (col_ix, column) in self.columns.iter().enumerate() {
                            let current = sort.filter(|(ix, _)| *ix == col_ix).map(|(_, sort)| sort);
                            let (rect, response) = header.col(|ui| {
                                let title = if ui.visuals().dark_mode {
                                    let sorted = matches!(current, Some(ColumnSort::Ascending | ColumnSort::Descending));
                                    let color = theme::paint_header_cell(ui, sorted);
                                    let title = if self.exact_titles.contains(&column.key) {
                                        theme::exact_heading_text(&column.name)
                                    } else {
                                        theme::heading_text(&column.name)
                                    };
                                    title.color(color)
                                } else {
                                    RichText::new(&column.name).strong()
                                };
                                // The whole cell takes the click: the label senses nothing.
                                let mut text_rect = ui.max_rect();
                                text_rect.max.x -= SORT_ARROWS_WIDTH;
                                ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
                                    ui.add(Label::new(title).selectable(false).truncate());
                                });
                                sort_arrows(ui, current);
                            });
                            out.header_rects.push(rect);
                            let response = response
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text("Click to sort: ascending, descending, document order.\nRight-click to choose.");
                            if response.clicked() {
                                sort_picked = Some((col_ix, None));
                            }
                            response.context_menu(|ui| {
                                for (label, choice) in [
                                    ("Sort Ascending", ColumnSort::Ascending),
                                    ("Sort Descending", ColumnSort::Descending),
                                    ("Document Order", ColumnSort::Default),
                                ] {
                                    let checked = current.unwrap_or(ColumnSort::Default) == choice;
                                    if ui.add(egui::Button::new(label).selected(checked)).clicked() {
                                        sort_picked = Some((col_ix, Some(choice)));
                                    }
                                }
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(row_height, self.rows.len(), |mut row| {
                            let row_ix = row.index();
                            row.set_selected(selected == Some(row_ix));
                            for col_ix in 0..self.columns.len() {
                                let (_, response) = row.col(|ui| self.cell_ui(ui, row_ix, col_ix));
                                if response.clicked() {
                                    out.clicked_row = Some(row_ix);
                                }
                                if response.double_clicked() {
                                    self.start_edit(row_ix, col_ix);
                                }
                            }
                        });
                    });
                if let Some((col_ix, choice)) = sort_picked {
                    match choice {
                        Some(choice) => self.set_sort(col_ix, choice),
                        None => self.cycle_sort(col_ix),
                    }
                    out.resorted = true;
                }
            });
        out
    }

    /// Draws one cell: its editor while it is being edited, else its text in its [`CellStyle`].
    fn cell_ui(&mut self, ui: &mut Ui, row_ix: usize, col_ix: usize) {
        if ui.visuals().dark_mode {
            theme::paint_row_divider(ui);
        }
        let source_ix = self.rows[row_ix].source_ix;
        if let Some(editing) = &mut self.editing
            && editing.source_ix == source_ix
            && editing.col_ix == col_ix
        {
            let response = ui.add(TextEdit::singleline(&mut editing.text).desired_width(f32::INFINITY));
            if editing.focus {
                response.request_focus();
                editing.focus = false;
            } else if ui.input(|input| input.key_pressed(Key::Escape)) {
                self.finish_edit(false);
            } else if response.lost_focus() {
                // Enter, a click elsewhere or Tab.
                self.finish_edit(true);
            }
            return;
        }
        let text = &self.rows[row_ix].cells[col_ix];
        let rich = match self.rows[row_ix].styles[col_ix] {
            Some(CellStyle::Conflict) => {
                // Covers the whole cell, padding included.
                let rect = ui.max_rect().expand2(ui.spacing().item_spacing * 0.5);
                ui.painter().rect_filled(rect, 0., CONFLICT_BG);
                RichText::new(text).color(Color32::WHITE)
            }
            Some(CellStyle::Profile) => {
                let color = if ui.visuals().dark_mode { PROFILE_FG_DARK } else { PROFILE_FG_LIGHT };
                RichText::new(text).color(color).family(FontFamily::Name(super::BOLD.into()))
            }
            None => RichText::new(text),
        };
        ui.add(Label::new(rich).selectable(false).truncate());
    }
}

/// Room at a header cell's right edge for its [`sort_arrows`].
const SORT_ARROWS_WIDTH: f32 = 12.;

/// Draws an up and a down triangle at the right edge of a header cell: both faint when
/// the column is not sorted, else the one of its direction highlighted. Painted instead of
/// typed so they don't depend on the font having arrow glyphs.
fn sort_arrows(ui: &Ui, sort: Option<ColumnSort>) {
    let rect = ui.max_rect();
    let x = rect.right() - SORT_ARROWS_WIDTH / 2.;
    let y = rect.center().y;
    let (half, gap) = (4., 1.5);
    let visuals = ui.visuals();
    let strong = visuals.strong_text_color();
    let faint = visuals.weak_text_color().gamma_multiply(0.6);
    let (up, down) = match sort {
        Some(ColumnSort::Ascending) => (strong, faint),
        Some(ColumnSort::Descending) => (faint, strong),
        _ => (faint, faint),
    };
    let painter = ui.painter();
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(x, y - gap - half),
            egui::pos2(x + half, y - gap),
            egui::pos2(x - half, y - gap),
        ],
        up,
        egui::Stroke::NONE,
    ));
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(x - half, y + gap),
            egui::pos2(x + half, y + gap),
            egui::pos2(x, y + gap + half),
        ],
        down,
        egui::Stroke::NONE,
    ));
}

/// The length that `FIT_SHARE` of the non-empty `lengths` fit in; 0 when all are empty.
fn typical_length(lengths: impl Iterator<Item = usize>) -> usize {
    let mut lengths: Vec<usize> = lengths.filter(|len| *len > 0).collect();
    if lengths.is_empty() {
        return 0;
    }
    lengths.sort_unstable();
    let ix = ((lengths.len() as f32 * FIT_SHARE).ceil() as usize).clamp(1, lengths.len()) - 1;
    lengths[ix]
}

/// Orders numbers numerically and everything else case-insensitively, so
/// `2 < 10` and `"Speed" == "speed"` as a user would expect. Empty cells go last.
fn compare_values(a: &str, b: &str) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.total_cmp(&y),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        _ => a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctrlxref::xml_doc;

    #[test]
    fn numbers_sort_numerically_before_text_and_empty_last() {
        let mut values = vec!["10", "", "b", "2", "A", "-1.5"];
        values.sort_by(|a, b| compare_values(a, b));
        assert_eq!(values, ["-1.5", "2", "10", "A", "b", ""]);
    }

    /// The children of `root` as rows of their attributes.
    fn attribute_rows(xml: &str) -> Vec<TableRow> {
        let root = xml_doc::from_str(xml).unwrap();
        xml_doc::child_elements(&root)
            .map(|element| TableRow {
                tag: element.name.clone().into(),
                cells: element
                    .attributes
                    .iter()
                    .map(|(name, value)| (name.clone().into(), value.clone().into()))
                    .collect(),
                styles: Vec::new(),
            })
            .collect()
    }

    fn column_names(table: &AttributeTable) -> Vec<&str> {
        table.columns.iter().map(|c| c.name.as_ref()).collect()
    }

    #[test]
    fn leading_columns_come_first_then_others_in_first_seen_order() {
        let rows = attribute_rows(r#"<m><a x="p" n="1"/><a y="q" n="2" k="z"/></m>"#);
        let mut table = AttributeTable::new();
        table.set_rows(&["n", "rebind", "k"], &rows);

        assert_eq!(column_names(&table), ["n", "rebind", "k", "x", "y"]);
        assert_eq!(table.rows[0].cells, ["1", "", "", "p", ""]);
        assert_eq!(table.rows[1].cells, ["2", "", "z", "", "q"]);
    }

    #[test]
    fn column_width_ignores_rare_long_cells() {
        let lengths = [vec![0; 50], vec![5; 98], vec![50, 60]].concat();
        assert_eq!(typical_length(lengths.into_iter()), 5);
        assert_eq!(typical_length([3, 9].into_iter()), 9);
        assert_eq!(typical_length([0, 0].into_iter()), 0);
    }

    #[test]
    fn rows_keep_their_source_index_when_sorted() {
        let rows = attribute_rows(r#"<m><a n="3"/><a n="1"/><a n="2"/></m>"#);
        let mut table = AttributeTable::new();
        table.set_rows(&["n"], &rows);
        table.sort = Some((0, ColumnSort::Ascending));
        table.apply_sort();

        assert_eq!(table.rows[0].cells, ["1"]);
        assert_eq!(table.row_of(0), Some(2));
        assert_eq!(table.row_of(1), Some(0));
        assert_eq!(table.source_of(0), Some(1));
    }

    /// Runs `table` headless for a few frames, clicking the header cell of column `col_ix`
    /// at the point `at` gives for the cell's rectangle.
    fn click_header(table: &mut AttributeTable, col_ix: usize, at: impl Fn(egui::Rect) -> egui::Pos2) {
        let ctx = egui::Context::default();
        // Headers use the theme's heading font, which must be registered.
        crate::ui::setup_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200., 600.));
        let input = |events: Vec<egui::Event>| egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut rects = Vec::new();
        for _ in 0..3 {
            ctx.run_ui(input(Vec::new()), |ui| rects = table.show(ui, None, None).header_rects).textures_delta.clear();
        }
        let pos = at(rects[col_ix]);
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [vec![egui::Event::PointerMoved(pos)], vec![press(true)], vec![press(false)], Vec::new()] {
            ctx.run_ui(input(events), |ui| {
                table.show(ui, None, None);
            })
            .textures_delta
            .clear();
        }
    }

    #[test]
    fn clicking_a_header_cycles_its_sort() {
        let rows = attribute_rows(r#"<m><a n="3" k="b"/><a n="1" k="c"/><a n="2" k="a"/></m>"#);
        let mut table = AttributeTable::new();
        table.set_rows(&["n", "k"], &rows);
        let column = |table: &AttributeTable, ix: usize| table.rows.iter().map(|row| row.cells[ix].clone()).collect::<Vec<_>>();

        // On the header text.
        let on_text = |rect: egui::Rect| rect.left_center() + egui::vec2(4., 0.);
        // On the blank part of the cell, over the sort arrows.
        let on_arrows = |rect: egui::Rect| rect.right_center() - egui::vec2(SORT_ARROWS_WIDTH / 2., 0.);

        click_header(&mut table, 0, on_text);
        assert_eq!(column(&table, 0), ["1", "2", "3"]);
        click_header(&mut table, 0, on_arrows);
        assert_eq!(column(&table, 0), ["3", "2", "1"]);
        click_header(&mut table, 0, on_text);
        assert_eq!(column(&table, 0), ["3", "1", "2"]);
        // Another column starts ascending.
        click_header(&mut table, 1, on_arrows);
        assert_eq!(column(&table, 1), ["a", "b", "c"]);
        assert_eq!(table.sort, Some((1, ColumnSort::Ascending)));
    }

    #[test]
    fn the_sort_survives_new_rows_with_the_same_columns() {
        let rows = attribute_rows(r#"<m><a n="3"/><a n="1"/></m>"#);
        let mut table = AttributeTable::new();
        table.set_rows(&["n"], &rows);
        table.set_sort(0, ColumnSort::Descending);
        // Like after a cell edit or opening a rebind file.
        table.set_rows(&["n"], &attribute_rows(r#"<m><a n="3"/><a n="1"/><a n="7"/></m>"#));
        assert_eq!(table.rows.iter().map(|row| row.cells[0].as_str()).collect::<Vec<_>>(), ["7", "3", "1"]);
    }

    #[test]
    fn mixed_tags_add_element_column() {
        let rows = attribute_rows(r#"<m><a n="1"/><b n="2"/></m>"#);
        let mut table = AttributeTable::new();
        table.set_rows(&[], &rows);

        assert_eq!(column_names(&table), ["Element", "n"]);
        assert_eq!(table.rows[1].cells, ["b", "2"]);
    }
}
