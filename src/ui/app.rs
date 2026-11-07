//! Main window: menu bar, actionmap tree on the left, and on the right one table with the
//! actions of all actionmaps. The tree only navigates the table; selecting in either one
//! selects the same action in the other. Elements outside the actionmaps are not shown but
//! stay in the document, so saving keeps them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ctrlxref::localization::Strings;
use ctrlxref::search::{self, Place};
use ctrlxref::xml_doc;
use ctrlxref::{extract, game_dir, keys};
use eframe::egui::{
    self, Align, Align2, Color32, Context, IconData, Id, Key, Layout, RichText, ScrollArea, Theme, ViewportBuilder,
    ViewportCommand,
};
use egui_commonmark::CommonMarkCache;
use xmltree::{Element, XMLNode};

use super::attribute_table::{self, AttributeTable, CellEdit, CellStyle, TableRow};
use super::help::{self, HelpTopic};
use super::menus::{self, Command, ThemeChoice};
use super::search::{SearchAction, SearchDialog};
use super::theme;

/// Tree id of the document root; descendants append `/<element index>` per level.
const ROOT_ID: &str = "r";

/// Attributes whose value is shown next to the element name in the tree.
const LABEL_ATTRIBUTES: [&str; 4] = ["name", "id", "key", "action"];

/// The only elements listed in the tree; their children are the table rows.
const ACTIONMAP_TAG: &str = "actionmap";

/// Child element of an action in a rebind file; its `input` fills its device's column.
const REBIND_TAG: &str = "rebind";

/// Input devices; an action binds them in an attribute or in a child element of that name.
const DEVICES: [&str; 4] = ["keyboard", "mouse", "joystick", "gamepad"];

/// Read-only column with the profile's own bindings, space separated, so the device
/// columns only hold rebinds and can all be edited.
const DEFAULT_COLUMN: &str = "DEFAULT from: Data.p4k R/O";

/// Separates several bindings shown in one cell.
const BINDING_SEPARATOR: &str = " | ";

/// The table's columns, in this order; other action attributes are not shown.
const COLUMNS: [&str; 10] = [
    "name",
    DEFAULT_COLUMN,
    DEVICES[0],
    DEVICES[1],
    DEVICES[2],
    DEVICES[3],
    "activationMode",
    "UILabel",
    "UIDescription",
    "Category",
];

/// The action attribute that names an element of the profile's `ActionCategories` block.
const CATEGORY_ATTRIBUTE: &str = "Category";

/// Name of the localization file whose texts replace `@key` references in the table.
const STRINGS_FILE: &str = "global.ini";

/// How long a notification stays up; errors stay longer.
const TOAST_TIME: Duration = Duration::from_secs(5);
const ERROR_TOAST_TIME: Duration = Duration::from_secs(9);

/// Rebind inputs by `(actionmap name, action name)`, in file order.
type Rebinds = HashMap<(String, String), Vec<String>>;

/// A tree node: an actionmap with its actions, or an action (no children).
struct TreeNode {
    id: String,
    /// The game's text for the element, else [`readable_name`] of its technical name.
    label: String,
    /// Whether `label` was made from the technical name because the game has no text for it.
    fallback: bool,
    /// The technical name, shown on hover.
    name: String,
    children: Vec<TreeNode>,
}

impl TreeNode {
    fn new(id: String, element: &Element, strings: &Strings, children: Vec<TreeNode>) -> Self {
        let name = element_label(element);
        let localized = localized_label(element, strings).map(str::to_string);
        let fallback = localized.is_none();
        let label = localized.unwrap_or_else(|| readable_name(&name));
        Self { id, label, fallback, name, children }
    }

    /// The label as shown in the tree, in italics when it is not the game's own text.
    fn text(&self) -> RichText {
        let text = RichText::new(&self.label);
        if self.fallback { text.italics() } else { text }
    }
}

struct Document {
    path: PathBuf,
    root: Element,
    element_count: usize,
    /// The actionmaps and their actions, as shown in the tree.
    tree: Vec<TreeNode>,
    actionmap_count: usize,
    /// Element index path of each table row's element, by source index.
    rows: Vec<Vec<usize>>,
    /// From the last opened rebind file. Shown in the table only, never written into `root`.
    rebinds: Rebinds,
    rebind_path: Option<PathBuf>,
    /// The last opened rebind file's tree, which the save actions write. The profile `root`
    /// is exported from the game's p4k package and is never written.
    rebind_root: Option<Element>,
    /// Texts for `@key` references.
    strings: Rc<Strings>,
    /// Actions with a blank keyboard cell in the table, rebinds applied.
    empty_keyboard_rows: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Info,
    Success,
    Warning,
    Error,
}

/// A notification in the window's bottom-right corner.
struct Toast {
    level: Level,
    text: String,
    until: Instant,
}

/// What a dialog's confirm button does.
#[derive(Clone)]
enum DialogAction {
    ExtractDefaults,
    WriteRebindFile,
    Copy(String),
    ShowInExplorer(PathBuf),
    OpenRebindFile(PathBuf),
}

/// A modal message box; with `confirm`, it has that button and a cancel button, and with
/// `extra`, that button between them.
struct Dialog {
    title: String,
    message: String,
    warning: bool,
    /// Lines shown in a scrollable list under the message.
    list: Vec<String>,
    confirm: Option<(&'static str, &'static str, DialogAction)>,
    extra: Option<(&'static str, DialogAction)>,
    /// A phrase drawn like the Default column's bindings wherever it occurs in `message`.
    highlight: Option<&'static str>,
    width: f32,
}

impl Dialog {
    fn warning(title: &str, message: String) -> Self {
        Self {
            title: title.into(),
            message,
            warning: true,
            list: Vec::new(),
            confirm: None,
            extra: None,
            highlight: None,
            width: 480.,
        }
    }

    /// Tells that the file at `path` was saved, offering to show it in File Explorer.
    fn saved(message: String, path: &Path) -> Self {
        let show = DialogAction::ShowInExplorer(path.to_path_buf());
        Self {
            title: "Saved".into(),
            message,
            warning: false,
            list: Vec::new(),
            confirm: Some(("Open in Explorer", "Close", show)),
            extra: None,
            highlight: None,
            width: 480.,
        }
    }

    /// Tells that the rebind file at `path` was created, offering to open it, or to show it
    /// in File Explorer.
    fn created_rebind(message: String, path: &Path) -> Self {
        let show = DialogAction::ShowInExplorer(path.to_path_buf());
        let open = DialogAction::OpenRebindFile(path.to_path_buf());
        Self {
            confirm: Some(("Open Rebind File", "Close", open)),
            extra: Some(("Open in Explorer", show)),
            ..Self::saved(message, path)
        }
    }
}

pub struct CtrlXrefApp {
    ctx: Context,
    table: AttributeTable,
    document: Option<Document>,
    /// The last loaded localization file, reused while profiles find the same one.
    strings_cache: Option<(PathBuf, Rc<Strings>)>,
    /// Tree id of the selected actionmap or action, shared by the tree and the table.
    selected: Option<String>,
    /// Set when the selection changed outside the tree, so the tree reveals it.
    reveal_in_tree: bool,
    /// A displayed table row to scroll into view on the next frame.
    scroll_table_to: Option<usize>,
    /// The Star Citizen installation folder picked with Source > Locate Star Citizen Game Folder,
    /// or saved in the registry by an earlier run.
    game_dir: Option<PathBuf>,
    /// The result of the running Extract Defaults, so it is not started twice.
    extracting: Option<Receiver<anyhow::Result<()>>>,
    /// The startup profile argument while it waits for a game folder or an extraction
    /// (`Some(None)`: the default profile); `None` once it has been opened.
    startup_profile: Option<Option<PathBuf>>,
    /// The startup rebind file, opened after the startup profile.
    startup_rebind: Option<PathBuf>,
    /// Whether the first frame has run, which opens the startup profile.
    started: bool,
    theme: ThemeChoice,
    /// The open help windows.
    help_open: Vec<HelpTopic>,
    help_cache: CommonMarkCache,
    icon: Option<Arc<IconData>>,
    toasts: Vec<Toast>,
    /// Message boxes, shown one at a time, oldest first.
    dialogs: Vec<Dialog>,
    search: SearchDialog,
}

impl CtrlXrefApp {
    pub fn new(
        initial_file: Option<PathBuf>,
        initial_rebind_file: Option<PathBuf>,
        icon: Option<Arc<IconData>>,
        ctx: &Context,
    ) -> Self {
        let theme = ThemeChoice::System;
        ctx.set_theme(menus::preference(theme));
        theme::apply(ctx);
        let mut table = AttributeTable::new();
        // Only the bindings can be edited; the edits go to the rebind file.
        table.set_editable(&DEVICES);
        table.set_exact_titles(&[DEFAULT_COLUMN]);
        Self {
            ctx: ctx.clone(),
            table,
            document: None,
            strings_cache: None,
            selected: None,
            reveal_in_tree: false,
            scroll_table_to: None,
            game_dir: game_dir::load_saved(),
            extracting: None,
            startup_profile: Some(initial_file),
            startup_rebind: initial_rebind_file,
            started: false,
            theme,
            help_open: Vec::new(),
            help_cache: CommonMarkCache::default(),
            icon,
            toasts: Vec::new(),
            dialogs: Vec::new(),
            search: SearchDialog::new(),
        }
    }

    fn notify(&mut self, level: Level, text: impl Into<String>) {
        let time = if level == Level::Error { ERROR_TOAST_TIME } else { TOAST_TIME };
        self.toasts.push(Toast { level, text: text.into(), until: Instant::now() + time });
        self.ctx.request_repaint();
    }

    /// Shows the open rebind file in the window title, or the profile while no rebind file
    /// is open.
    fn update_title(&self) {
        let title = match &self.document {
            Some(doc) => {
                let shown = doc.rebind_path.as_deref().unwrap_or(&doc.path);
                format!("ctrlXref - {}", shown.display())
            }
            None => "ctrlXref".to_string(),
        };
        self.ctx.send_viewport_cmd(ViewportCommand::Title(title));
    }

    fn run(&mut self, command: Command, frame: &eframe::Frame) {
        match command {
            Command::OpenFile => self.open_file(frame),
            Command::OpenCurrentRebindFile => self.open_current_rebind_file(),
            Command::OpenCustomRebindFile => self.open_custom_rebind_file(frame),
            Command::SaveRebindFile => self.save_rebind_file(),
            Command::SaveRebindCopyAs => self.save_rebind_copy_as(frame),
            Command::CreateBlankRebindFile => self.create_blank_rebind_file(frame),
            Command::CloseFile => self.close_file(),
            Command::Quit => self.ctx.send_viewport_cmd(ViewportCommand::Close),
            Command::Theme(choice) => self.set_theme(choice),
            Command::ToggleTheme => self.toggle_theme(),
            Command::LocateGameFolder => self.locate_game_folder(frame),
            Command::ExtractDefaults => self.extract_defaults(),
            Command::ShowUnusedCombos => self.show_unused_combos(),
            Command::AutofillKeyboard => self.autofill_keyboard(),
            Command::ExportCsv => self.export_csv(frame),
            Command::ExportJson => self.export_json(frame),
            Command::Find => self.open_search(),
            Command::FindNext => self.find_with_key(true),
            Command::FindPrevious => self.find_with_key(false),
            Command::Help(topic) => self.show_help(topic),
            Command::About => self.about(),
        }
    }

    /// Opens the startup profile, relative to the game folder, and then the startup rebind
    /// file. Without a game folder, or before `Data.p4k` has been extracted, it asks for that
    /// instead and keeps waiting; locating the folder or extracting calls this again.
    fn open_startup_profile(&mut self) {
        let Some(arg) = self.startup_profile.clone() else {
            return;
        };
        let path = match &arg {
            Some(absolute) if absolute.is_absolute() => absolute.clone(),
            _ => {
                let Some(dir) = self.game_dir.clone().filter(|dir| game_dir::is_game_dir(dir)) else {
                    self.dialogs.push(no_game_dir_warning());
                    return;
                };
                if !game_dir::is_extracted(&dir) {
                    self.ask_to_extract(&dir);
                    return;
                }
                game_dir::profile_path(&dir, arg.as_deref())
            }
        };
        self.startup_profile = None;
        self.open_path(path);
        if let Some(path) = self.startup_rebind.take()
            && self.document.is_some()
        {
            self.open_rebind_path(path);
        }
    }

