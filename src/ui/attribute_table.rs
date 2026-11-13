//! Table that lists XML elements as rows, one column per attribute name.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::RangeInclusive;

use eframe::egui::text_selection::LabelSelectionState;
use eframe::egui::{
    self, Align, Color32, FontFamily, Key, KeyboardShortcut, Label, Modifiers, RichText, ScrollArea, Sense, TextEdit, Ui,
};
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

/// The text color of profile bindings in the current theme.
pub fn profile_color(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode { PROFILE_FG_DARK } else { PROFILE_FG_LIGHT }
}

/// The background of what a search marked: softer than the selection, which stays visible on it.
pub fn marked_color(ui: &Ui) -> Color32 {
    ui.visuals().selection.bg_fill.gamma_multiply(MARKED_STRENGTH)
}

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

/// Going through the conflicting cells, editing one at a time.
struct Resolving {
    /// Where it is: the row's source index and the column.
    source_ix: usize,
    col_ix: usize,
    /// Which way to go next, `true` for forward, once the committed edit has been applied.
    step: Option<bool>,
    /// The displayed row just arrived at, to select and scroll to.
    arrived: Option<usize>,
}

/// How much of the selection color marks what a search marked.
const MARKED_STRENGTH: f32 = 0.45;

/// The item of a cell's context menu that does the same as Ctrl+C.
const COPY_MENU_ITEM: &str = "Copy to clipboard";

/// The item of a cell's context menu that does the same as Ctrl+A.
const SELECT_ALL_MENU_ITEM: &str = "Select All";

/// The item of an editable cell's context menu that asks to clear the cell's whole column.
const CLEAR_COLUMN_MENU_ITEM: &str = "Clear All Column Bindings";

/// The item of an editable cell's context menu that asks to copy the default bindings into
/// the cell's column, where an action has none of its own.
const CLONE_DEFAULTS_MENU_ITEM: &str = "Clone default bindings";

/// The submenu of an editable cell's context menu that asks to copy the default bindings into
/// the cell's column for the device instance picked from it, like `js2_`.
const CLONE_DEFAULTS_TO_MENU_ITEM: &str = "Clone default bindings to";

/// The item of an editable cell's context menu that asks for a `*` wildcard pattern of
/// bindings to remove from the cell's column.
const REMOVE_BY_WILDCARDS_MENU_ITEM: &str = "Remove by * wildcards...";

/// The device instances [`CLONE_DEFAULTS_TO_MENU_ITEM`] offers, from 1.
pub const MAX_DEVICE_INSTANCE: u32 = 8;

/// How far from the edge of the visible cells a selection drag starts scrolling, in rows.
const DRAG_SCROLL_ZONE: f32 = 1.;

/// A selection drag's scrolling speed at the edge of the visible cells, in rows per second.
const DRAG_SCROLL_SPEED: f32 = 12.;

/// The fastest a selection drag scrolls, as a multiple of [`DRAG_SCROLL_SPEED`], reached with
/// the pointer beyond the edge of the visible cells.
const DRAG_SCROLL_MAX: f32 = 3.;

/// Whether one binding of a cell names a device but no input, like `kb1_ ` or `js2_`.
fn is_empty_binding(binding: &str) -> bool {
    let binding = binding.trim();
    binding.len() > 1 && binding.ends_with('_')
}

/// A rectangle of selected cells, between two `(displayed row, column)` corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellSelection {
    /// Where the selection started.
    anchor: (usize, usize),
    /// The corner a drag moves.
    focus: (usize, usize),
    /// Whether it is Select All, which is copied with the column headers.
    all: bool,
}

impl CellSelection {
    fn rows(&self) -> RangeInclusive<usize> {
        self.anchor.0.min(self.focus.0)..=self.anchor.0.max(self.focus.0)
    }

    fn columns(&self) -> RangeInclusive<usize> {
        self.anchor.1.min(self.focus.1)..=self.anchor.1.max(self.focus.1)
    }

    fn contains(&self, row_ix: usize, col_ix: usize) -> bool {
        self.rows().contains(&row_ix) && self.columns().contains(&col_ix)
    }
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
    /// The key of the column the user asked to clear with the context menu.
    pub clear_column: Option<String>,
    /// The key of the column whose empty bindings the user asked to clear with the context menu.
    pub clear_empty_column: Option<String>,
    /// The key of the column the user asked to fill with its default bindings with the context
    /// menu, and the device instance picked for them in a column that asks for one.
    pub clone_defaults_column: Option<(String, Option<u32>)>,
    /// The key of the column the user asked to remove bindings from by a wildcard pattern.
    pub wildcard_column: Option<String>,
    /// The key of the column whose device numbers, like `js1_`, the user asked to change.
    pub change_instance_column: Option<String>,
    /// Whether resolving conflicts ended because none are left.
    pub conflicts_resolved: bool,
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
    /// The Clear Empty Bindings menu item text of each editable column that has one, by key.
    clear_empty_items: HashMap<String, String>,
    /// Keys of the editable columns whose context menu offers [`CLONE_DEFAULTS_MENU_ITEM`].
    clone_default_columns: HashSet<String>,
    /// The editable columns whose context menu offers [`CLONE_DEFAULTS_TO_MENU_ITEM`], by key,
    /// with the device prefix code of its submenu's instances, like `js` for `js1_`.
    clone_default_instances: HashMap<String, String>,
    /// The editable columns whose context menu can change their bindings' device numbers, by
    /// key, with the menu item text.
    change_instance_items: HashMap<String, String>,
    /// Keys of the columns whose titles keep their case in the uppercase headers.
    exact_titles: HashSet<String>,
    editing: Option<Editing>,
    resolving: Option<Resolving>,
    /// Committed edits not yet taken by the owner.
    edits: Vec<CellEdit>,
    /// The cells selected with a drag or Select All. Replaces the highlight of the selected row.
    selection: Option<CellSelection>,
    /// Whether the mouse is dragging `selection`'s focus.
    dragging: bool,
    /// Source indices of the rows marked by a search. By source, so they stay through a sort.
    marked: HashSet<usize>,
    /// Whether the marks were cleared since the owner last asked.
    marks_cleared: bool,
}