    /// Asks to extract `Data.p4k` into the game folder `dir`, which has no `Data` folder yet,
    /// so the startup profile can be opened; or says which extraction tools are missing.
    fn ask_to_extract(&mut self, dir: &Path) {
        let missing = game_dir::missing_extraction_tools(dir);
        if !missing.is_empty() {
            self.dialogs.push(missing_tools_warning(dir, &missing));
            return;
        }
        let message = format!(
            "The Data folder is missing from the Star Citizen folder:\n{}\n\
             Extract Data.p4k to proceed?",
            dir.display()
        );
        let mut dialog = Dialog::warning("Data not extracted", message);
        dialog.confirm = Some(("Extract", "Cancel", DialogAction::ExtractDefaults));
        self.dialogs.push(dialog);
    }

    fn open_path(&mut self, path: PathBuf) {
        let root = match xml_doc::load(&path) {
            Ok(root) => root,
            Err(err) => {
                self.notify(Level::Error, format!("{err:#}"));
                return;
            }
        };

        let maps = find_actionmaps(&root);
        let rows = action_paths(&root, &maps);
        let strings_path = find_strings_file(&path, self.game_dir.as_deref());
        let strings: Rc<Strings> = match &strings_path {
            Some(strings_path) => self.load_strings(strings_path),
            None => Rc::default(),
        };
        let tree = actionmap_tree(&root, &maps, &strings);
        self.document = Some(Document {
            element_count: count_elements(&root),
            tree,
            actionmap_count: maps.len(),
            rows,
            rebinds: Rebinds::new(),
            rebind_path: None,
            rebind_root: None,
            strings,
            empty_keyboard_rows: 0,
            path,
            root,
        });
        self.selected = None;
        self.refresh_table();
        self.update_title();
        if self.document.as_ref().is_some_and(|doc| doc.actionmap_count == 0) {
            self.notify(Level::Warning, "The file has no actionmap elements");
        }
    }

    /// The localization texts in `path`, from the cache when it holds that file.
    fn load_strings(&mut self, path: &Path) -> Rc<Strings> {
        if let Some((cached_path, strings)) = &self.strings_cache
            && cached_path == path
        {
            return strings.clone();
        }
        match Strings::load(path) {
            Ok(strings) => {
                let strings = Rc::new(strings);
                self.strings_cache = Some((path.to_path_buf(), strings.clone()));
                strings
            }
            Err(err) => {
                self.notify(Level::Error, format!("{err:#}"));
                Rc::default()
            }
        }
    }

    /// Loads the rebind file at `path` and shows its inputs in the table's rebind column,
    /// matched to the open profile's actions by actionmap and action name.
    fn open_rebind_path(&mut self, path: PathBuf) {
        // Empty rebinds like `kb1_ ` stay: in the game they unbind that device's default.
        let root = match xml_doc::load(&path) {
            Ok(root) => root,
            Err(err) => {
                self.notify(Level::Error, format!("{err:#}"));
                return;
            }
        };
        let Some(doc) = &mut self.document else {
            return;
        };
        let rebinds = read_rebinds(&root);
        let actions: HashSet<(String, String)> =
            doc.rows.iter().filter_map(|path| action_key(&doc.root, path)).collect();
        let unmatched: Vec<String> = rebinds
            .keys()
            .filter(|key| !actions.contains(*key))
            .map(|(map, action)| format!("{map}/{action}"))
            .collect();
        let note = format!(
            "{} of {} rebinds matched from {}",
            rebinds.len() - unmatched.len(),
            rebinds.len(),
            path.display()
        );
        doc.rebinds = rebinds;
        doc.rebind_path = Some(path);
        doc.rebind_root = Some(root);
        self.refresh_table();
        self.update_title();

        if unmatched.is_empty() {
            self.notify(Level::Success, note);
        } else {
            let mut unmatched = unmatched;
            unmatched.sort();
            let more = unmatched.len().saturating_sub(5);
            unmatched.truncate(5);
            let mut list = unmatched.join(", ");
            if more > 0 {
                list.push_str(&format!(" and {more} more"));
            }
            self.notify(Level::Warning, format!("{note}. Not in the profile: {list}"));
        }
    }

    /// Rebuilds the table from the document. The selection is kept, since it is by tree id.
    fn refresh_table(&mut self) {
        let rows = match &self.document {
            Some(doc) => table_rows(&doc.root, &doc.rows, &doc.rebinds, &doc.strings),
            None => Vec::new(),
        };
        if let Some(doc) = &mut self.document {
            doc.empty_keyboard_rows = count_empty_keyboard(&doc.root, &doc.rows, &doc.rebinds);
        }
        self.table.set_rows(&COLUMNS, &rows);
        self.update_search_counts();
    }

    /// Everything the search can stop at, in its order, with the matches of its regular
    /// expression; nothing without one.
    fn search_places(&self) -> Vec<Place> {
        match (&self.document, self.search.regex()) {
            (Some(doc), Some(regex)) => search_places(&doc.tree, &doc.rows, self.table.displayed_rows(), regex),
            _ => Vec::new(),
        }
    }

    fn update_search_counts(&mut self) {
        let places = self.search_places();
        self.search.set_counts(&places);
    }

    /// Opens the Search dialog with the keyboard in its text field. Not over a message box.
    fn open_search(&mut self) {
        if self.dialogs.is_empty() && self.search.open(true) {
            self.update_search_counts();
        }
    }

    /// F3 and Shift+F3 open the Search dialog when it is closed, and search.
    fn find_with_key(&mut self, forward: bool) {
        if !self.dialogs.is_empty() {
            return;
        }
        if self.search.open(false) {
            self.update_search_counts();
        }
        if self.search.is_blank() {
            self.search.open(true);
        } else {
            self.find(forward);
        }
    }

    /// Moves to the next or the previous match, going around the ends of the table and the
    /// tree. A search that finds something joins the previous ones. The status bar tells how
    /// it went.
    fn find(&mut self, forward: bool) {
        if self.search.regex().is_none() {
            return;
        }
        let places = self.search_places();
        self.search.set_counts(&places);
        let found = search::find(&places, self.selected.as_deref(), forward);
        self.search.set_found(found);
        let Some(place) = found.map(|found| &places[found.place]) else {
            return;
        };
        if let Err(err) = self.search.remember() {
            self.notify(Level::Error, format!("{err:#}"));
        }
        match place.row {
            Some(row) => {
                self.table.clear_selection();
                self.selected = Some(place.id.clone());
                self.scroll_table_to = Some(row);
            }
            None => self.select_in_tree(place.id.clone()),
        }
        self.reveal_in_tree = true;
    }

    /// Draws the Search dialog and does what was asked in it.
    fn search_dialog(&mut self, ctx: &Context) {
        match self.search.show(ctx) {
            Some(SearchAction::Changed) => self.update_search_counts(),
            Some(SearchAction::FindNext) => self.find(true),
            Some(SearchAction::FindPrevious) => self.find(false),
            None => {}
        }
    }

    /// Source index (into `Document::rows`) of the action with tree id `id`.
    fn source_of_id(&self, id: &str) -> Option<usize> {
        let path = parse_tree_id(id)?;
        self.document.as_ref()?.rows.iter().position(|row| *row == path)
    }

    /// The displayed table row of the selected action.
    fn selected_row(&self) -> Option<usize> {
        let source_ix = self.source_of_id(self.selected.as_deref()?)?;
        self.table.row_of(source_ix)
    }

    /// Tree selection changed to `id`: scroll the table to the action's row, or for an
    /// actionmap to its first row in the current sort order.
    fn select_in_tree(&mut self, id: String) {
        let Some(doc) = &self.document else {
            return;
        };
        let map_path = parse_tree_id(&id).unwrap_or_default();
        let first_row = match self.source_of_id(&id) {
            Some(source_ix) => self.table.row_of(source_ix),
            None => doc
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.starts_with(&map_path))
                .filter_map(|(ix, _)| self.table.row_of(ix))
                .min(),
        };
        self.selected = Some(id);
        self.scroll_table_to = first_row;
        self.table.clear_selection();
    }

    /// Table selection moved to displayed `row_ix`: select and reveal its action in the tree.
    fn select_in_table(&mut self, row_ix: usize) {
        let Some(doc) = &self.document else {
            return;
        };
        let Some(path) = self.table.source_of(row_ix).and_then(|ix| doc.rows.get(ix)) else {
            return;
        };
        self.selected = Some(tree_id(path));
        self.reveal_in_tree = true;
    }

    /// Up and Down move the table selection, while no text field has the keyboard.
    fn arrow_keys(&mut self, ctx: &Context) {
        if ctx.egui_wants_keyboard_input() || self.table.row_count() == 0 {
            return;
        }
        let (up, down) = ctx.input(|input| (input.key_pressed(Key::ArrowUp), input.key_pressed(Key::ArrowDown)));
        if !up && !down {
            return;
        }
        let last = self.table.row_count() - 1;
        let row = match (self.selected_row(), up) {
            (Some(row), true) => row.saturating_sub(1),
            (Some(row), false) => (row + 1).min(last),
            (None, _) => 0,
        };
        self.table.clear_selection();
        self.select_in_table(row);
        self.scroll_table_to = Some(row);
    }

    /// Writes the binding cells edited in the table into the open rebind file. The profile is
    /// never changed; its (light blue) bindings are shown in the read-only Default column.
    fn apply_cell_edits(&mut self) {
        let edits = self.table.take_edits();
        if edits.is_empty() {
            return;
        }
        for edit in edits {
            self.apply_cell_edit(edit);
        }
        self.refresh_table();
    }

    fn apply_cell_edit(&mut self, edit: CellEdit) {
        let Some(device) = DEVICES.iter().copied().find(|device| edit.column == *device) else {
            return;
        };
        let Some(doc) = &mut self.document else {
            return;
        };
        let Some(rebind_root) = &mut doc.rebind_root else {
            self.notify(
                Level::Warning,
                "Open a rebind file first (File > Open Current Rebind File or Open Custom Rebind File...); \
                 edited bindings are written to it, never to defaultProfile.xml",
            );
            return;
        };
        let inputs = match parse_cell_inputs(device, &edit.value) {
            Ok(inputs) => inputs,
            Err(message) => {
                self.notify(Level::Warning, message);
                return;
            }
        };
        let Some((map, action)) = doc.rows.get(edit.source_ix).and_then(|path| action_key(&doc.root, path)) else {
            return;
        };
        set_device_rebinds(rebind_root, &map, &action, device, &inputs);
        doc.rebinds = read_rebinds(rebind_root);
    }

    /// Asks for a profile, suggesting the game's extracted [`game_dir::DEFAULT_PROFILE`].
    /// Starts in the game folder rather than whichever folder Windows used last, such as
    /// the rebind mappings folder.
    fn open_file(&mut self, frame: &eframe::Frame) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open defaultProfile")
            .add_filter("XML files", &["xml"])
            .add_filter("All files", &["*"])
            .set_parent(frame);
        if let Some(dir) = self.valid_game_dir() {
            let profile = dir.join(game_dir::DEFAULT_PROFILE);
            match profile.parent().filter(|config| config.is_dir()) {
                Some(config) => {
                    dialog = dialog.set_directory(config).set_file_name("defaultProfile.xml");
                }
                None => dialog = dialog.set_directory(dir),
            }
        }
        if let Some(path) = dialog.pick_file() {
            // A file picked by hand replaces a startup profile still waiting to open.
            self.startup_profile = None;
            self.startup_rebind = None;
            self.open_path(path);
        }
    }

    /// Whether a profile is open, which rebind files are matched against; warns when not.
    fn has_profile(&mut self) -> bool {
        if self.document.is_none() {
            self.notify(Level::Warning, "Open the profile first (File > Open defaultProfile), then its rebind file");
        }
        self.document.is_some()
    }

    /// The saved game folder, when it is a valid one.
    fn valid_game_dir(&self) -> Option<PathBuf> {
        self.game_dir.clone().filter(|dir| game_dir::is_game_dir(dir))
    }

    /// Opens [`game_dir::CURRENT_REBIND`], the rebind file the game last used.
    fn open_current_rebind_file(&mut self) {
        if !self.has_profile() {
            return;
        }
        let Some(dir) = self.valid_game_dir() else {
            self.dialogs.push(no_game_dir_warning());
            return;
        };
        self.open_rebind_path(dir.join(game_dir::CURRENT_REBIND));
    }

    /// Asks for a rebind file, starting in the game's [`game_dir::MAPPINGS_DIR`] where it
    /// saves the user's mappings.
    fn open_custom_rebind_file(&mut self, frame: &eframe::Frame) {
        if !self.has_profile() {
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open Custom Rebind File")
            .add_filter("XML files", &["xml"])
            .add_filter("All files", &["*"])
            .set_parent(frame);
        let start = self.valid_game_dir().map(|dir| {
            let mappings = dir.join(game_dir::MAPPINGS_DIR);
            if mappings.is_dir() { mappings } else { dir }
        });
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        if let Some(path) = dialog.pick_file() {
            self.open_rebind_path(path);
        }
    }

    /// The open rebind file's path, or a warning when there is none.
    fn rebind_path(&mut self) -> Option<PathBuf> {
        let path = self
            .document
            .as_ref()
            .filter(|doc| doc.rebind_root.is_some())
            .and_then(|doc| doc.rebind_path.clone());
        if path.is_none() {
            self.notify(
                Level::Warning,
                "No rebind file is open (File > Open Current Rebind File or Open Custom Rebind File...)",
            );
        }
        path
    }

    /// Saves the rebind file where it was opened from. The game's own
    /// [`game_dir::CURRENT_REBIND`] gets a warning with Cancel first, since the game rewrites
    /// it whenever controls are changed in the game.
    fn save_rebind_file(&mut self) {
        let Some(path) = self.rebind_path() else {
            return;
        };
        if !is_current_rebind(&path) {
            self.write_rebind_file(&path);
            return;
        }
        let message = format!(
            "Saving your current actionmaps.xml:\n{}\n\n\
             It will be overwritten by the game engine when you modify controls inside the game.",
            path.display()
        );
        let mut dialog = Dialog::warning("Warning", message);
        dialog.confirm = Some(("Save", "Cancel", DialogAction::WriteRebindFile));
        self.dialogs.push(dialog);
    }

    /// Writes the open rebind file to `path`, telling how that went.
    fn write_rebind_file(&mut self, path: &Path) {
        let Some(root) = self.document.as_ref().and_then(|doc| doc.rebind_root.as_ref()) else {
            return;
        };
        match xml_doc::save(root, path) {
            Ok(()) => self.dialogs.push(Dialog::saved(format!("Saved {}", path.display()), path)),
            Err(err) => self.notify(Level::Error, format!("{err:#}")),
        }
    }

    /// Writes the rebind file to a new path, as a backup or a copy to edit later. The open
    /// rebind file stays the one Save Rebind File writes.
    fn save_rebind_copy_as(&mut self, frame: &eframe::Frame) {
        let Some(rebind_path) = self.rebind_path() else {
            return;
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save Rebind Copy As")
            .add_filter("XML files", &["xml"])
            .add_filter("All files", &["*"])
            .set_parent(frame);
        if let Some(dir) = rebind_path.parent() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(stem) = rebind_path.file_stem() {
            dialog = dialog.set_file_name(format!("{}_copy.xml", stem.to_string_lossy()));
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        let Some(root) = self.document.as_ref().and_then(|doc| doc.rebind_root.as_ref()) else {
            return;
        };
        match xml_doc::save(root, &path) {
            Ok(()) => self.dialogs.push(Dialog::saved(format!("Saved copy to {}", path.display()), &path)),
            Err(err) => self.notify(Level::Error, format!("{err:#}")),
        }
    }

    /// Writes a rebind file that clears every binding of the open profile: the game's
    /// [`game_dir::BLANK_LAYOUT`] with its header and devices kept, made to hold exactly
    /// the profile's actions (see [`complete_blank_layout`]). The file is saved, then a
    /// dialog offers to open it.
    fn create_blank_rebind_file(&mut self, frame: &eframe::Frame) {
        let Some(doc) = &self.document else {
            self.notify(Level::Warning, "Open the profile first (File > Open defaultProfile)");
            return;
        };
        let file_name = Path::new(game_dir::BLANK_LAYOUT).file_name().expect("a file name");
        // The layout extracted with the open profile, else the game folder's.
        let next_to_profile = doc.path.parent().map(|config| config.join("Mappings").join(file_name));
        let in_game_dir = self.valid_game_dir().map(|dir| dir.join(game_dir::BLANK_LAYOUT));
        let Some(template_path) = next_to_profile.into_iter().chain(in_game_dir).find(|path| path.is_file()) else {
            self.notify(
                Level::Warning,
                format!(
                    "{} not found next to the profile or in the game folder; \
                     extract it with Source > Extract Defaults from Data.p4k Package",
                    file_name.to_string_lossy()
                ),
            );
            return;
        };
        let mut root = match xml_doc::load(&template_path) {
            Ok(root) => root,
            Err(err) => {
                self.notify(Level::Error, format!("{err:#}"));
                return;
            }
        };
        let actions: Vec<(String, String)> =
            doc.rows.iter().filter_map(|path| action_key(&doc.root, path)).collect();
        let (added, removed) = complete_blank_layout(&mut root, &actions);

        let mut dialog = rfd::FileDialog::new()
            .set_title("Create Blank Rebind File")
            .add_filter("XML files", &["xml"])
            .add_filter("All files", &["*"])
            .set_file_name("layout_all_blank_complete.xml")
            .set_parent(frame);
        let mappings = self
            .valid_game_dir()
            .map(|dir| dir.join(game_dir::MAPPINGS_DIR))
            .filter(|dir| dir.is_dir());
        if let Some(dir) = mappings {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        match xml_doc::save(&root, &path) {
            Ok(()) => {
                let mut message = format!(
                    "Created {} from {}: added {added} actions from the profile, \
                     removed {removed} not in it.",
                    path.display(),
                    template_path.display()
                );
                if self.document.as_ref().is_some_and(|doc| doc.rebind_root.is_some()) {
                    message.push_str(
                        "\n\nOpening it replaces the open rebind file; changes not saved are lost.",
                    );
                }
                self.dialogs.push(Dialog::created_rebind(message, &path));
            }
            Err(err) => self.notify(Level::Error, format!("{err:#}")),
        }
    }

    /// Exports the table, in its current sort order and with each action's actionmap name
    /// in a first column, to a CSV file picked with a save dialog. Fields are separated by
    /// the system list separator, so spreadsheets of the user's locale split them.
    fn export_csv(&mut self, frame: &eframe::Frame) {
        let Some(doc) = &self.document else {
            self.notify(Level::Warning, "Open the profile first (File > Open defaultProfile)");
            return;
        };
        let csv = {
            let header = std::iter::once("actionmap".to_string()).chain(self.table.headers());
            let rows = self.table.displayed_rows().map(|(source_ix, cells)| {
                let map = doc
                    .rows
                    .get(source_ix)
                    .and_then(|path| action_key(&doc.root, path))
                    .map(|(map, _)| map)
                    .unwrap_or_default();
                std::iter::once(map).chain(cells.iter().cloned()).collect::<Vec<_>>()
            });
            ctrlxref::csv::to_csv(std::iter::once(header.collect()).chain(rows), ctrlxref::csv::list_separator())
        };
        let source = doc.rebind_path.as_deref().unwrap_or(&doc.path);
        let Some(path) = self.export_dialog(source, "CSV files", "csv", frame).save_file() else {
            return;
        };
        match ctrlxref::csv::save(&csv, &path) {
            Ok(()) => self.dialogs.push(Dialog::saved(format!("Exported {}", path.display()), &path)),
            Err(err) => self.notify(Level::Error, format!("{err:#}")),
        }
    }

    /// Exports the actionmaps and their actions, nested and in document order like the tree
    /// whatever the table's sort, to a JSON file picked with a save dialog.
    fn export_json(&mut self, frame: &eframe::Frame) {
        let Some(doc) = &self.document else {
            self.notify(Level::Warning, "Open the profile first (File > Open defaultProfile)");
            return;
        };
        let json = tree_json(&doc.root, &doc.rows, &doc.strings, &self.table.headers(), self.table.document_rows());
        let source = doc.rebind_path.as_deref().unwrap_or(&doc.path);
        let Some(path) = self.export_dialog(source, "JSON files", "json", frame).save_file() else {
            return;
        };
        match ctrlxref::json::save(&json, &path) {
            Ok(()) => self.dialogs.push(Dialog::saved(format!("Exported {}", path.display()), &path)),
            Err(err) => self.notify(Level::Error, format!("{err:#}")),
        }
    }

    /// The save dialog of an export to a `.<extension>` file, suggesting the name of `source`.
    fn export_dialog(&self, source: &Path, filter: &str, extension: &str, frame: &eframe::Frame) -> rfd::FileDialog {
        let mut dialog = rfd::FileDialog::new()
            .set_title(format!("Export to .{extension}"))
            .add_filter(filter, &[extension])
            .add_filter("All files", &["*"])
            .set_parent(frame);
        // The game's mappings folder, where its exported mappings are too; else next to the file.
        let mappings = self
            .valid_game_dir()
            .map(|dir| dir.join(game_dir::MAPPINGS_DIR))
            .filter(|dir| dir.is_dir());
        if let Some(dir) = mappings.as_deref().or(source.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(stem) = source.file_stem() {
            dialog = dialog.set_file_name(format!("{}.{extension}", stem.to_string_lossy()));
        }
        dialog
    }

    fn close_file(&mut self) {
        self.document = None;
        self.selected = None;
        self.refresh_table();
        self.update_title();
    }

    fn set_theme(&mut self, theme: ThemeChoice) {
        self.theme = theme;
        self.ctx.set_theme(menus::preference(theme));
    }

    fn toggle_theme(&mut self) {
        let next = match self.ctx.theme() {
            Theme::Dark => ThemeChoice::Light,
            Theme::Light => ThemeChoice::Dark,
        };
        self.set_theme(next);
    }

    /// Asks for the game folder with the native folder browser, and saves it for the next run
    /// when it holds the game's `LIVE\Data.p4k`.
    fn locate_game_folder(&mut self, frame: &eframe::Frame) {
        let start = self
            .game_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from(game_dir::DEFAULT));
        let Some(path) = rfd::FileDialog::new()
            .set_title("Locate Star Citizen Game Folder")
            .set_directory(start)
            .set_parent(frame)
            .pick_folder()
        else {
            return;
        };
        if !game_dir::is_game_dir(&path) {
            self.dialogs.push(invalid_game_dir_warning(&path));
        } else {
            match game_dir::save(&path) {
                Ok(()) => self.notify(Level::Success, format!("Game folder: {}", path.display())),
                Err(err) => self.notify(Level::Error, format!("{err:#}")),
            }
            let missing = game_dir::missing_extraction_tools(&path);
            if !missing.is_empty() {
                self.dialogs.push(missing_tools_warning(&path, &missing));
            }
        }
        self.game_dir = Some(path);
        self.open_startup_profile();
    }

    /// Runs the [`extract::steps`] in the game folder on a background thread.
    fn extract_defaults(&mut self) {
        let Some(dir) = self.game_dir.clone().filter(|dir| can_extract(Some(dir))) else {
            let dir = self.game_dir.clone().unwrap_or_default();
            let dialog = if game_dir::is_game_dir(&dir) {
                missing_tools_warning(&dir, &game_dir::missing_extraction_tools(&dir))
            } else {
                invalid_game_dir_warning(&dir)
            };
            self.dialogs.push(dialog);
            return;
        };
        if self.extracting.is_some() {
            self.notify(Level::Info, "Extraction is already running");
            return;
        }
        self.notify(Level::Info, "Extracting defaults from Data.p4k...");
        let (sender, receiver) = mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = extract::run_all(&extract::steps(&dir), &dir);
            sender.send(result).ok();
            ctx.request_repaint();
        });
        self.extracting = Some(receiver);
    }

    /// Finishes Extract Defaults once its thread is done.
    fn poll_extraction(&mut self) {
        let Some(receiver) = &self.extracting else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!("The extraction stopped unexpectedly")),
        };
        self.extracting = None;
        match result {
            Ok(()) => {
                self.notify(Level::Success, "Extracted defaults from Data.p4k");
                self.open_startup_profile();
            }
            Err(err) => self.notify(Level::Error, format!("{err:#}")),
        }
    }

    /// The key combinations no binding uses, in [`keys::unused_combos`] order, and the
    /// files whose bindings were taken into account: the open profile, the open rebind file,
    /// and the game's current rebind file when that is not the open one.
    fn unused_combos(&mut self) -> (Vec<keys::Combo>, Vec<String>) {
        let current = self.valid_game_dir().map(|dir| dir.join(game_dir::CURRENT_REBIND));
        let Some(doc) = &self.document else {
            return (Vec::new(), Vec::new());
        };
        let mut inputs = keys::profile_keyboard_inputs(&doc.root);
        let mut sources = vec![doc.path.display().to_string()];
        if let (Some(root), Some(path)) = (&doc.rebind_root, &doc.rebind_path) {
            inputs.extend(keys::rebind_keyboard_inputs(root));
            sources.push(path.display().to_string());
        }
        let mut error = None;
        if let Some(current) = current.filter(|path| path.is_file() && Some(path) != doc.rebind_path.as_ref()) {
            match xml_doc::load(&current) {
                Ok(root) => {
                    inputs.extend(keys::rebind_keyboard_inputs(&root));
                    sources.push(current.display().to_string());
                }
                Err(err) => error = Some(format!("{err:#}")),
            }
        }
        if let Some(error) = error {
            self.notify(Level::Error, error);
        }
        let bound = keys::bound_combos(inputs.iter().map(String::as_str));
        (keys::unused_combos(&bound), sources)
    }

    /// Gives every action without a keyboard binding the next unused key combination,
    /// in table order, as a `kb1_<modifier>+<key>` rebind added to the open rebind file.
    fn autofill_keyboard(&mut self) {
        if !self.has_profile() {
            return;
        }
        if self.document.as_ref().is_some_and(|doc| doc.rebind_root.is_none()) {
            self.notify(
                Level::Warning,
                "Open a rebind file first (File > Open Current Rebind File or Open Custom Rebind File...); \
                 the new bindings are added to it",
            );
            return;
        }
        let (unused, _) = self.unused_combos();
        let Some(doc) = &mut self.document else {
            return;
        };
        let unbound = unbound_keyboard_actions(&doc.root, &doc.rows, &doc.rebinds);
        let Some(rebind_root) = &mut doc.rebind_root else {
            return;
        };
        let filled = unbound.len().min(unused.len());
        for ((map, action), combo) in unbound.iter().zip(&unused) {
            set_keyboard_rebind(rebind_root, map, action, &format!("kb1_{combo}"));
        }
        doc.rebinds = read_rebinds(rebind_root);
        self.refresh_table();

        let mut note = format!(
            "Filled {filled} of {} actions without a keyboard binding",
            unbound.len()
        );
        if filled < unbound.len() {
            note.push_str(&format!("; {} are left, out of unused combinations", unbound.len() - filled));
        }
        note.push_str(". Save the rebind file to keep them");
        let level = if filled == 0 && !unbound.is_empty() { Level::Warning } else { Level::Success };
        self.notify(level, note);
    }

    /// Lists the key combinations no binding uses: from the open profile, the open
    /// rebind file, and the game's current rebind file when that is not the open one.
    fn show_unused_combos(&mut self) {
        if !self.has_profile() {
            return;
        }
        let (unused, sources) = self.unused_combos();
        let candidates = keys::candidates().len();
        let two_key = unused.iter().filter(|combo| combo.second.is_none()).count();
        let summary = format!(
            "{} of {} key combinations are unused: {} two-key and {} three-key ({} in use). \
             Windows ones such as Alt+F4, Alt+Tab, Ctrl+Esc, Ctrl+Shift+Esc and Ctrl+Alt+Del \
             are left out. Modifiers: {}; three-key combinations pair two different kinds.\n\
             Bindings from:\n{}",
            unused.len(),
            candidates,
            two_key,
            unused.len() - two_key,
            candidates - unused.len(),
            keys::MODIFIERS.join(", "),
            sources.join("\n"),
        );
        let list = keys::format_by_modifier(&unused);
        let copy = format!("{summary}\n\n{list}");
        self.dialogs.push(Dialog {
            title: "Unused Key Combinations".into(),
            message: summary,
            warning: false,
            list: list.lines().map(str::to_string).collect(),
            confirm: Some(("Copy", "Close", DialogAction::Copy(copy))),
            extra: None,
            highlight: None,
            width: 760.,
        });
    }

    fn about(&mut self) {
        self.dialogs.push(Dialog {
            title: "About ctrlXref".into(),
            message: format!(
                "ctrlXref {}\n\n\
                 ctrlXref is a simple tool aiming to help in resolving missing, conflicting or unintended control bindings \
                 of the Cloud Imperium Games / Roberts Space Industries Star Citizen (Alpha) game.\n\n\
                 Various applications and devices, such as Stream Decks, Game Glass, iCUE etc. send \
                 keyboard (and possibly also mouse and joystick) events to the game engine. \
                 Resolving missing or overlapping controls with the game's integrated controls \
                 configurator has always been quite messy and confusing. This tool \
                 shows the game's default control bindings and the user's modified (rebind) \
                 settings together in one tree and freely sortable table view.\n\n\
                 With the external unp4k tools, ctrlXref can extract the default settings from the \
                 current game installation, so the default profile can be updated every time the \
                 game release is updated.\n\n\
                 It can also sequentially bind every missing keyboard controls automatically, for devices and \
                 applications whose plugins read the bound keys straight from the game engine or its logs. \
                 These automatic key bindings are not intended to be used through a real keyboard device \
                 as they are not any way ergonomic or thought after instead just picked from the \
                 unused key combination list. NOTE: The huge number of controls inside the game \
                 (1103 as the time of writing) forces the use of some three key combinations also.\n\n\
                 Editing / modifying the bindings is supported but not really refined as this tool \
                 is not intended to replace the game internal control modification features and \
                 other already existing 3rd party applications already handling the task better.",
                env!("CTRLXREF_VERSION")
            ),
            warning: false,
            list: Vec::new(),
            confirm: None,
            extra: None,
            highlight: Some("Star Citizen"),
            width: 560.,
        });
    }

    /// Shows the help page `topic` in its own window, bringing that window forward when it
    /// is already open.
    fn show_help(&mut self, topic: HelpTopic) {
        if self.help_open.contains(&topic) {
            self.ctx.send_viewport_cmd_to(topic.viewport_id(), ViewportCommand::Focus);
        } else {
            self.help_open.push(topic);
        }
    }

    fn run_dialog_action(&mut self, action: DialogAction) {
        match action {
            DialogAction::ExtractDefaults => self.extract_defaults(),
            DialogAction::WriteRebindFile => {
                if let Some(path) = self.rebind_path() {
                    self.write_rebind_file(&path);
                }
            }
            DialogAction::Copy(text) => self.ctx.copy_text(text),
            DialogAction::ShowInExplorer(path) => {
                if let Err(err) = show_in_explorer(&path) {
                    self.notify(Level::Error, format!("Could not open File Explorer: {err}"));
                }
            }
            DialogAction::OpenRebindFile(path) => {
                if self.has_profile() {
                    self.open_rebind_path(path);
                }
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut picked = None;
        egui::MenuBar::new().ui(ui, |ui| {
            picked = menus::menu_items(ui, self.theme, can_extract(self.game_dir.as_deref()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let icon = if ui.visuals().dark_mode { "☀" } else { "🌙" };
                if ui.button(icon).on_hover_text("Toggle light / dark theme").clicked() {
                    picked = Some(Command::ToggleTheme);
                }
            });
        });
        picked
    }

    fn tree_panel(&mut self, ui: &mut egui::Ui) {
        let title = match &self.document {
            Some(doc) => format!("Actionmaps  -  {}", doc.actionmap_count),
            None => "Actionmaps".to_string(),
        };
        panel_header(ui, title);
        let Some(doc) = &self.document else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("File > Open defaultProfile (Ctrl+O) to load defaultProfile.xml").weak());
            });
            return;
        };
        let reveal = std::mem::take(&mut self.reveal_in_tree);
        let mut clicked = None;
        ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            for map in &doc.tree {
                let id = ui.make_persistent_id(&map.id);
                let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
                let holds_selection = self
                    .selected
                    .as_deref()
                    .is_some_and(|selected| selected.strip_prefix(map.id.as_str()).is_some_and(|rest| rest.starts_with('/')));
                if reveal && holds_selection {
                    state.set_open(true);
                }
                state
                    .show_header(ui, |ui| {
                        let response = ui
                            .selectable_label(self.selected.as_deref() == Some(&map.id), map.text())
                            .on_hover_text(map.name.as_str());
                        if response.clicked() {
                            clicked = Some(map.id.clone());
                        }
                        if reveal && self.selected.as_deref() == Some(&map.id) {
                            response.scroll_to_me(Some(Align::Center));
                        }
                    })
                    .body(|ui| {
                        for action in &map.children {
                            let is_selected = self.selected.as_deref() == Some(&action.id);
                            let response =
                                ui.selectable_label(is_selected, action.text()).on_hover_text(action.name.as_str());
                            if response.clicked() {
                                clicked = Some(action.id.clone());
                            }
                            if reveal && is_selected {
                                response.scroll_to_me(Some(Align::Center));
                            }
                        }
                    });
            }
        });
        if let Some(id) = clicked {
            self.select_in_tree(id);
        }
    }

    fn attribute_panel(&mut self, ui: &mut egui::Ui) {
        let title = match &self.document {
            Some(doc) => format!("Actions  -  {} rows", doc.rows.len()),
            None => "Actions".to_string(),
        };
        panel_header(ui, title);
        let selected = self.selected_row();
        // An open dialog is modal, and has its own text to copy.
        if self.dialogs.is_empty() {
            self.table.shortcuts(ui.ctx(), selected);
        }
        let scroll_to = self.scroll_table_to.take();
        let out = self.table.show(ui, selected, scroll_to);
        if let Some(row_ix) = out.clicked_row {
            self.select_in_table(row_ix);
        }
        if out.resorted {
            // The selected action's row moved; keep it in view.
            self.scroll_table_to = self.selected_row();
        }
        self.apply_cell_edits();
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.search.status(ui);
            self.status_text(ui);
        });
    }

    fn status_text(&self, ui: &mut egui::Ui) {
        let text = match &self.document {
            Some(doc) => {
                let mut text = format!(
                    "{} of {} actions without keyboard binding  -  {}  -  {} elements",
                    doc.empty_keyboard_rows,
                    doc.rows.len(),
                    doc.path.display(),
                    doc.element_count
                );
                if let Some(rebind_path) = &doc.rebind_path {
                    text.push_str(&format!(
                        "  -  rebinds: {} ({})",
                        rebind_path.display(),
                        doc.rebinds.len()
                    ));
                }
                text
            }
            None => "No file open".to_string(),
        };
        ui.add(egui::Label::new(RichText::new(text).small().weak()).truncate());
    }

    /// Shows the oldest dialog, running its action when confirmed.
    fn dialog(&mut self, ctx: &Context) {
        let Some(dialog) = self.dialogs.first() else {
            return;
        };
        let mut close = false;
        let mut action = None;
        let response = egui::Modal::new(Id::unique("dialog")).show(ctx, |ui| {
            ui.set_width(dialog.width);
            ui.horizontal(|ui| {
                if dialog.warning {
                    ui.label(RichText::new("⚠").size(20.).color(ui.visuals().warn_fg_color));
                }
                ui.heading(&dialog.title);
            });
            ui.add_space(8.);
            match dialog.highlight {
                Some(phrase) => ui.label(highlighted(ui, &dialog.message, phrase)),
                None => ui.label(&dialog.message),
            };
            if !dialog.list.is_empty() {
                ui.add_space(8.);
                ScrollArea::vertical().max_height(420.).auto_shrink([false, true]).show(ui, |ui| {
                    for line in &dialog.list {
                        ui.label(line);
                    }
                });
            }
            ui.add_space(12.);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &dialog.confirm {
                Some((ok, cancel, confirm)) => {
                    if ui.button(*ok).clicked() {
                        action = Some(confirm.clone());
                        close = true;
                    }
                    if let Some((label, extra)) = &dialog.extra
                        && ui.button(*label).clicked()
                    {
                        action = Some(extra.clone());
                        close = true;
                    }
                    if ui.button(*cancel).clicked() {
                        close = true;
                    }
                }
                None => {
                    if ui.button("OK").clicked() {
                        close = true;
                    }
                }
            });
        });
        if close || response.should_close() {
            self.dialogs.remove(0);
        }
        if let Some(action) = action {
            self.run_dialog_action(action);
        }
    }

    /// Draws the notifications in the bottom-right corner, newest at the bottom; a click
    /// dismisses one.
    fn toasts(&mut self, ctx: &Context) {
        let now = Instant::now();
        self.toasts.retain(|toast| toast.until > now);
        if self.toasts.is_empty() {
            return;
        }
        let mut dismissed = None;
        egui::Area::new(Id::unique("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, [-12., -32.])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(460.);
                for (ix, toast) in self.toasts.iter().enumerate() {
                    let fill = match toast.level {
                        Level::Info => Color32::from_rgb(0x1e, 0x40, 0xaf),
                        Level::Success => Color32::from_rgb(0x16, 0x65, 0x34),
                        Level::Warning => Color32::from_rgb(0x9a, 0x58, 0x00),
                        Level::Error => Color32::from_rgb(0x99, 0x1b, 0x1b),
                    };
                    let response = egui::Frame::popup(ui.style())
                        .fill(fill)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&toast.text).color(Color32::WHITE))
                                    .wrap()
                                    .selectable(false),
                            );
                        })
                        .response
                        .interact(egui::Sense::click());
                    if response.clicked() {
                        dismissed = Some(ix);
                    }
                }
            });
        if let Some(ix) = dismissed {
            self.toasts.remove(ix);
        }
        if let Some(next) = self.toasts.iter().map(|toast| toast.until).min() {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
    }

    /// Shows each open help page in a native window of its own.
    fn help_windows(&mut self, ctx: &Context) {
        for topic in HelpTopic::ALL {
            if !self.help_open.contains(&topic) {
                continue;
            }
            let mut builder = ViewportBuilder::default()
                .with_title(topic.title())
                .with_inner_size([760., 720.])
                .with_min_inner_size([400., 300.]);
            if let Some(icon) = &self.icon {
                builder = builder.with_icon(icon.clone());
            }
            let cache = &mut self.help_cache;
            let close = ctx.show_viewport_immediate(topic.viewport_id(), builder, |ui, _| {
                help::show(ui, topic, cache);
                ui.input(|input| input.viewport().close_requested())
            });
            if close {
                self.help_open.retain(|open| *open != topic);
            }
        }
    }
}