impl AttributeTable {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            original: Vec::new(),
            rows: Vec::new(),
            sort: None,
            editable: HashSet::new(),
            clear_empty_items: HashMap::new(),
            clone_default_columns: HashSet::new(),
            clone_default_instances: HashMap::new(),
            change_instance_items: HashMap::new(),
            exact_titles: HashSet::new(),
            editing: None,
            resolving: None,
            edits: Vec::new(),
            selection: None,
            dragging: false,
            marked: HashSet::new(),
            marks_cleared: false,
        }
    }

    /// Clears the selected cells and the marked rows, leaving only the selected row.
    pub fn clear_selection(&mut self) {
        self.clear_cell_selection();
        self.clear_marks();
    }

    /// Clears the selected cells, keeping the marked rows.
    pub fn clear_cell_selection(&mut self) {
        self.selection = None;
        self.dragging = false;
    }

    /// Clears the marked rows.
    pub fn clear_marks(&mut self) {
        self.marked.clear();
        self.marks_cleared = true;
    }

    /// Marks the rows with these source indices, in place of any selected cells and earlier marks.
    pub fn set_marks(&mut self, rows: HashSet<usize>) {
        self.clear_cell_selection();
        self.marked = rows;
        self.marks_cleared = false;
    }

    /// Whether the marks were cleared since the last call, for an owner that marks more with them.
    pub fn take_marks_cleared(&mut self) -> bool {
        std::mem::take(&mut self.marks_cleared)
    }

    /// Ctrl+A selects all cells, and Ctrl+C copies the selected cells or else displayed row
    /// `selected`. Leaves the keys to the text field that has the keyboard, and Ctrl+C to any
    /// selected label text, which egui copies itself.
    pub fn shortcuts(&mut self, ctx: &egui::Context, selected: Option<usize>) {
        if self.editing.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let select_all = KeyboardShortcut::new(Modifiers::CTRL, Key::A);
        if ctx.input_mut(|input| input.consume_shortcut(&select_all)) {
            self.select_all(ctx);
        }
        let copy = ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Copy)));
        let label_selected = ctx.with_plugin(|labels: &mut LabelSelectionState| labels.has_selection());
        if copy
            && label_selected != Some(true)
            && let Some(text) = self.selection_text(selected)
        {
            ctx.copy_text(text);
        }
    }

    /// Selects all cells, as Ctrl+A and the context menu do.
    fn select_all(&mut self, ctx: &egui::Context) {
        if self.rows.is_empty() {
            return;
        }
        self.clear_selection();
        self.selection = Some(CellSelection {
            anchor: (0, 0),
            focus: (self.rows.len() - 1, self.columns.len() - 1),
            all: true,
        });
        clear_label_selection(ctx);
    }

    /// The selected cells, or else the marked rows, or else displayed row `selected`, as
    /// tab-separated text in the displayed order. Only Select All has the column headers as its
    /// first line.
    fn selection_text(&self, selected: Option<usize>) -> Option<String> {
        if self.selection.is_none() && !self.marked.is_empty() {
            let marked = self.rows.iter().filter(|row| self.marked.contains(&row.source_ix));
            return Some(ctrlxref::csv::to_tsv(marked.map(|row| row.cells.as_slice())));
        }
        let (rows, columns, all) = match self.selection {
            Some(selection) => (selection.rows(), selection.columns(), selection.all),
            None => {
                let row = selected.filter(|row| *row < self.rows.len())?;
                (row..=row, 0..=self.columns.len().checked_sub(1)?, false)
            }
        };
        let headers = all.then(|| self.headers());
        let cells = self.rows[rows].iter().map(|row| row.cells[columns.clone()].to_vec());
        Some(ctrlxref::csv::to_tsv(headers.into_iter().chain(cells)))
    }

    /// Where a selection drag points, inside the visible cells, while it goes on. Scrolls
    /// them when the pointer is near or beyond their edge. Call inside the table body, whose
    /// scroll areas take the scrolling.
    fn drag_position(&mut self, ui: &Ui, zone: f32) -> Option<egui::Pos2> {
        if !self.dragging {
            return None;
        }
        let (down, pos, dt) = ui.input(|input| (input.pointer.primary_down(), input.pointer.latest_pos(), input.stable_dt));
        if !down {
            self.dragging = false;
            return None;
        }
        let pos = pos?;
        let visible = ui.clip_rect();
        let delta = drag_scroll(pos, visible, zone, dt);
        if delta != egui::Vec2::ZERO {
            ui.scroll_with_delta_animation(delta, egui::style::ScrollAnimation::none());
            ui.ctx().request_repaint();
        }
        Some(visible.clamp(pos))
    }

    /// Makes the cells of the columns with these keys editable with a double-click.
    pub fn set_editable(&mut self, columns: &[&str]) {
        self.editable = columns.iter().map(|name| name.to_string()).collect();
    }

    /// Gives the editable columns `items` (`(column key, menu item text)`) a context menu item
    /// that asks to clear the column's empty bindings, such as `kb1_ `.
    pub fn set_clear_empty_items(&mut self, items: &[(&str, String)]) {
        self.clear_empty_items = items.iter().map(|(key, text)| (key.to_string(), text.clone())).collect();
    }

    /// Gives the editable `columns` a context menu item that asks to copy the default
    /// bindings into the column.
    pub fn set_clone_default_columns(&mut self, columns: &[&str]) {
        self.clone_default_columns = columns.iter().map(|name| name.to_string()).collect();
    }

    /// Gives the editable `columns` (`(column key, device prefix code)`) a context submenu that
    /// asks to copy the default bindings into the column for a device instance picked from it,
    /// like `js2_` for the code `js`.
    pub fn set_clone_default_instances(&mut self, columns: &[(&str, &str)]) {
        self.clone_default_instances = columns.iter().map(|(key, code)| (key.to_string(), code.to_string())).collect();
    }

    /// Gives the editable columns `items` (`(column key, menu item text)`) a context menu item
    /// that asks to change the device number of the column's bindings, like `js1_` to `js2_`.
    pub fn set_change_instance_items(&mut self, items: &[(&str, &str)]) {
        self.change_instance_items = items.iter().map(|(key, text)| (key.to_string(), text.to_string())).collect();
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

    /// Starts editing the first conflicting cell, to go through them with [`Self::resolve_keys`].
    /// Returns whether there is one.
    pub fn start_resolving(&mut self) -> bool {
        match self.next_conflict(None, true) {
            Some(cell) => {
                self.open_conflict(cell);
                true
            }
            None => false,
        }
    }

    /// The conflicting cell after `from` (a displayed row and column), or before it when not
    /// `forward`, going around the ends. Rows go from top to bottom, and within one left to right.
    fn next_conflict(&self, from: Option<(usize, usize)>, forward: bool) -> Option<(usize, usize)> {
        let conflicts: Vec<(usize, usize)> = self
            .rows
            .iter()
            .enumerate()
            .flat_map(|(row_ix, row)| {
                let columns = row.styles.iter().enumerate();
                columns.filter(|(_, style)| **style == Some(CellStyle::Conflict)).map(move |(col_ix, _)| (row_ix, col_ix))
            })
            .collect();
        let next = from.and_then(|from| match forward {
            true => conflicts.iter().find(|cell| **cell > from),
            false => conflicts.iter().rev().find(|cell| **cell < from),
        });
        next.or(if forward { conflicts.first() } else { conflicts.last() }).copied()
    }

    fn open_conflict(&mut self, (row_ix, col_ix): (usize, usize)) {
        self.start_edit(row_ix, col_ix);
        self.resolving = Some(Resolving { source_ix: self.rows[row_ix].source_ix, col_ix, step: None, arrived: Some(row_ix) });
    }

    /// While resolving conflicts: Tab or Enter keeps the cell's text and goes to the next
    /// conflict, Shift+Tab to the previous, and Escape drops the text and stops. Leaving the editor
    /// any other way, such as by clicking elsewhere, also stops. Runs before the editor is
    /// drawn, to get the keys first. Returns whether no conflicts are left.
    fn resolve_keys(&mut self, ctx: &egui::Context) -> bool {
        let Some(resolving) = &mut self.resolving else {
            return false;
        };
        let (escape, step) = ctx.input_mut(|input| {
            // The variant with Shift comes first: the plain one also matches with Shift held.
            let escape = input.consume_key(Modifiers::NONE, Key::Escape);
            let step = if input.consume_key(Modifiers::SHIFT, Key::Tab) {
                Some(false)
            } else if input.consume_key(Modifiers::NONE, Key::Tab) || input.consume_key(Modifiers::NONE, Key::Enter) {
                Some(true)
            } else {
                None
            };
            (escape, step)
        });
        if escape {
            self.resolving = None;
            self.finish_edit(false);
        } else if step.is_some() {
            resolving.step = step;
            self.finish_edit(true);
            // The next frame goes on, with the edit applied and the conflicts up to date.
            ctx.request_repaint();
        } else if let Some(forward) = resolving.step.take() {
            let (source_ix, col_ix) = (resolving.source_ix, resolving.col_ix);
            let from = self.row_of(source_ix).map(|row_ix| (row_ix, col_ix));
            match self.next_conflict(from, forward) {
                Some(cell) => self.open_conflict(cell),
                None => {
                    self.resolving = None;
                    return true;
                }
            }
        } else if self.editing.is_none() {
            self.resolving = None;
        }
        false
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
        // The rows the editor, the selection and the marks were on are gone.
        self.editing = None;
        self.clear_selection();
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

    /// Each row's source index (into the last `set_rows` slice) and cells, in document order
    /// whatever the current sort.
    pub fn document_rows(&self) -> impl Iterator<Item = (usize, &[String])> {
        self.original.iter().map(|row| (row.source_ix, row.cells.as_slice()))
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
        // The selected rows moved; marks go with their rows.
        self.clear_cell_selection();
    }

    /// Draws the table filling `ui`, with displayed row `selected` highlighted and, when
    /// given, row `scroll_to` scrolled into the middle of the view.
    pub fn show(&mut self, ui: &mut Ui, selected: Option<usize>, scroll_to: Option<usize>) -> TableOutput {
        let mut out = TableOutput::default();
        if self.columns.is_empty() {
            return out;
        }
        out.conflicts_resolved = self.resolve_keys(ui.ctx());
        let arrived = self.resolving.as_mut().and_then(|resolving| resolving.arrived.take());
        out.clicked_row = arrived;
        let scroll_to = arrived.or(scroll_to);
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
                    .sense(Sense::click_and_drag())
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
                                let title = if self.exact_titles.contains(&column.key) {
                                    theme::exact_heading_text(&column.name)
                                } else {
                                    theme::heading_text(&column.name)
                                };
                                let title = if ui.visuals().dark_mode {
                                    let sorted = matches!(current, Some(ColumnSort::Ascending | ColumnSort::Descending));
                                    title.color(theme::paint_header_cell(ui, sorted))
                                } else {
                                    title
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
                    .body(|mut body| {
                        let drag = self.drag_position(body.ui_mut(), row_height * DRAG_SCROLL_ZONE);
                        // The last drawn row and column that start before the drag's position.
                        let (mut drag_row, mut drag_col) = (None, None);
                        body.rows(row_height, self.rows.len(), |mut row| {
                            let row_ix = row.index();
                            row.set_selected(selected == Some(row_ix) && self.selection.is_none());
                            for col_ix in 0..self.columns.len() {
                                let (_, response) = row.col(|ui| self.cell_ui(ui, row_ix, col_ix));
                                if let Some(pos) = drag {
                                    if pos.y >= response.rect.top() {
                                        drag_row = Some(row_ix);
                                    }
                                    if pos.x >= response.rect.left() {
                                        drag_col = Some(col_ix);
                                    }
                                }
                                if response.clicked() {
                                    out.clicked_row = Some(row_ix);
                                    self.clear_selection();
                                    clear_label_selection(&response.ctx);
                                }
                                if response.drag_started_by(egui::PointerButton::Primary) {
                                    // The tree stays on the row where the drag started.
                                    out.clicked_row = Some(row_ix);
                                    self.clear_selection();
                                    self.selection = Some(CellSelection {
                                        anchor: (row_ix, col_ix),
                                        focus: (row_ix, col_ix),
                                        all: false,
                                    });
                                    self.dragging = true;
                                    clear_label_selection(&response.ctx);
                                }
                                if response.double_clicked() {
                                    self.start_edit(row_ix, col_ix);
                                }
                                // A right-click inside the selected cells or the marked rows keeps
                                // them, so the menu copies them; elsewhere it selects the row, like
                                // a click.
                                if response.secondary_clicked()
                                    && !self.selection.is_some_and(|selection| selection.contains(row_ix, col_ix))
                                    && !self.is_marked(row_ix)
                                {
                                    out.clicked_row = Some(row_ix);
                                    self.clear_selection();
                                }
                                response.context_menu(|ui| {
                                    if ui.button(COPY_MENU_ITEM).clicked() {
                                        if let Some(text) = self.selection_text(selected) {
                                            ui.ctx().copy_text(text);
                                        }
                                        ui.close();
                                    }
                                    if ui.button(SELECT_ALL_MENU_ITEM).clicked() {
                                        self.select_all(ui.ctx());
                                        ui.close();
                                    }
                                    let key = &self.columns[col_ix].key;
                                    if self.editable.contains(key) {
                                        ui.separator();
                                        let has_values =
                                            self.rows.iter().any(|row| !row.cells[col_ix].trim().is_empty());
                                        let clear = egui::Button::new(CLEAR_COLUMN_MENU_ITEM);
                                        if ui.add_enabled(has_values, clear).clicked() {
                                            out.clear_column = Some(key.clone());
                                            ui.close();
                                        }
                                        if let Some(text) = self.clear_empty_items.get(key) {
                                            let has_empty = self
                                                .rows
                                                .iter()
                                                .any(|row| row.cells[col_ix].split('|').any(is_empty_binding));
                                            let clear = egui::Button::new(text.as_str());
                                            if ui.add_enabled(has_empty, clear).clicked() {
                                                out.clear_empty_column = Some(key.clone());
                                                ui.close();
                                            }
                                        }
                                        if self.clone_default_columns.contains(key)
                                            && ui.button(CLONE_DEFAULTS_MENU_ITEM).clicked()
                                        {
                                            out.clone_defaults_column = Some((key.clone(), None));
                                            ui.close();
                                        }
                                        if let Some(code) = self.clone_default_instances.get(key) {
                                            ui.menu_button(CLONE_DEFAULTS_TO_MENU_ITEM, |ui| {
                                                for instance in 1..=MAX_DEVICE_INSTANCE {
                                                    if ui.button(format!("{code}{instance}_")).clicked() {
                                                        out.clone_defaults_column = Some((key.clone(), Some(instance)));
                                                        ui.close();
                                                    }
                                                }
                                            });
                                        }
                                        let remove = egui::Button::new(REMOVE_BY_WILDCARDS_MENU_ITEM);
                                        if ui.add_enabled(has_values, remove).clicked() {
                                            out.wildcard_column = Some(key.clone());
                                            ui.close();
                                        }
                                        if let Some(text) = self.change_instance_items.get(key) {
                                            let change = egui::Button::new(text.as_str());
                                            if ui.add_enabled(has_values, change).clicked() {
                                                out.change_instance_column = Some(key.clone());
                                                ui.close();
                                            }
                                        }
                                    }
                                });
                            }
                        });
                        if let Some(selection) = &mut self.selection {
                            selection.focus.0 = drag_row.unwrap_or(selection.focus.0);
                            selection.focus.1 = drag_col.unwrap_or(selection.focus.1);
                        }
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

    /// Whether the row displayed at `row_ix` is marked, while no cells are selected.
    fn is_marked(&self, row_ix: usize) -> bool {
        self.selection.is_none() && self.rows.get(row_ix).is_some_and(|row| self.marked.contains(&row.source_ix))
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
            // While resolving, Tab goes to the next conflict instead of the next widget.
            let edit = TextEdit::singleline(&mut editing.text).desired_width(f32::INFINITY).lock_focus(self.resolving.is_some());
            let mut output = edit.show(ui);
            let response = output.response.response.clone();
            // Shift+click keeps extending the selection.
            if response.clicked()
                && !ui.input(|input| input.modifiers.shift)
                && let Some(cursor) = output.cursor_range
            {
                let (start, end) = binding_selection(&editing.text, usize::from(cursor.primary.index));
                let range = egui::text::CCursorRange::two(egui::text::CCursor::new(start), egui::text::CCursor::new(end));
                output.state.cursor.set_char_range(Some(range));
                output.state.store(ui.ctx(), response.id);
            }
            if editing.focus {
                response.request_focus();
                response.scroll_to_me(None);
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
        let cell_selected = self.selection.is_some_and(|selection| selection.contains(row_ix, col_ix));
        if cell_selected {
            let rect = ui.max_rect().expand2(ui.spacing().item_spacing * 0.5);
            ui.painter().rect_filled(rect, 0., ui.visuals().selection.bg_fill);
        } else if self.is_marked(row_ix) {
            let rect = ui.max_rect().expand2(ui.spacing().item_spacing * 0.5);
            ui.painter().rect_filled(rect, 0., marked_color(ui));
        }
        let style = self.rows[row_ix].styles[col_ix].filter(|style| !(cell_selected && *style == CellStyle::Conflict));
        let rich = match style {
            Some(CellStyle::Conflict) => {
                // Covers the whole cell, padding included.
                let rect = ui.max_rect().expand2(ui.spacing().item_spacing * 0.5);
                ui.painter().rect_filled(rect, 0., CONFLICT_BG);
                RichText::new(text).color(Color32::WHITE)
            }
            Some(CellStyle::Profile) => {
                RichText::new(text).color(profile_color(ui)).family(FontFamily::Name(super::BOLD.into()))
            }
            None => RichText::new(text),
        };
        // Other text colors are hard to read on the selection's.
        let rich = if cell_selected { rich.color(ui.visuals().selection.stroke.color) } else { rich };
        ui.add(Label::new(rich).selectable(false).truncate());
    }
}

/// The characters a click at character `cursor` selects in the text of a cell: the binding there,
/// all of it when the cell has only one. With several, it goes with the separator after it, or for
/// the last one the separator before it, so that deleting the selection leaves one separator
/// between the bindings that remain.
fn binding_selection(text: &str, cursor: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let separators: Vec<usize> =
        chars.iter().enumerate().filter(|(_, c)| matches!(c, '|' | ',')).map(|(ix, _)| ix).collect();
    let skip_spaces = |mut ix: usize| {
        while chars.get(ix).is_some_and(|c| c.is_whitespace()) {
            ix += 1;
        }
        ix
    };
    let binding = separators.iter().filter(|separator| **separator < cursor).count();
    match separators.get(binding) {
        // Up to the next binding, and from the start of this one.
        Some(separator) => {
            let start = if binding == 0 { 0 } else { skip_spaces(separators[binding - 1] + 1) };
            (start, skip_spaces(separator + 1))
        }
        None => match separators.last() {
            Some(separator) => {
                let mut start = *separator;
                while start > 0 && chars[start - 1].is_whitespace() {
                    start -= 1;
                }
                (start, chars.len())
            }
            None => (0, chars.len()),
        },
    }
}

/// Clears selected label text, such as in the status bar or a help window, so that Ctrl+C goes
/// to the table again.
fn clear_label_selection(ctx: &egui::Context) {
    ctx.with_plugin(|labels: &mut LabelSelectionState| labels.clear_selection());
}

/// How far to scroll for a selection drag at `pos` this frame: nothing while it is further than
/// `zone` inside `visible`, else more the further out it is, up to [`DRAG_SCROLL_MAX`].
fn drag_scroll(pos: egui::Pos2, visible: egui::Rect, zone: f32, dt: f32) -> egui::Vec2 {
    let axis = |pos: f32, min: f32, max: f32| {
        let beyond = (pos - (max - zone)).max(0.) - ((min + zone) - pos).max(0.);
        -(beyond / zone).clamp(-DRAG_SCROLL_MAX, DRAG_SCROLL_MAX) * DRAG_SCROLL_SPEED * zone * dt
    };
    egui::vec2(axis(pos.x, visible.left(), visible.right()), axis(pos.y, visible.top(), visible.bottom()))
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

    #[test]
    fn document_rows_ignore_the_sort() {
        let rows = attribute_rows(r#"<m><a n="3"/><a n="1"/><a n="2"/></m>"#);
        let mut table = AttributeTable::new();
        table.set_rows(&["n"], &rows);
        table.set_sort(0, ColumnSort::Ascending);

        let displayed: Vec<usize> = table.displayed_rows().map(|(source_ix, _)| source_ix).collect();
        assert_eq!(displayed, [1, 2, 0]);
        let document: Vec<(usize, &str)> =
            table.document_rows().map(|(source_ix, cells)| (source_ix, cells[0].as_str())).collect();
        assert_eq!(document, [(0, "3"), (1, "1"), (2, "2")]);
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

    /// `count` rows of cells `<name><row>` for each of `names`.
    fn table_of(names: &[&str], count: usize) -> AttributeTable {
        let rows: Vec<TableRow> = (0..count)
            .map(|row| TableRow {
                tag: "a".into(),
                cells: names.iter().map(|name| (name.to_string(), format!("{name}{row}"))).collect(),
                styles: Vec::new(),
            })
            .collect();
        let mut table = AttributeTable::new();
        table.set_rows(names, &rows);
        table
    }

    /// Runs a table headless, one frame per call, like the app does.
    struct Headless {
        ctx: egui::Context,
        screen: egui::Rect,
        header_rects: Vec<egui::Rect>,
        row_step: f32,
    }

    impl Headless {
        fn new(table: &mut AttributeTable, width: f32, height: f32) -> Self {
            let ctx = egui::Context::default();
            crate::ui::setup_fonts(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            let mut headless = Self { ctx, screen, header_rects: Vec::new(), row_step: 0. };
            for _ in 0..3 {
                headless.frame(table, Vec::new(), None);
            }
            headless
        }

        /// One frame with `events`; returns what the table reported and the clipboard text.
        fn frame(
            &mut self,
            table: &mut AttributeTable,
            events: Vec<egui::Event>,
            selected: Option<usize>,
        ) -> (TableOutput, Option<String>) {
            let input = egui::RawInput { screen_rect: Some(self.screen), events, ..Default::default() };
            let mut out = TableOutput::default();
            let mut row_step = 0.;
            let mut output = self.ctx.run_ui(input, |ui| {
                row_step = ui.text_style_height(&egui::TextStyle::Body) + 6. + ui.spacing().item_spacing.y;
                table.shortcuts(ui.ctx(), selected);
                out = table.show(ui, selected, None);
            });
            output.textures_delta.clear();
            self.row_step = row_step;
            self.header_rects = out.header_rects.clone();
            let copied = output.platform_output.commands.iter().find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            });
            (out, copied)
        }

        /// The middle of the cell at displayed `row`, `col`, with nothing scrolled.
        fn cell(&self, row: usize, col: usize) -> egui::Pos2 {
            let header = self.header_rects[col];
            egui::pos2(header.center().x, header.bottom() + self.row_step * (row as f32 + 0.5))
        }

        fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }
        }

        /// Presses at `from` and moves to `to`, still holding the button, for `frames` frames.
        /// Returns the row reported on the way.
        fn drag(&mut self, table: &mut AttributeTable, from: egui::Pos2, to: egui::Pos2, frames: usize) -> Option<usize> {
            let mut clicked = None;
            for events in [vec![egui::Event::PointerMoved(from)], vec![Self::press(from, true)], vec![egui::Event::PointerMoved(to)]] {
                clicked = clicked.or(self.frame(table, events, None).0.clicked_row);
            }
            for _ in 0..frames {
                clicked = clicked.or(self.frame(table, Vec::new(), None).0.clicked_row);
            }
            clicked
        }

        fn release(&mut self, table: &mut AttributeTable, pos: egui::Pos2) {
            self.frame(table, vec![Self::press(pos, false)], None);
            self.frame(table, Vec::new(), None);
        }

        fn ctrl_a(&mut self, table: &mut AttributeTable) {
            let key = egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            };
            self.frame(table, vec![key], None);
        }
    }

    #[test]
    fn dragging_selects_a_rectangle_and_copies_it_without_headers() {
        let mut table = table_of(&["a", "b", "c"], 6);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        let (from, to) = (headless.cell(4, 2), headless.cell(2, 1));

        // The tree stays on the row where the drag started.
        assert_eq!(headless.drag(&mut table, from, to, 2), Some(4));
        assert!(table.dragging);
        headless.release(&mut table, to);
        assert!(!table.dragging);

        let selection = table.selection.unwrap();
        assert_eq!((selection.rows(), selection.columns()), (2..=4, 1..=2));
        let (_, copied) = headless.frame(&mut table, vec![egui::Event::Copy], None);
        assert_eq!(copied.as_deref(), Some("b2\tc2\nb3\tc3\nb4\tc4"));
    }

    #[test]
    fn a_click_clears_the_selection_and_ctrl_c_copies_the_selected_row() {
        let mut table = table_of(&["a", "b"], 3);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        headless.ctrl_a(&mut table);
        assert!(table.selection.is_some());

        let pos = headless.cell(1, 0);
        headless.frame(&mut table, vec![egui::Event::PointerMoved(pos)], None);
        headless.frame(&mut table, vec![Headless::press(pos, true)], None);
        let (out, _) = headless.frame(&mut table, vec![Headless::press(pos, false)], None);
        assert_eq!(out.clicked_row, Some(1));
        assert_eq!(table.selection, None);

        let (_, copied) = headless.frame(&mut table, vec![egui::Event::Copy], Some(1));
        assert_eq!(copied.as_deref(), Some("a1\tb1"));
        // Nothing is selected: the clipboard is left alone.
        let (_, copied) = headless.frame(&mut table, vec![egui::Event::Copy], None);
        assert_eq!(copied, None);
    }

    /// A right-click at `pos`: its press and release, then a frame for the menu to open.
    fn right_click(headless: &mut Headless, table: &mut AttributeTable, pos: egui::Pos2, selected: Option<usize>) -> Option<usize> {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut clicked = None;
        for events in [vec![egui::Event::PointerMoved(pos)], vec![button(true)], vec![button(false)], Vec::new()] {
            clicked = clicked.or(headless.frame(table, events, selected).0.clicked_row);
        }
        clicked
    }

    /// Clicks an item, counted from 0, of the menu opened at `pos`, returning the copied text.
    fn click_menu_item(
        headless: &mut Headless,
        table: &mut AttributeTable,
        pos: egui::Pos2,
        item: usize,
        selected: Option<usize>,
    ) -> Option<String> {
        let item = pos + egui::vec2(24., 16. + 21. * item as f32);
        let mut copied = None;
        for events in [
            vec![egui::Event::PointerMoved(item)],
            vec![Headless::press(item, true)],
            vec![Headless::press(item, false)],
            Vec::new(),
        ] {
            copied = copied.or(headless.frame(table, events, selected).1);
        }
        copied
    }

    #[test]
    fn the_context_menu_copies_the_selected_cells() {
        let mut table = table_of(&["a", "b", "c"], 6);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        let (from, to) = (headless.cell(1, 0), headless.cell(2, 1));
        headless.drag(&mut table, from, to, 2);
        headless.release(&mut table, to);

        // Inside the selection: it stays, and the row selection doesn't move.
        let pos = headless.cell(2, 0);
        assert_eq!(right_click(&mut headless, &mut table, pos, None), None);
        assert!(table.selection.is_some());
        assert_eq!(click_menu_item(&mut headless, &mut table, pos, 0, None).as_deref(), Some("a1\tb1\na2\tb2"));
    }

    /// Right-clicks the cell at `row`, `col` and clicks the menu's clear item, past the separator.
    /// Returns the column the table then asks to clear.
    fn clear_from_menu(headless: &mut Headless, table: &mut AttributeTable, row: usize, col: usize) -> Option<String> {
        let pos = headless.cell(row, col);
        right_click(headless, table, pos, Some(row));
        let item = pos + egui::vec2(24., 16. + 21. * 2. + 6.);
        let mut asked = None;
        for events in [
            vec![egui::Event::PointerMoved(item)],
            vec![Headless::press(item, true)],
            vec![Headless::press(item, false)],
            Vec::new(),
        ] {
            asked = asked.or(headless.frame(table, events, Some(row)).0.clear_column);
        }
        asked
    }

    #[test]
    fn the_context_menu_clears_only_editable_columns_with_values() {
        let mut table = table_of(&["name", "keyboard", "mouse"], 3);
        table.set_editable(&["keyboard", "mouse"]);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        assert_eq!(clear_from_menu(&mut headless, &mut table, 1, 1).as_deref(), Some("keyboard"));
        assert_eq!(clear_from_menu(&mut headless, &mut table, 2, 2).as_deref(), Some("mouse"));
        assert_eq!(clear_from_menu(&mut headless, &mut table, 1, 0), None);

        // A column with nothing to clear: its item is disabled.
        let rows = attribute_rows(r#"<m><a name="1" keyboard=" "/><a name="2" keyboard=" "/></m>"#);
        let mut table = AttributeTable::new();
        table.set_rows(&["name", "keyboard"], &rows);
        table.set_editable(&["keyboard"]);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        assert_eq!(clear_from_menu(&mut headless, &mut table, 0, 1), None);
    }

    #[test]
    fn the_context_menu_clears_empty_bindings_only_when_the_column_has_some() {
        /// Right-clicks the cell at `row`, `col` and clicks the item below Clear All Column Bindings.
        fn clear_empty_from_menu(headless: &mut Headless, table: &mut AttributeTable, row: usize, col: usize) -> Option<String> {
            let pos = headless.cell(row, col);
            right_click(headless, table, pos, Some(row));
            // Clear All Column Bindings spans about 60..85 below the click; this item is next.
            let item = pos + egui::vec2(24., 95.);
            let mut asked = None;
            for events in [
                vec![egui::Event::PointerMoved(item)],
                vec![Headless::press(item, true)],
                vec![Headless::press(item, false)],
                Vec::new(),
            ] {
                asked = asked.or(headless.frame(table, events, Some(row)).0.clear_empty_column);
            }
            asked
        }

        let rows = attribute_rows(r#"<m><a name="1" keyboard="kb1_  | kb1_x" mouse="mo1_m"/><a name="2" keyboard="kb1_y"/></m>"#);
        let mut table = AttributeTable::new();
        table.set_rows(&["name", "keyboard", "mouse"], &rows);
        table.set_editable(&["keyboard", "mouse"]);
        table.set_clear_empty_items(&[("keyboard", "Clear all kb1_ empty bindings".into()), ("mouse", "Clear all mo1_ empty bindings".into())]);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        assert_eq!(clear_empty_from_menu(&mut headless, &mut table, 1, 1).as_deref(), Some("keyboard"));
        // The mouse column has no empty bindings: its item is disabled.
        assert_eq!(clear_empty_from_menu(&mut headless, &mut table, 0, 2), None);
    }

    #[test]
    fn the_context_menu_selects_all_like_ctrl_a() {
        let mut table = table_of(&["a", "b"], 3);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        let pos = headless.cell(1, 0);

        right_click(&mut headless, &mut table, pos, None);
        click_menu_item(&mut headless, &mut table, pos, 1, Some(1));
        let from_menu = table.selection;
        assert_eq!(from_menu.map(|selection| (selection.rows(), selection.columns(), selection.all)), Some((0..=2, 0..=1, true)));
        let (_, copied) = headless.frame(&mut table, vec![egui::Event::Copy], Some(1));
        assert_eq!(copied.as_deref(), Some("a\tb\na0\tb0\na1\tb1\na2\tb2"));

        table.clear_selection();
        headless.ctrl_a(&mut table);
        assert_eq!(table.selection, from_menu);
    }

    #[test]
    fn the_context_menu_elsewhere_selects_and_copies_that_row() {
        let mut table = table_of(&["a", "b"], 6);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        let pos = headless.cell(0, 0);
        let (from, to) = (headless.cell(4, 0), headless.cell(5, 0));
        headless.drag(&mut table, from, to, 2);
        headless.release(&mut table, to);
        assert_eq!(right_click(&mut headless, &mut table, pos, None), Some(0));
        assert_eq!(table.selection, None);
        // The app selects the reported row before the menu item is clicked.
        assert_eq!(click_menu_item(&mut headless, &mut table, pos, 0, Some(0)).as_deref(), Some("a0\tb0"));
    }

    #[test]
    fn select_all_copies_with_headers_in_the_displayed_order() {
        let rows = attribute_rows(r#"<m><a n="3" k=" "/><a n="1" k="x"/><a n="2" k="y"/></m>"#);
        let mut table = AttributeTable::new();
        table.set_rows(&["n", "k"], &rows);
        table.set_sort(0, ColumnSort::Ascending);
        let mut headless = Headless::new(&mut table, 1200., 600.);

        headless.ctrl_a(&mut table);
        let (_, copied) = headless.frame(&mut table, vec![egui::Event::Copy], None);
        assert_eq!(copied.as_deref(), Some("n\tk\n1\tx\n2\ty\n3\t"));

        // Sorting moves the rows, so the selection goes.
        table.set_sort(0, ColumnSort::Descending);
        assert_eq!(table.selection, None);
    }

    #[test]
    fn dragging_past_the_edges_scrolls_to_more_cells() {
        // Only some of the rows and columns fit.
        let names = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"];
        let mut table = table_of(&names, 300);
        let mut headless = Headless::new(&mut table, 260., 220.);
        let from = headless.cell(0, 0);
        let to = egui::pos2(260., 220.);

        headless.drag(&mut table, from, to, 90);
        let selection = table.selection.unwrap();
        assert!(selection.focus.0 > 20, "row {}", selection.focus.0);
        assert_eq!(selection.focus.1, names.len() - 1);

        // And back to where it started.
        headless.frame(&mut table, vec![egui::Event::PointerMoved(egui::pos2(0., 0.))], None);
        for _ in 0..240 {
            headless.frame(&mut table, Vec::new(), None);
        }
        assert_eq!(table.selection.unwrap().focus, (0, 0));
    }

    #[test]
    fn marks_copy_their_rows_in_the_displayed_order_and_stay_through_a_sort() {
        let mut table = table_of(&["a", "b"], 4);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        table.set_marks(HashSet::from([0, 2]));
        assert!(!table.take_marks_cleared());

        let (_, copied) = headless.frame(&mut table, vec![egui::Event::Copy], Some(3));
        assert_eq!(copied.as_deref(), Some("a0\tb0\na2\tb2"));

        table.set_sort(0, ColumnSort::Descending);
        assert!(table.is_marked(1) && table.is_marked(3));
        assert!(!table.is_marked(0) && !table.is_marked(2));
        let (_, copied) = headless.frame(&mut table, vec![egui::Event::Copy], None);
        assert_eq!(copied.as_deref(), Some("a2\tb2\na0\tb0"));
        assert!(!table.take_marks_cleared());
    }

    #[test]
    fn finding_keeps_the_marks_and_clicking_clears_them() {
        let mut table = table_of(&["a"], 4);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        table.set_marks(HashSet::from([0]));

        // What Find Next does to the table.
        table.clear_cell_selection();
        assert!(table.is_marked(0));
        assert!(!table.take_marks_cleared());

        let pos = headless.cell(2, 0);
        headless.frame(&mut table, vec![egui::Event::PointerMoved(pos)], None);
        headless.frame(&mut table, vec![Headless::press(pos, true)], None);
        headless.frame(&mut table, vec![Headless::press(pos, false)], None);
        assert!(!table.is_marked(0));
        assert!(table.take_marks_cleared());
        assert!(!table.take_marks_cleared());
    }

    #[test]
    fn dragging_and_select_all_replace_the_marks() {
        let mut table = table_of(&["a", "b"], 4);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        table.set_marks(HashSet::from([3]));
        headless.ctrl_a(&mut table);
        assert!(table.marked.is_empty());
        assert!(table.take_marks_cleared());

        table.clear_selection();
        table.set_marks(HashSet::from([3]));
        let (from, to) = (headless.cell(0, 0), headless.cell(1, 1));
        headless.drag(&mut table, from, to, 2);
        headless.release(&mut table, to);
        assert!(table.marked.is_empty());
        let (_, copied) = headless.frame(&mut table, vec![egui::Event::Copy], None);
        assert_eq!(copied.as_deref(), Some("a0\tb0\na1\tb1"));
    }

    #[test]
    fn a_right_click_on_a_marked_row_keeps_the_marks() {
        let mut table = table_of(&["a"], 4);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        table.set_marks(HashSet::from([1, 2]));

        let inside = headless.cell(1, 0);
        assert_eq!(right_click(&mut headless, &mut table, inside, None), None);
        assert_eq!(click_menu_item(&mut headless, &mut table, inside, 0, None).as_deref(), Some("a1\na2"));

        let outside = headless.cell(3, 0);
        assert_eq!(right_click(&mut headless, &mut table, outside, None), Some(3));
        assert!(table.marked.is_empty());
    }

    #[test]
    fn the_cell_editor_keeps_the_keyboard() {
        let mut table = table_of(&["a"], 2);
        table.set_editable(&["a"]);
        table.start_edit(0, 0);
        let mut headless = Headless::new(&mut table, 1200., 600.);

        headless.ctrl_a(&mut table);
        assert_eq!(table.selection, None);
        let (_, copied) = headless.frame(&mut table, vec![egui::Event::Copy], Some(1));
        assert_eq!(copied, None);
    }

    /// Rows of columns `a` and `b`, with the given `(row, column name)` cells in conflict.
    fn conflict_table(conflicts: &[(usize, &str)]) -> Vec<TableRow> {
        (0..4)
            .map(|row| TableRow {
                tag: "x".into(),
                cells: ["a", "b"].map(|name| (name.to_string(), format!("{name}{row}"))).to_vec(),
                styles: conflicts
                    .iter()
                    .filter(|(ix, _)| *ix == row)
                    .map(|(_, name)| (name.to_string(), CellStyle::Conflict))
                    .collect(),
            })
            .collect()
    }

    fn key_press(key: Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]
    }

    fn editing_cell(table: &AttributeTable) -> Option<(usize, usize)> {
        table.editing.as_ref().map(|editing| (editing.source_ix, editing.col_ix))
    }

    /// A table in conflict at row 0 column b, row 2 column a and row 2 column b, resolving.
    fn resolving_table() -> (AttributeTable, Headless) {
        let mut table = AttributeTable::new();
        table.set_rows(&["a", "b"], &conflict_table(&[(0, "b"), (2, "a"), (2, "b")]));
        table.set_editable(&["a", "b"]);
        let mut headless = Headless::new(&mut table, 1200., 600.);
        assert!(table.start_resolving());
        headless.frame(&mut table, Vec::new(), None);
        headless.frame(&mut table, Vec::new(), None);
        (table, headless)
    }

    /// Presses `key` and runs the frame after, which arrives at the next conflict.
    fn press_key(headless: &mut Headless, table: &mut AttributeTable, key: Key, modifiers: egui::Modifiers) {
        headless.frame(table, key_press(key, modifiers), None);
        headless.frame(table, Vec::new(), None);
        headless.frame(table, Vec::new(), None);
    }

    #[test]
    fn tab_and_shift_tab_go_through_the_conflicts_even_unresolved() {
        let (mut table, mut headless) = resolving_table();
        assert_eq!(editing_cell(&table), Some((0, 1)));
        press_key(&mut headless, &mut table, Key::Tab, egui::Modifiers::NONE);
        assert_eq!(editing_cell(&table), Some((2, 0)));
        press_key(&mut headless, &mut table, Key::Tab, egui::Modifiers::NONE);
        assert_eq!(editing_cell(&table), Some((2, 1)));
        press_key(&mut headless, &mut table, Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(editing_cell(&table), Some((2, 0)));
        press_key(&mut headless, &mut table, Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(editing_cell(&table), Some((0, 1)));
        // Around the start, and nothing changed on the way.
        press_key(&mut headless, &mut table, Key::Tab, egui::Modifiers::SHIFT);
        assert_eq!(editing_cell(&table), Some((2, 1)));
        assert!(table.take_edits().is_empty());
    }

    #[test]
    fn escape_stops_resolving_without_changing_the_cell() {
        let (mut table, mut headless) = resolving_table();
        table.editing.as_mut().unwrap().text = "changed".into();
        press_key(&mut headless, &mut table, Key::Escape, egui::Modifiers::NONE);
        assert_eq!(editing_cell(&table), None);
        assert!(table.resolving.is_none());
        assert!(table.take_edits().is_empty());
    }

    #[test]
    fn tab_commits_the_cell_and_goes_on_after_the_table_is_rebuilt() {
        let (mut table, mut headless) = resolving_table();
        table.editing.as_mut().unwrap().text = "b0".to_string() + "x";
        headless.frame(&mut table, key_press(Key::Tab, egui::Modifiers::NONE), None);
        let edits = table.take_edits();
        assert_eq!(edits, [CellEdit { source_ix: 0, column: "b".into(), value: "b0x".into() }]);
        // The owner applies the edit, which resolves that conflict.
        table.set_rows(&["a", "b"], &conflict_table(&[(2, "a"), (2, "b")]));
        headless.frame(&mut table, Vec::new(), None);
        assert_eq!(editing_cell(&table), Some((2, 0)));
    }

    #[test]
    fn resolving_ends_when_no_conflicts_are_left_or_the_table_changes() {
        let (mut table, mut headless) = resolving_table();
        headless.frame(&mut table, key_press(Key::Tab, egui::Modifiers::NONE), None);
        table.set_rows(&["a", "b"], &conflict_table(&[]));
        let (out, _) = headless.frame(&mut table, Vec::new(), None);
        assert!(out.conflicts_resolved);
        assert!(table.resolving.is_none());

        let (mut table, mut headless) = resolving_table();
        table.set_rows(&["a", "b"], &conflict_table(&[(0, "b")]));
        headless.frame(&mut table, Vec::new(), None);
        assert!(table.resolving.is_none());
    }

    #[test]
    fn without_conflicts_resolving_does_not_start() {
        let mut table = AttributeTable::new();
        table.set_rows(&["a", "b"], &conflict_table(&[]));
        table.set_editable(&["a", "b"]);
        assert!(!table.start_resolving());
        assert!(table.resolving.is_none());
    }

    #[test]
    fn conflicts_follow_the_displayed_order() {
        let mut table = AttributeTable::new();
        table.set_rows(&["a", "b"], &conflict_table(&[(0, "a"), (3, "a")]));
        table.set_editable(&["a", "b"]);
        table.set_sort(0, ColumnSort::Descending);
        assert!(table.start_resolving());
        assert_eq!(editing_cell(&table), Some((3, 0)));
    }

    /// What a click at character `cursor` of `text` selects, and what deleting that leaves.
    fn clicked(text: &str, cursor: usize) -> (String, String) {
        let (start, end) = binding_selection(text, cursor);
        let chars: Vec<char> = text.chars().collect();
        (chars[start..end].iter().collect(), chars[..start].iter().chain(&chars[end..]).collect())
    }

    #[test]
    fn a_click_selects_one_binding_with_its_separator() {
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(clicked("kb1_lalt+f8", 4), pair("kb1_lalt+f8", ""));
        assert_eq!(clicked("kb1_x", 0), pair("kb1_x", ""));
        assert_eq!(clicked("kb1_x", 5), pair("kb1_x", ""));

        let two = "kb1_lalt+f8 | kb1_ralt+g";
        assert_eq!(clicked(two, 3), pair("kb1_lalt+f8 | ", "kb1_ralt+g"));
        assert_eq!(clicked(two, 11), pair("kb1_lalt+f8 | ", "kb1_ralt+g"));
        assert_eq!(clicked(two, 18), pair(" | kb1_ralt+g", "kb1_lalt+f8"));
        assert_eq!(clicked(two, 24), pair(" | kb1_ralt+g", "kb1_lalt+f8"));

        let three = "kb1_a | kb1_b | kb1_c";
        assert_eq!(clicked(three, 2), pair("kb1_a | ", "kb1_b | kb1_c"));
        assert_eq!(clicked(three, 10), pair("kb1_b | ", "kb1_a | kb1_c"));
        assert_eq!(clicked(three, 20), pair(" | kb1_c", "kb1_a | kb1_b"));

        let four = "a | b | c | d";
        assert_eq!(clicked(four, 8), pair("c | ", "a | b | d"));
        assert_eq!(clicked("", 0), pair("", ""));
    }

    /// The characters selected in the focused editor, in order.
    fn editor_selection(headless: &Headless) -> (usize, usize) {
        let id = headless.ctx.memory(|memory| memory.focused()).expect("the editor has the focus");
        let state = egui::text_edit::TextEditState::load(&headless.ctx, id).expect("the editor has a state");
        let [start, end] = state.cursor.char_range().expect("a cursor").sorted_cursors();
        (usize::from(start.index), usize::from(end.index))
    }

    fn click_at(headless: &mut Headless, table: &mut AttributeTable, pos: egui::Pos2) {
        headless.frame(table, vec![egui::Event::PointerMoved(pos)], None);
        headless.frame(table, vec![Headless::press(pos, true)], None);
        headless.release(table, pos);
    }

    #[test]
    fn clicking_in_the_cell_editor_selects_what_is_under_the_pointer() {
        let rows = vec![TableRow {
            tag: "a".into(),
            cells: vec![("a".to_string(), "aaaaaaaaaa | bbbbbbbbbb".to_string())],
            styles: Vec::new(),
        }];
        let mut table = AttributeTable::new();
        table.set_rows(&["a"], &rows);
        table.set_editable(&["a"]);
        table.start_edit(0, 0);
        let mut headless = Headless::new(&mut table, 1200., 600.);

        let cell = headless.cell(0, 0);
        let left = headless.header_rects[0].left();
        click_at(&mut headless, &mut table, egui::pos2(left + 12., cell.y));
        assert_eq!(editor_selection(&headless), (0, 13));
        click_at(&mut headless, &mut table, egui::pos2(left + 130., cell.y));
        assert_eq!(editor_selection(&headless), (10, 23));
        // Back to the first, and past the end of the text, which is the last binding too.
        click_at(&mut headless, &mut table, egui::pos2(left + 12., cell.y));
        assert_eq!(editor_selection(&headless), (0, 13));
        click_at(&mut headless, &mut table, egui::pos2(left + 180., cell.y));
        assert_eq!(editor_selection(&headless), (10, 23));
        // The text stays until the user changes it.
        assert_eq!(table.editing.as_ref().unwrap().text, "aaaaaaaaaa | bbbbbbbbbb");
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