impl eframe::App for CtrlXrefApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_extraction();
        if !self.started {
            self.started = true;
            self.open_startup_profile();
        }
        let mut command = menus::shortcut(&ctx);
        self.arrow_keys(&ctx);

        egui::Panel::top("menu").show(ui, |ui| {
            if let Some(picked) = self.menu_bar(ui) {
                command = Some(picked);
            }
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(260.)
            .size_range(160.0..=900.0)
            .show(ui, |ui| self.tree_panel(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| self.attribute_panel(ui));

        self.search_dialog(&ctx);
        self.dialog(&ctx);
        self.toasts(&ctx);
        self.help_windows(&ctx);
        if let Some(command) = command {
            self.run(command, frame);
        }
    }
}

/// Whether `game_dir` passes both game folder checks, so its defaults can be extracted.
fn can_extract(game_dir: Option<&Path>) -> bool {
    game_dir.is_some_and(|dir| game_dir::is_game_dir(dir) && game_dir::has_extraction_tools(dir))
}

/// Tells the user that `dir` is not a Star Citizen installation, listing which requirements it
/// meets and which it lacks, and that the `Data` folder is not one of them.
fn invalid_game_dir_warning(dir: &Path) -> Dialog {
    let message = format!(
        "The selected folder is not a valid Star Citizen game folder:\n{}\n\n\
         Required for a valid game folder:\n  {}\n\n\
         Required for extracting the defaults (the unp4k tools):\n  {}\n\n\
         The <StarCitizen>\\Data folder does not have to exist yet. It is created under the \
         selected <StarCitizen> folder when the files are extracted from the Data.p4k package.",
        dir.display(),
        game_dir::folder_checks(dir).join("\n  "),
        game_dir::tool_checks(dir).join("\n  "),
    );
    Dialog::warning("Not a game folder", message)
}

/// Whether `path` is a game's own current rebind file, `...\Profiles\default\actionmaps.xml`
/// (LIVE, PTU or any other channel), which the game rewrites. Ignores case.
fn is_current_rebind(path: &Path) -> bool {
    let tail: Vec<String> = path
        .components()
        .rev()
        .take(3)
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    tail == ["actionmaps.xml", "default", "profiles"]
}

/// Tells the user that the startup profile, relative to the game folder, needs one located.
fn no_game_dir_warning() -> Dialog {
    let message = format!(
        "The Star Citizen game folder is not known, so the control profile cannot be opened.\n\
         Use Source > Locate Star Citizen Game Folder to select it; the profile opens after that.\n\
         Default profile: {}",
        game_dir::DEFAULT_PROFILE
    );
    Dialog::warning("Game folder not located", message)
}

/// Tells the user that `game_dir` lacks the `missing` tool files for extracting the default
/// control profiles.
fn missing_tools_warning(game_dir: &Path, missing: &[&str]) -> Dialog {
    let message = format!(
        "Extracting and using the default control profiles is impossible because the tools \
         required for extraction are missing.\n\
         Please paste the unp4k tools ({}) into your Star Citizen folder:\n{}\n\
         Missing: {}",
        game_dir::EXTRACTION_TOOLS.join(", "),
        game_dir.display(),
        missing.join(", ")
    );
    Dialog::warning("Extraction tools missing", message)
}

/// Opens File Explorer on the folder of `path`, with the file selected.
#[cfg(windows)]
fn show_in_explorer(path: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    let path = std::path::absolute(path)?;
    // Not `arg`: that would quote the whole argument, which Explorer doesn't understand.
    std::process::Command::new("explorer").raw_arg(select_argument(&path)).spawn()?;
    Ok(())
}

#[cfg(not(windows))]
fn show_in_explorer(_path: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// The Explorer argument that selects the file at `path`.
#[cfg(windows)]
fn select_argument(path: &Path) -> String {
    format!("/select,\"{}\"", path.display())
}

/// `message` as body text with every `phrase` in it drawn like the Default column's bindings.
fn highlighted(ui: &egui::Ui, message: &str, phrase: &str) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let bold = egui::FontId::new(font.size, egui::FontFamily::Name(super::BOLD.into()));
    let plain = egui::TextFormat::simple(font, ui.visuals().text_color());
    let accent = egui::TextFormat::simple(bold, attribute_table::profile_color(ui));
    let mut job = egui::text::LayoutJob::default();
    for (ix, part) in message.split(phrase).enumerate() {
        if ix > 0 {
            job.append(phrase, 0.0, accent.clone());
        }
        job.append(part, 0.0, plain.clone());
    }
    job
}

fn panel_header(ui: &mut egui::Ui, title: String) {
    ui.add_space(2.);
    let text = if ui.visuals().dark_mode { theme::heading_text(&title) } else { RichText::new(title).strong() };
    ui.label(text);
    ui.separator();
}

/// Element index paths of all `actionmap` elements, in document order. Actionmaps
/// nested in another actionmap are not searched for.
fn find_actionmaps(root: &Element) -> Vec<Vec<usize>> {
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
fn action_paths(root: &Element, maps: &[Vec<usize>]) -> Vec<Vec<usize>> {
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
fn action_key(root: &Element, path: &[usize]) -> Option<(String, String)> {
    let map = xml_doc::element_at(root, path.split_last()?.1)?;
    let action = xml_doc::element_at(root, path)?;
    let name = |element: &Element| element.attributes.get("name").cloned().unwrap_or_default();
    Some((name(map), name(action)))
}

/// The actionmaps at their tree positions as JSON: `{"actionmaps": [{name, UILabel, actions}]}`,
/// with each action's non-blank cells under their `headers`, in column order. `cells` are the
/// table's rows in document order, whose source indices point into `rows`. Blank values and
/// empty `actions` are left out; an actionmap without actions stays, since the tree lists it.
fn tree_json<'a>(
    root: &Element,
    rows: &[Vec<usize>],
    strings: &Strings,
    headers: &[String],
    cells: impl Iterator<Item = (usize, &'a [String])>,
) -> serde_json::Value {
    let mut cells = cells.peekable();
    let mut actionmaps = Vec::new();
    for map_path in find_actionmaps(root) {
        let Some(map) = xml_doc::element_at(root, &map_path) else {
            continue;
        };
        let mut object = serde_json::Map::new();
        let attribute = |name: &str| map.attributes.get(name).map_or("", String::as_str);
        insert_non_blank(&mut object, "name", attribute("name"));
        insert_non_blank(&mut object, "UILabel", strings.resolve(attribute("UILabel")));

        let in_map = |(source_ix, _): &(usize, &[String])| {
            rows.get(*source_ix).and_then(|path| path.split_last()).is_some_and(|(_, parent)| parent == map_path)
        };
        let mut actions = Vec::new();
        while let Some((_, row)) = cells.next_if(in_map) {
            let mut action = serde_json::Map::new();
            for (header, value) in headers.iter().zip(row) {
                insert_non_blank(&mut action, header, value);
            }
            actions.push(serde_json::Value::Object(action));
        }
        if !actions.is_empty() {
            object.insert("actions".into(), serde_json::Value::Array(actions));
        }
        actionmaps.push(serde_json::Value::Object(object));
    }
    serde_json::json!({ "actionmaps": actionmaps })
}

/// Adds `key` to `object` unless `value` is empty or only whitespace.
fn insert_non_blank(object: &mut serde_json::Map<String, serde_json::Value>, key: &str, value: &str) {
    if !value.trim().is_empty() {
        object.insert(key.into(), value.into());
    }
}

/// What a search goes through, in its order: the table's rows as displayed, then the tree's
/// actionmaps, which have no row. A row's matches are those in its cells and in a tree label
/// the game has no text for, which no cell shows. An actionmap's are those in its technical name
/// and its label, which counts only when it does not hold the name. `cells` are the displayed
/// rows, whose source indices point into `rows`.
fn search_places<'a>(
    tree: &[TreeNode],
    rows: &[Vec<usize>],
    cells: impl Iterator<Item = (usize, &'a [String])>,
    regex: &regex::Regex,
) -> Vec<Place> {
    let made_up_labels: HashMap<&str, &str> = tree
        .iter()
        .flat_map(|map| &map.children)
        .filter(|action| action.fallback)
        .map(|action| (action.id.as_str(), action.label.as_str()))
        .collect();
    let actions = cells.enumerate().filter_map(|(row, (source_ix, cells))| {
        let id = tree_id(rows.get(source_ix)?);
        let in_cells: usize = cells.iter().map(|cell| search::occurrences(regex, cell)).sum();
        let in_label = made_up_labels.get(id.as_str()).map_or(0, |label| search::occurrences(regex, label));
        Some(Place { id, row: Some(row), occurrences: in_cells + in_label })
    });
    let maps = tree.iter().map(|map| {
        let in_label = if map.label.contains(&map.name) { 0 } else { search::occurrences(regex, &map.label) };
        Place { id: map.id.clone(), row: None, occurrences: search::occurrences(regex, &map.name) + in_label }
    });
    actions.chain(maps).collect()
}

/// The table row for each action: its attributes in `COLUMNS` with `@key` references resolved
/// through `strings`. The profile's device inputs, in attributes or child elements, go into
/// the read-only Default column separated by [`BINDING_SEPARATOR`]; the device columns only
/// show rebinds, separated the same way.
/// A device with more than one non-empty rebind is marked as a conflict.
/// A `Category` shows its [`category_labels`] text, or its own value when it has none.
fn table_rows(root: &Element, rows: &[Vec<usize>], rebinds: &Rebinds, strings: &Strings) -> Vec<TableRow> {
    let categories = category_labels(root, strings);
    rows.iter()
        .filter_map(|path| {
            let action = xml_doc::element_at(root, path)?;
            let mut cells: Vec<(String, String)> = action
                .attributes
                .iter()
                .filter(|(name, _)| COLUMNS.contains(&name.as_str()) && !DEVICES.contains(&name.as_str()))
                .map(|(name, value)| {
                    let text = match categories.get(value) {
                        Some(label) if name == CATEGORY_ATTRIBUTE => label.as_str(),
                        _ => strings.resolve(value),
                    };
                    (name.clone(), text.to_string())
                })
                .collect();
            let mut styles: Vec<(String, CellStyle)> = Vec::new();
            // Blank profile bindings like `keyboard=" "` bind nothing and are left out.
            let defaults: Vec<String> = DEVICES
                .iter()
                .filter_map(|device| profile_input(action, device))
                .flat_map(|value| value.split(',').map(|input| input.trim().to_string()).collect::<Vec<_>>())
                .filter(|input| !input.is_empty())
                .collect();
            if !defaults.is_empty() {
                cells.push((DEFAULT_COLUMN.into(), defaults.join(BINDING_SEPARATOR)));
                styles.push((DEFAULT_COLUMN.into(), CellStyle::Profile));
            }
            if let Some(inputs) = action_key(root, path).and_then(|key| rebinds.get(&key)) {
                for device in DEVICES {
                    let rebound: Vec<&str> = inputs
                        .iter()
                        .filter(|input| rebind_device(input) == Some(device))
                        .map(String::as_str)
                        .collect();
                    if rebound.is_empty() {
                        continue;
                    }
                    cells.push((device.into(), rebound.join(BINDING_SEPARATOR)));
                    // Empty rebinds like the blank layout's `js1_ | js2_ | js3_` bind
                    // nothing, so they never conflict.
                    if rebound.iter().filter(|input| !is_empty_rebind(input)).count() > 1 {
                        styles.push((device.into(), CellStyle::Conflict));
                    }
                }
            }
            Some(TableRow { tag: action.name.clone(), cells, styles })
        })
        .collect()
}

/// Text by name of the `Category` elements in the profile's `ActionCategories` block, which
/// actions' `Category` attributes refer to. Categories may nest; those without text are left out.
fn category_labels(root: &Element, strings: &Strings) -> HashMap<String, String> {
    fn walk(element: &Element, strings: &Strings, out: &mut HashMap<String, String>) {
        for child in xml_doc::child_elements(element) {
            if let (Some(name), Some(text)) = (child.attributes.get("name"), localized_label(child, strings)) {
                out.insert(name.clone(), text.to_string());
            }
            walk(child, strings, out);
        }
    }
    let mut out = HashMap::new();
    for block in xml_doc::child_elements(root).filter(|child| child.name == "ActionCategories") {
        walk(block, strings, &mut out);
    }
    out
}

/// The profile's `device` input of `action`: its attribute, else its child element's.
fn profile_input(action: &Element, device: &str) -> Option<String> {
    match action.attributes.get(device) {
        Some(value) => Some(value.clone()),
        None => child_device_input(action, device),
    }
}

/// How many actions in `rows` have no keyboard input once `rebinds` are applied.
fn count_empty_keyboard(root: &Element, rows: &[Vec<usize>], rebinds: &Rebinds) -> usize {
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
fn unbound_keyboard_actions(root: &Element, rows: &[Vec<usize>], rebinds: &Rebinds) -> Vec<(String, String)> {
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
fn device_prefix(device: &str) -> &'static str {
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
/// bars (as [`BINDING_SEPARATOR`] shows them), trimmed, blanks dropped, and given the
/// column's [`device_prefix`] when they have none. An input prefixed for another device is
/// an error.
fn parse_cell_inputs(device: &str, text: &str) -> Result<Vec<String>, String> {
    text.split([',', '|'])
        .map(str::trim)
        .filter(|input| !input.is_empty())
        .map(|input| match prefixed_device(input) {
            Some(found) if found == device => Ok(input.to_string()),
            Some(found) => Err(format!("\"{input}\" is a {found} input and can't go in the {device} column")),
            None => Ok(format!("{}{input}", device_prefix(device))),
        })
        .collect()
}

/// Makes `inputs` the `device` rebinds of `action` in actionmap `map` of a rebind file, one
/// `<rebind>` each in that order, where its old `device` rebinds were. A rebind kept from
/// before keeps its other attributes. Creates the action and the actionmap when needed, and
/// removes the action when clearing leaves it empty.
fn set_device_rebinds(root: &mut Element, map: &str, action: &str, device: &str, inputs: &[String]) {
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

/// Binds `input` (a `kb...` keyboard input) to `action` of actionmap `map` in a rebind
/// file: fills its empty keyboard rebind, or adds a `<rebind>`, creating the action and
/// the actionmap when the file has none yet.
fn set_keyboard_rebind(root: &mut Element, map: &str, action: &str, input: &str) {
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
fn complete_blank_layout(root: &mut Element, actions: &[(String, String)]) -> (usize, usize) {
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
fn rebind_device(input: &str) -> Option<&'static str> {
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
fn is_empty_rebind(input: &str) -> bool {
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

/// Where to find the localization file for the profile at `profile`: the first existing
/// [`strings_candidates`] file, searching `xml\` under the working directory and the
/// executable's directory last.
fn find_strings_file(profile: &Path, game_dir: Option<&Path>) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    let xml_dirs = [std::env::current_dir().ok(), exe_dir]
        .into_iter()
        .flatten()
        .map(|dir| dir.join("xml").join(STRINGS_FILE));
    strings_candidates(profile, game_dir)
        .into_iter()
        .chain(xml_dirs)
        .find(|path| path.is_file())
}

/// Localization files for `profile`, best first: next to the profile; the game's English
/// texts in the extracted `Data` folder holding the profile (as `Data\Libs\Config\
/// defaultProfile.xml` is); and those in the `game_dir`.
fn strings_candidates(profile: &Path, game_dir: Option<&Path>) -> Vec<PathBuf> {
    let english = Path::new(game_dir::ENGLISH_STRINGS);
    let data_dir = profile
        .ancestors()
        .skip(1)
        .find(|dir| dir.file_name().is_some_and(|name| name.eq_ignore_ascii_case("Data")));
    [
        profile.parent().map(|dir| dir.join(STRINGS_FILE)),
        data_dir.and_then(Path::parent).map(|game| game.join(english)),
        game_dir.map(|game| game.join(english)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The `<rebind input="..."/>` inputs of every action in a rebind file's actionmaps, as
/// [`rebind_input`] reads them.
fn read_rebinds(root: &Element) -> Rebinds {
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

/// A collapsed tree node for the actionmap at `path` with one leaf per child element.
/// The children's own sub-elements are left out.
fn actionmap_item(root: &Element, path: &[usize], strings: &Strings) -> Option<TreeNode> {
    let map = xml_doc::element_at(root, path)?;
    let id = tree_id(path);
    let mut children = xml_doc::child_elements(map)
        .enumerate()
        .map(|(ix, child)| TreeNode::new(format!("{id}/{ix}"), child, strings, Vec::new()))
        .collect::<Vec<_>>();
    disambiguate_labels(&mut children);
    Some(TreeNode::new(id, map, strings, children))
}

/// The tree of the actionmaps at `paths`, in document order.
fn actionmap_tree(root: &Element, paths: &[Vec<usize>], strings: &Strings) -> Vec<TreeNode> {
    let mut tree: Vec<TreeNode> = paths.iter().filter_map(|path| actionmap_item(root, path, strings)).collect();
    disambiguate_labels(&mut tree);
    tree
}

/// Adds the technical name to labels that several `nodes` share, such as the actionmaps
/// `player` and `prone`, both "On Foot - All" in the game.
fn disambiguate_labels(nodes: &mut [TreeNode]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for node in nodes.iter() {
        *counts.entry(node.label.clone()).or_default() += 1;
    }
    for node in nodes {
        if counts[&node.label] > 1 {
            node.label = format!("{} ({})", node.label, node.name);
        }
    }
}

/// The text of `element`'s `UILabel`, if it has one that is not blank or an unknown `@key`.
fn localized_label<'a>(element: &'a Element, strings: &'a Strings) -> Option<&'a str> {
    let label = element.attributes.get("UILabel")?;
    Some(strings.resolve(label).trim()).filter(|text| !text.is_empty() && !text.starts_with('@'))
}

/// A technical name in words, for elements the game has no text for: `v_hud_cycle_mode_fwd`
/// becomes `Hud Cycle Mode Fwd` and `useAttachmentTop` becomes `Use Attachment Top`.
fn readable_name(name: &str) -> String {
    let mut words = String::new();
    let mut after_lower = false;
    for c in name.strip_prefix("v_").unwrap_or(name).chars() {
        if c == '_' {
            words.push(' ');
            after_lower = false;
            continue;
        }
        if after_lower && c.is_uppercase() {
            words.push(' ');
        }
        if words.is_empty() || words.ends_with(' ') {
            words.extend(c.to_uppercase());
        } else {
            words.push(c);
        }
        after_lower = c.is_lowercase() || c.is_ascii_digit();
    }
    words
}

/// Turns the element index path `[3, 0]` into the tree id `r/3/0`.
fn tree_id(path: &[usize]) -> String {
    path.iter().fold(ROOT_ID.to_string(), |id, ix| format!("{id}/{ix}"))
}

/// The first identifying attribute's value, e.g. `seat_general`; the tag name only when
/// the element has none. The tag is left out otherwise to save horizontal space.
fn element_label(element: &Element) -> String {
    LABEL_ATTRIBUTES
        .iter()
        .find_map(|key| element.attributes.get(*key))
        .unwrap_or(&element.name)
        .clone()
}

/// Turns a tree id like `r/3/0` into the element index path `[3, 0]`.
fn parse_tree_id(id: &str) -> Option<Vec<usize>> {
    let mut parts = id.split('/');
    if parts.next()? != ROOT_ID {
        return None;
    }
    parts.map(|part| part.parse().ok()).collect()
}

fn count_elements(element: &Element) -> usize {
    1 + xml_doc::child_elements(element).map(count_elements).sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::{
        ACTIONMAP_TAG, CellStyle, DEFAULT_COLUMN, DEVICES, Rebinds, TableRow, action_key, actionmap_tree, complete_blank_layout, action_paths, actionmap_item, count_elements, readable_name,
        count_empty_keyboard,
        find_actionmaps, is_current_rebind, parse_cell_inputs, parse_tree_id, read_rebinds, set_device_rebinds,
        set_keyboard_rebind, strings_candidates,
        unbound_keyboard_actions,
        table_rows, tree_id, xml_doc,
    };
    use ctrlxref::localization::Strings;
    use std::path::{Path, PathBuf};

    #[cfg(windows)]
    #[test]
    fn explorer_argument_quotes_only_the_path() {
        assert_eq!(
            super::select_argument(Path::new(r"C:\Star Citizen\my file.xml")),
            r#"/select,"C:\Star Citizen\my file.xml""#
        );
    }

    #[test]
    fn saved_dialog_offers_to_show_the_file() {
        let dialog = super::Dialog::saved("Saved it".into(), Path::new("a.xml"));
        assert_eq!(dialog.message, "Saved it");
        assert!(!dialog.warning);
        let Some((ok, cancel, super::DialogAction::ShowInExplorer(path))) = dialog.confirm else {
            panic!("no Explorer button");
        };
        assert_eq!((ok, cancel), ("Open in Explorer", "Close"));
        assert_eq!(path, Path::new("a.xml"));
    }

    #[test]
    fn created_rebind_dialog_offers_to_open_or_show_the_file() {
        let dialog = super::Dialog::created_rebind("Created it".into(), Path::new("a.xml"));
        assert_eq!(dialog.message, "Created it");
        let Some((ok, cancel, super::DialogAction::OpenRebindFile(open))) = dialog.confirm else {
            panic!("no open button");
        };
        assert_eq!((ok, cancel), ("Open Rebind File", "Close"));
        assert_eq!(open, Path::new("a.xml"));
        let Some((label, super::DialogAction::ShowInExplorer(show))) = dialog.extra else {
            panic!("no Explorer button");
        };
        assert_eq!(label, "Open in Explorer");
        assert_eq!(show, Path::new("a.xml"));
    }

    #[cfg(windows)]
    #[test]
    fn extracted_profile_uses_english_global_ini() {
        let profile = Path::new(r"D:\SC\StarCitizen\Data\Libs\Config\defaultProfile.xml");
        assert_eq!(
            strings_candidates(profile, Some(Path::new(r"E:\Other\StarCitizen"))),
            [
                PathBuf::from(r"D:\SC\StarCitizen\Data\Libs\Config\global.ini"),
                PathBuf::from(r"D:\SC\StarCitizen\Data\Localization\english\global.ini"),
                PathBuf::from(r"E:\Other\StarCitizen\Data\Localization\english\global.ini"),
            ]
        );
        // Outside a Data folder, only next to the profile and the game folder's texts.
        let profile = Path::new(r"D:\xml\actionmaps.xml");
        assert_eq!(
            strings_candidates(profile, Some(Path::new(r"E:\StarCitizen"))),
            [
                PathBuf::from(r"D:\xml\global.ini"),
                PathBuf::from(r"E:\StarCitizen\Data\Localization\english\global.ini"),
            ]
        );
    }

    /// Runs `show` in a headless frame `width` wide, with the app's fonts.
    fn in_frame(width: f32, mut show: impl FnMut(&mut eframe::egui::Ui)) {
        use eframe::egui;
        let ctx = egui::Context::default();
        crate::ui::setup_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.));
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        ctx.run_ui(input, |ui| show(ui)).textures_delta.clear();
    }

    #[test]
    fn highlighted_phrase_is_bold_and_colored_like_the_default_column() {
        use eframe::egui;
        in_frame(800., |ui| {
            let job = super::highlighted(ui, "The Star Citizen game, Star Citizen.", "Star Citizen");
            assert_eq!(job.text, "The Star Citizen game, Star Citizen.");
            let accent = super::attribute_table::profile_color(ui);
            let parts: Vec<(&str, bool, bool)> = job
                .sections
                .iter()
                .map(|section| {
                    let bold = section.format.font_id.family == egui::FontFamily::Name(crate::ui::BOLD.into());
                    let range = section.byte_range.start.0..section.byte_range.end.0;
                    (&job.text[range], bold, section.format.color == accent)
                })
                .collect();
            assert_eq!(
                parts,
                [
                    ("The ", false, false),
                    ("Star Citizen", true, true),
                    (" game, ", false, false),
                    ("Star Citizen", true, true),
                    (".", false, false),
                ]
            );
        });
    }

    #[test]
    fn highlighted_text_wraps_to_the_available_width() {
        in_frame(300., |ui| {
            let message = "A long message about the Star Citizen game. ".repeat(10);
            let width = ui.available_width();
            let response = ui.label(super::highlighted(ui, &message, "Star Citizen"));
            assert!(response.rect.width() <= width + 1., "{} wider than {width}", response.rect.width());
            let row = ui.text_style_height(&eframe::egui::TextStyle::Body);
            assert!(response.rect.height() > row * 3., "{} is not wrapped", response.rect.height());
        });
    }

    const PROFILE: &str = r#"<profile>
        <modifiers><mod input="lalt"/></modifiers>
        <actionmap name="m1"><action name="a"><gamepad input="x"/></action><action name="b"/></actionmap>
        <group><actionmap name="m2"><action name="c"/></actionmap></group>
    </profile>"#;

    /// The places a search for `pattern` goes through, as `(id, displayed row, occurrences)`.
    fn searched(profile: &str, strings: &Strings, pattern: &str) -> Vec<(String, Option<usize>, usize)> {
        let profile = xml_doc::from_str(profile).unwrap();
        let maps = find_actionmaps(&profile);
        let rows = action_paths(&profile, &maps);
        let tree = actionmap_tree(&profile, &maps, strings);
        let mut table = super::AttributeTable::new();
        table.set_rows(&super::COLUMNS, &table_rows(&profile, &rows, &Rebinds::new(), strings));
        let regex = ctrlxref::search::compile(pattern).unwrap();
        super::search_places(&tree, &rows, table.displayed_rows(), &regex)
            .into_iter()
            .map(|place| (place.id, place.row, place.occurrences))
            .collect()
    }

    const SEARCHED: &str = r#"<p>
        <actionmap name="seats" UILabel="@ui_seats">
            <action name="v_eject" UILabel="@ui_eject" keyboard="kb1_e"/>
            <action name="v_hud_cycle" keyboard="kb1_h"/>
        </actionmap>
        <actionmap name="player"><action name="jump" UILabel="@ui_jump" UIDescription="Jump, then jump"/></actionmap>
    </p>"#;

    fn searched_counts(pattern: &str) -> Vec<usize> {
        let strings = Strings::parse("ui_seats=Pilot Seats\nui_eject=Eject\nui_jump=Jump\n");
        searched(SEARCHED, &strings, pattern).into_iter().map(|(_, _, count)| count).collect()
    }

    #[test]
    fn a_search_goes_through_the_rows_and_then_the_actionmaps() {
        let strings = Strings::default();
        let ids: Vec<(String, Option<usize>)> =
            searched(SEARCHED, &strings, "x").into_iter().map(|(id, row, _)| (id, row)).collect();
        let expected = [
            ("r/0/0", Some(0)),
            ("r/0/1", Some(1)),
            ("r/1/0", Some(2)),
            ("r/0", None),
            ("r/1", None),
        ];
        assert_eq!(ids, expected.map(|(id, row)| (id.to_string(), row)));
    }

    #[test]
    fn a_search_counts_the_table_and_what_only_the_tree_shows() {
        // Rows, then actionmaps: the cells, a whole description's worth.
        assert_eq!(searched_counts("jump"), [0, 0, 4, 0, 0]);
        // Case is ignored, and a regular expression spans the cell's text.
        assert_eq!(searched_counts("^V_.*E"), [1, 1, 0, 0, 0]);
        // The bindings count.
        assert_eq!(searched_counts("kb1_"), [1, 1, 0, 0, 0]);
        // An actionmap's label and its technical name.
        assert_eq!(searched_counts("pilot"), [0, 0, 0, 1, 0]);
        assert_eq!(searched_counts("seats"), [0, 0, 0, 2, 0]);
        // A tree label made from the technical name, which no cell shows.
        assert_eq!(searched_counts("Hud Cycle"), [0, 1, 0, 0, 0]);
        // Nothing to find.
        assert_eq!(searched_counts("nowhere"), [0, 0, 0, 0, 0]);
    }

    /// `profile` as the JSON export, through a table like the app's, with `rebinds` applied.
    fn exported(profile: &str, rebinds: &str, strings: &Strings) -> serde_json::Value {
        let profile = xml_doc::from_str(profile).unwrap();
        let rebinds = read_rebinds(&xml_doc::from_str(rebinds).unwrap());
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let mut table = super::AttributeTable::new();
        table.set_rows(&super::COLUMNS, &table_rows(&profile, &rows, &rebinds, strings));
        super::tree_json(&profile, &rows, strings, &table.headers(), table.document_rows())
    }

    #[test]
    fn json_export_nests_actions_in_tree_order_and_leaves_out_blanks() {
        let json = exported(
            r#"<profile>
                <actionmap name="m2" UILabel="@ui_map">
                    <action name="z" UILabel="@ui_z" UIDescription=" " keyboard=" " activationMode="press"/>
                    <action name="a" joystick="js1_button1"/>
                </actionmap>
                <actionmap name="no_actions" UILabel=" "/>
                <group><actionmap name="m1"><action name="b"/></actionmap></group>
            </profile>"#,
            r#"<ActionMaps><actionmap name="m2"><action name="a">
                <rebind input="kb1_ "/><rebind input="mo1_x"/><rebind input="mo1_y"/>
            </action></actionmap></ActionMaps>"#,
            &Strings::parse("ui_map=Map\nui_z=Zed\n"),
        );
        assert_eq!(
            json,
            serde_json::json!({ "actionmaps": [
                { "name": "m2", "UILabel": "Map", "actions": [
                    { "name": "z", "activationMode": "press", "UILabel": "Zed" },
                    // Empty rebinds stay: they unbind the device's default in the game.
                    {
                        "name": "a",
                        DEFAULT_COLUMN: "js1_button1",
                        "keyboard": "kb1_ ",
                        "mouse": "mo1_x | mo1_y",
                    },
                ] },
                { "name": "no_actions" },
                { "name": "m1", "actions": [{ "name": "b" }] },
            ] })
        );
    }

    #[test]
    fn json_export_keeps_the_column_order() {
        let json = exported(
            r#"<p><actionmap name="m"><action UILabel="L" activationMode="press" name="a" mouse="mo1_x"/></actionmap></p>"#,
            "<ActionMaps/>",
            &Strings::default(),
        );
        let keys: Vec<&String> = json["actionmaps"][0]["actions"][0].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["name", DEFAULT_COLUMN, "activationMode", "UILabel"]);
    }

    /// The value of cell `name` in `row`, if it has one.
    fn cell<'a>(row: &'a TableRow, name: &str) -> Option<&'a str> {
        row.cells.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_ref())
    }

    #[test]
    fn only_actionmaps_and_their_actions_are_listed() {
        let root = xml_doc::from_str(PROFILE).unwrap();
        let maps = find_actionmaps(&root);
        assert_eq!(maps, [vec![1], vec![2, 0]]);

        let rows = action_paths(&root, &maps);
        assert_eq!(rows, [vec![1, 0], vec![1, 1], vec![2, 0, 0]]);
        let keys: Vec<(String, String)> =
            rows.iter().filter_map(|path| action_key(&root, path)).collect();
        let keys: Vec<(&str, &str)> = keys.iter().map(|(m, a)| (m.as_str(), a.as_str())).collect();
        assert_eq!(keys, [("m1", "a"), ("m1", "b"), ("m2", "c")]);

        // A device bound in a child element still shows in the Default column.
        let table_rows = table_rows(&root, &rows, &Rebinds::new(), &Strings::default());
        assert_eq!(cell(&table_rows[0], DEFAULT_COLUMN), Some("x"));
        assert_eq!(cell(&table_rows[0], "gamepad"), None);
        assert_eq!(cell(&table_rows[1], DEFAULT_COLUMN), None);

        // The action's <gamepad> child is hidden from the tree.
        let item = actionmap_item(&root, &maps[0], &Strings::default()).unwrap();
        assert_eq!(item.name, "m1");
        assert_eq!(item.children[1].name, "b");
        assert_eq!(item.children.len(), 2);
        assert!(item.children[0].children.is_empty());
        // Hidden elements are still in the document for saving.
        assert!(xml_doc::to_string(&root).unwrap().contains("lalt"));
        assert_eq!(count_elements(&root), 10);
    }

    #[test]
    fn rebinds_match_by_actionmap_and_action_name() {
        let profile = xml_doc::from_str(PROFILE).unwrap();
        let rebind_file = xml_doc::from_str(
            r#"<ActionMaps>
                <actionmap name="m1">
                    <action name="a"><rebind input="gp1_y"/></action>
                    <action name="b"><rebind input="kb1_rctrl+lbracket"/><rebind input="js1_button3"/></action>
                    <action name="no_rebind"/>
                </actionmap>
                <actionmap name="m2"><action name="a"><rebind input="kb1_x"/></action></actionmap>
            </ActionMaps>"#,
        )
        .unwrap();
        let rebinds = read_rebinds(&rebind_file);
        assert_eq!(rebinds.len(), 3);

        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let table_rows = table_rows(&profile, &rows, &rebinds, &Strings::default());
        let column = |name| table_rows.iter().map(|row| cell(row, name)).collect::<Vec<_>>();
        // A rebind replaces the profile's input; m2/a is not in the profile (m2 only has c).
        assert_eq!(column("gamepad"), [Some("gp1_y"), None, None]);
        assert_eq!(column("keyboard"), [None, Some("kb1_rctrl+lbracket"), None]);
        assert_eq!(column("joystick"), [None, Some("js1_button3"), None]);
        assert_eq!(column("rebind"), [None, None, None]);
        // Rebinds are plain text; m1/a's profile binding "x" stays in the Default column.
        assert_eq!(column(DEFAULT_COLUMN), [Some("x"), None, None]);
        assert_eq!(table_rows[0].styles, [(DEFAULT_COLUMN.into(), CellStyle::Profile)]);
        assert!(table_rows[1..].iter().all(|row| row.styles.is_empty()));
    }

    #[test]
    fn profile_bindings_are_styled() {
        let profile = xml_doc::from_str(PROFILE).unwrap();
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let table_rows = table_rows(&profile, &rows, &Rebinds::new(), &Strings::default());
        assert_eq!(table_rows[0].styles, [(DEFAULT_COLUMN.into(), CellStyle::Profile)]);
        assert!(table_rows[1].styles.is_empty());
    }

    #[test]
    fn profile_bindings_go_to_the_default_column_space_separated() {
        let profile = xml_doc::from_str(
            r#"<p><actionmap name="m">
                <action name="a" keyboard="f" mouse="mouse1" joystick=" "/>
            </actionmap></p>"#,
        )
        .unwrap();
        let rebind_file = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="kb1_g"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let plain = table_rows(&profile, &rows, &Rebinds::new(), &Strings::default());
        assert_eq!(cell(&plain[0], DEFAULT_COLUMN), Some("f | mouse1"));
        assert!(DEVICES.iter().all(|device| cell(&plain[0], device).is_none()));

        let rebound = table_rows(&profile, &rows, &read_rebinds(&rebind_file), &Strings::default());
        assert_eq!(cell(&rebound[0], DEFAULT_COLUMN), Some("f | mouse1"));
        assert_eq!(cell(&rebound[0], "keyboard"), Some("kb1_g"));
        assert_eq!(cell(&rebound[0], "mouse"), None);
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
    fn several_rebinds_of_one_device_conflict() {
        let profile = xml_doc::from_str(PROFILE).unwrap();
        let rebind_file = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m1">
                <action name="b"><rebind input="kb1_x"/><rebind input="mo1_mouse1"/><rebind input="kb1_y"/><rebind input="js1_x"/></action>
            </actionmap></ActionMaps>"#,
        )
        .unwrap();
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let table_rows = table_rows(&profile, &rows, &read_rebinds(&rebind_file), &Strings::default());
        assert_eq!(cell(&table_rows[1], "keyboard"), Some("kb1_x | kb1_y"));
        assert_eq!(cell(&table_rows[1], "mouse"), Some("mo1_mouse1"));
        assert_eq!(table_rows[1].styles, [("keyboard".into(), CellStyle::Conflict)]);
    }

    #[test]
    fn empty_rebinds_do_not_conflict() {
        let profile = xml_doc::from_str(PROFILE).unwrap();
        let rebind_file = xml_doc::from_str(
            r#"<ActionMaps><actionmap name="m1"><action name="b">
                <rebind device="joystick" input="js1_ "/><rebind device="joystick" input="js2_ "/><rebind device="joystick" input="js3_ "/>
                <rebind input="kb1_x"/><rebind input="kb1_ "/>
                <rebind input="mo1_mouse1"/><rebind input="mo1_mouse2"/><rebind input="mo2_ "/>
            </action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let table_rows = table_rows(&profile, &rows, &read_rebinds(&rebind_file), &Strings::default());
        assert_eq!(cell(&table_rows[1], "joystick"), Some("js1_  | js2_  | js3_ "));
        // Only the mouse has two real rebinds.
        assert_eq!(table_rows[1].styles, [("mouse".into(), CellStyle::Conflict)]);
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
            parse_cell_inputs("keyboard", " up , kb2_numpad8,, lalt+k ").unwrap(),
            ["kb1_up", "kb2_numpad8", "kb1_lalt+k"]
        );
        // A conflict cell is shown with " | " between the inputs; editing it keeps them apart.
        assert_eq!(parse_cell_inputs("keyboard", "kb1_x | kb1_y").unwrap(), ["kb1_x", "kb1_y"]);
        assert_eq!(parse_cell_inputs("mouse", "mouse4").unwrap(), ["mo1_mouse4"]);
        assert_eq!(parse_cell_inputs("joystick", " ").unwrap(), Vec::<String>::new());
        assert!(parse_cell_inputs("keyboard", "w, js1_button2").is_err());
    }

    #[test]
    fn current_rebind_file_is_recognized() {
        use std::path::Path;
        assert!(is_current_rebind(Path::new(
            r"D:\Games\StarCitizen\LIVE\user\client\0\Profiles\default\actionmaps.xml"
        )));
        assert!(is_current_rebind(Path::new(r"E:\SC\PTU\user\client\0\profiles\Default\ActionMaps.xml")));
        assert!(!is_current_rebind(Path::new(
            r"D:\Games\StarCitizen\LIVE\user\client\0\controls\mappings\actionmaps.xml"
        )));
        assert!(!is_current_rebind(Path::new(r"D:\xml\actionmaps.xml")));
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

    #[test]
    fn references_resolve_to_localized_texts() {
        let profile = xml_doc::from_str(
            r#"<p><actionmap name="m"><action name="v_eject" UILabel="@ui_CIEject" UIDescription="@ui_missing" onPress="1"/></actionmap></p>"#,
        )
        .unwrap();
        let strings = Strings::parse("ui_CIEject=Eject\n");
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let table_rows = table_rows(&profile, &rows, &Rebinds::new(), &strings);
        assert_eq!(cell(&table_rows[0], "UILabel"), Some("Eject"));
        assert_eq!(cell(&table_rows[0], "UIDescription"), Some("@ui_missing"));
        assert_eq!(cell(&table_rows[0], "name"), Some("v_eject"));
        // Attributes outside COLUMNS are not shown.
        assert_eq!(cell(&table_rows[0], "onPress"), None);
    }

    #[test]
    fn tree_shows_localized_labels_and_marks_fallbacks() {
        let profile = xml_doc::from_str(
            r#"<p>
                <actionmap name="seat_general" UILabel="@ui_seat">
                    <action name="v_eject" UILabel="@ui_eject"/>
                    <action name="v_hud_cycle_mode_fwd"/>
                    <action name="v_dead" UILabel="@ui_missing"/>
                    <action name="v_blank" UILabel="@ "/>
                </actionmap>
                <actionmap name="mapui" UILabel="@ "/>
            </p>"#,
        )
        .unwrap();
        let strings = Strings::parse("ui_seat=Seats\nui_eject=Eject\n");
        let tree = actionmap_tree(&profile, &find_actionmaps(&profile), &strings);

        assert_eq!((tree[0].label.as_str(), tree[0].name.as_str(), tree[0].fallback), ("Seats", "seat_general", false));
        assert_eq!((tree[1].label.as_str(), tree[1].fallback), ("Mapui", true));
        let actions: Vec<(&str, bool)> = tree[0].children.iter().map(|a| (a.label.as_str(), a.fallback)).collect();
        assert_eq!(actions, [("Eject", false), ("Hud Cycle Mode Fwd", true), ("Dead", true), ("Blank", true)]);
    }

    #[test]
    fn shared_tree_labels_get_the_technical_name() {
        let profile = xml_doc::from_str(
            r#"<p>
                <actionmap name="player" UILabel="@ui_foot"/>
                <actionmap name="prone" UILabel="@ui_foot"/>
                <actionmap name="stopwatch" UILabel="@ui_watch"/>
            </p>"#,
        )
        .unwrap();
        let strings = Strings::parse("ui_foot=On Foot - All\nui_watch=Stop Watch\n");
        let tree = actionmap_tree(&profile, &find_actionmaps(&profile), &strings);
        let labels: Vec<&str> = tree.iter().map(|map| map.label.as_str()).collect();
        assert_eq!(labels, ["On Foot - All (player)", "On Foot - All (prone)", "Stop Watch"]);
    }

    #[test]
    fn readable_names_split_words() {
        assert_eq!(readable_name("v_hud_cycle_mode_fwd"), "Hud Cycle Mode Fwd");
        assert_eq!(readable_name("useAttachmentTop"), "Use Attachment Top");
        assert_eq!(readable_name("RemoteRigidEntityController"), "Remote Rigid Entity Controller");
        assert_eq!(readable_name("v_IFCS_A"), "IFCS A");
        assert_eq!(readable_name("ui_3d_display_toggledPanX"), "Ui 3d Display Toggled Pan X");
    }

    #[test]
    fn categories_resolve_through_action_categories() {
        let profile = xml_doc::from_str(
            r#"<p>
                <ActionCategories>
                    <Category name="ShipSystems" UILabel="@pit_ship">
                        <Category name="RemoteTurret" UILabel="@pit_turret"/>
                    </Category>
                    <Category name="Dead" UILabel="@pit_missing"/>
                </ActionCategories>
                <actionmap name="m" UILabel="@pit_ship">
                    <action name="a" Category="ShipSystems"/>
                    <action name="b" Category="RemoteTurret"/>
                    <action name="c" Category="Notifications"/>
                    <action name="d" Category="Dead"/>
                    <action name="ShipSystems" UILabel="ShipSystems"/>
                </actionmap>
            </p>"#,
        )
        .unwrap();
        let strings = Strings::parse("pit_ship=Ship Systems \npit_turret=Remote Turret\n");
        let rows = action_paths(&profile, &find_actionmaps(&profile));
        let table = table_rows(&profile, &rows, &Rebinds::new(), &strings);
        let cell = |row: usize, name: &str| {
            table[row].cells.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str()).unwrap().to_string()
        };
        assert_eq!(cell(0, "Category"), "Ship Systems");
        assert_eq!(cell(1, "Category"), "Remote Turret");
        assert_eq!(cell(2, "Category"), "Notifications");
        assert_eq!(cell(3, "Category"), "Dead");
        // Only Category is looked up: the same word as a name or a label stays as it is.
        assert_eq!(cell(4, "name"), "ShipSystems");
        assert_eq!(cell(4, "UILabel"), "ShipSystems");
    }

    #[test]
    fn tree_ids_round_trip_to_paths() {
        assert_eq!(tree_id(&[2, 0, 1]), "r/2/0/1");
        assert_eq!(parse_tree_id(&tree_id(&[2, 0, 1])), Some(vec![2, 0, 1]));
        assert_eq!(parse_tree_id("r"), Some(vec![]));
        assert_eq!(parse_tree_id("x/1"), None);
    }
}
