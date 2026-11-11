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
use ctrlxref::rebinds::{
    DEVICES, MAX_HEADER_LABEL, MatchReason, Rebinds, Suggestion, action_key, action_paths, clear_device_rebinds,
    complete_blank_layout, count_empty_keyboard, find_actionmaps, header_label, header_label_error, is_empty_rebind,
    merge_rebinds, move_action_rebinds, parse_cell_inputs, profile_input, read_rebinds, rebind_device, remove_action,
    remove_unmatched, set_device_rebinds, set_header_label, set_keyboard_rebind, suggest_targets,
    unbound_keyboard_actions, unmatched_actions,
};
use ctrlxref::search::{self, Place};
use ctrlxref::xml_doc;
use ctrlxref::{extract, game_dir, keys};
use eframe::egui::text::CCursor;
use eframe::egui::text_selection::CCursorRange;
use eframe::egui::{
    self, Align, Align2, Color32, Context, IconData, Id, Key, Layout, RichText, ScrollArea, TextEdit, Theme,
    ViewportBuilder, ViewportCommand,
};
use egui_commonmark::CommonMarkCache;
use xmltree::Element;

use super::attribute_table::{self, AttributeTable, CellEdit, CellStyle, TableRow};
use super::control_names::{self, ControlNamesWindow};
use super::help::{self, HelpTopic};
use super::menus::{self, Command, ThemeChoice};
use super::search::{SearchAction, SearchDialog};
use super::theme;

/// Tree id of the document root; descendants append `/<element index>` per level.
const ROOT_ID: &str = "r";

/// Attributes whose value is shown next to the element name in the tree.
const LABEL_ATTRIBUTES: [&str; 4] = ["name", "id", "key", "action"];

/// Read-only column with the profile's own bindings, space separated, so the device
/// columns only hold rebinds and can all be edited.
const DEFAULT_COLUMN: &str = "DEFAULT from: Data.p4k R/O";

/// What messages call the open defaultProfile.xml: the actions of the installed game version,
/// which rebind files, often made for an older version, are matched against.
const MASTER_PROFILE: &str = "game engine current profile";

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

/// How many changes to the rebinds Ctrl+Z can undo.
const MAX_UNDO: usize = 100;

/// The size of a help window's content when it opens.
const HELP_SIZE: egui::Vec2 = egui::vec2(760., 720.);

/// How long a notification stays up; errors stay longer.
const TOAST_TIME: Duration = Duration::from_secs(5);
const ERROR_TOAST_TIME: Duration = Duration::from_secs(9);

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
    /// Whether another rebind file was merged into it, so that it is no longer what `rebind_path`
    /// holds plus edits and Save Rebind File is off.
    merged: bool,
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
    /// When it goes away by itself; `None` stays until dismissed.
    until: Option<Instant>,
    /// Whether it is the warning about rebinds of actions the profile lacks, which stays with a
    /// Review... button until they are dealt with or it is dismissed.
    review: bool,
}

/// How many profile actions the Unmatched Rebinds box suggests for each action the profile lacks.
const MAX_SUGGESTIONS: usize = 3;

/// An action of the open rebind file that the profile lacks, in the Unmatched Rebinds box.
struct UnmatchedRow {
    /// `(actionmap name, action name)`.
    key: (String, String),
    inputs: Vec<String>,
    /// Profile actions its rebinds may belong to, likeliest first, each with whether that action
    /// already has rebinds of a device these rebind too, so both show as a conflict.
    suggestions: Vec<(Suggestion, bool)>,
}

/// A button of the Unmatched Rebinds box.
enum UnmatchedChange {
    /// Moves the rebinds of the first action to the second.
    Move((String, String), (String, String)),
    Remove((String, String)),
    /// Moves the rebinds of every action to its first suggestion.
    ApplyAll,
    RemoveAll,
}

/// What a dialog's confirm button does.
#[derive(Clone)]
enum DialogAction {
    ExtractDefaults,
    WriteRebindFile,
    Copy(String),
    ShowInExplorer(PathBuf),
    OpenRebindFile(PathBuf),
    /// Saves the open rebind file, and then does what was asked if that worked.
    SaveThen(AfterPrompt),
    /// Asks where to save the open rebind file, and then does what was asked if that worked.
    SaveAsThen(AfterPrompt),
    /// Does what was asked without saving the open rebind file.
    DiscardThen(AfterPrompt),
}

/// The open rebind file's state before one change to it.
struct UndoStep {
    /// What the change was, for the notification.
    label: String,
    root: Element,
    /// Whether that state had changes that are not saved.
    modified: bool,
    /// Whether the change was the first merge into the file, so undoing it ends the merged state.
    first_merge: bool,
}

/// The changes to the open rebind file that Ctrl+Z can undo, latest last.
#[derive(Default)]
struct UndoStack(Vec<UndoStep>);

impl UndoStack {
    /// Remembers the state before a change, forgetting the oldest beyond [`MAX_UNDO`].
    fn push(&mut self, label: String, root: Element, modified: bool) {
        self.0.push(UndoStep { label, root, modified, first_merge: false });
        if self.0.len() > MAX_UNDO {
            self.0.remove(0);
        }
    }

    /// Notes that the file was saved, so going back to any earlier state differs from the file.
    fn mark_saved(&mut self) {
        for step in &mut self.0 {
            step.modified = true;
        }
    }
}

/// What was asked when [`unsaved_rebinds_warning`] came up, to do once it is answered.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AfterPrompt {
    Exit,
    CloseFile,
    OpenProfile(PathBuf),
    OpenRebind(PathBuf),
}

impl AfterPrompt {
    /// The end of the question, "Save them before ...?".
    fn question(&self) -> &'static str {
        match self {
            Self::Exit => "exiting",
            Self::CloseFile => "closing the file",
            Self::OpenProfile(_) | Self::OpenRebind(_) => "opening the other file",
        }
    }

    /// The label of the button that goes on without saving.
    fn discard_label(&self) -> &'static str {
        match self {
            Self::Exit => "Exit Without Saving",
            Self::CloseFile => "Close Without Saving",
            Self::OpenProfile(_) | Self::OpenRebind(_) => "Open Without Saving",
        }
    }
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

/// The box Save Rebind Copy As shows first, asking for the copy's header label.
struct LabelPrompt {
    text: String,
    /// Set until the field has taken the focus with all its text selected.
    focus: bool,
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
    help_open: Vec<(HelpTopic, Option<egui::Pos2>)>,
    help_cache: CommonMarkCache,
    icon: Option<Arc<IconData>>,
    toasts: Vec<Toast>,
    /// Message boxes, shown one at a time, oldest first.
    dialogs: Vec<Dialog>,
    search: SearchDialog,
    /// Tree ids of what Mark All marked; the table has its rows' marks itself.
    marked: HashSet<String>,
    /// Whether the open rebind file has changes that are not saved to it, which exiting, closing
    /// the file and opening another one ask about.
    rebinds_modified: bool,
    /// The changes to the open rebind file that Ctrl+Z can undo.
    undo: UndoStack,
    /// What to do after the Save As box the unsaved rebinds question asked for, shown next.
    save_as_after: Option<AfterPrompt>,
    /// The Tools > Show Control Names window, while it is open.
    control_names: Option<ControlNamesWindow>,
    /// The Save Rebind Copy As label box, while it is open.
    label_prompt: Option<LabelPrompt>,
    /// The label the box accepted, for the save box shown next.
    pending_copy_label: Option<String>,
    /// The Unmatched Rebinds box, while it is open.
    unmatched: Option<Vec<UnmatchedRow>>,
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
        help::install_images(ctx);
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
            marked: HashSet::new(),
            rebinds_modified: false,
            undo: UndoStack::default(),
            save_as_after: None,
            control_names: None,
            label_prompt: None,
            pending_copy_label: None,
            unmatched: None,
        }
    }

    fn notify(&mut self, level: Level, text: impl Into<String>) {
        let time = if level == Level::Error { ERROR_TOAST_TIME } else { TOAST_TIME };
        self.toasts.push(Toast { level, text: text.into(), until: Some(Instant::now() + time), review: false });
        self.ctx.request_repaint();
    }

    /// Warns, until dismissed or dealt with, that the open rebind file has rebinds of actions the
    /// profile lacks, with a Review... button for the Unmatched Rebinds box. `lead` starts the text.
    fn warn_unmatched(&mut self, lead: &str) {
        let count = self.document.as_ref().map_or(0, |doc| unmatched_actions(&doc.rebinds, &profile_actions(doc)).len());
        self.toasts.retain(|toast| !toast.review);
        if count == 0 {
            return;
        }
        let text = format!(
            "{lead}{count} rebound actions are not in the {MASTER_PROFILE} (defaultProfile.xml from \
             Data.p4k), so the game ignores their bindings. The rebind file was probably made for an older \
             game version, and these actions were renamed or moved since"
        );
        self.toasts.push(Toast { level: Level::Warning, text, until: None, review: true });
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
            Command::MergeRebindFile => self.merge_rebind_file(frame),
            Command::SaveRebindFile => self.save_rebind_file(),
            Command::SaveRebindCopyAs => self.save_rebind_copy_as(),
            Command::CreateBlankRebindFile => self.create_blank_rebind_file(frame),
            Command::CloseFile => self.unless_unsaved(AfterPrompt::CloseFile),
            Command::Quit => self.ctx.send_viewport_cmd(ViewportCommand::Close),
            Command::Theme(choice) => self.set_theme(choice),
            Command::ToggleTheme => self.toggle_theme(),
            Command::LocateGameFolder => self.locate_game_folder(frame),
            Command::ExtractDefaults => self.extract_defaults(),
            Command::ShowUnusedCombos => self.show_unused_combos(),
            Command::ResolveConflicts => self.resolve_conflicts(),
            Command::ResolveUnmatched => self.show_unmatched(),
            Command::ShowControlNames => self.show_control_names(),
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
        self.rebinds_modified = false;
        self.undo = UndoStack::default();
        self.document = Some(Document {
            element_count: count_elements(&root),
            tree,
            actionmap_count: maps.len(),
            rows,
            rebinds: Rebinds::new(),
            rebind_path: None,
            rebind_root: None,
            merged: false,
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
        let unmatched = unmatched_actions(&rebinds, &profile_actions(doc)).len();
        let note = format!("{} of {} rebinds matched from {}", rebinds.len() - unmatched, rebinds.len(), path.display());
        doc.rebinds = rebinds;
        doc.rebind_path = Some(path);
        doc.rebind_root = Some(root);
        doc.merged = false;
        self.rebinds_modified = false;
        self.undo = UndoStack::default();
        self.refresh_table();
        self.update_title();

        if unmatched == 0 {
            self.notify(Level::Success, note);
        } else {
            self.warn_unmatched(&format!("{note}. "));
        }
    }

    /// Whether Save Rebind File may overwrite the file: not once another file was merged into it.
    fn can_save(&self) -> bool {
        !self.document.as_ref().is_some_and(|doc| doc.merged)
    }

    /// Whether a rebind file is open for another one to be merged into. Opening a profile
    /// closes the rebind file, so this is false again after that.
    fn can_merge(&self) -> bool {
        self.document.as_ref().is_some_and(|doc| doc.rebind_root.is_some())
    }

    /// Appends the bindings of the rebind file at `path` to the open one, conflicting ones
    /// included, for the user to choose from in the table. Ctrl+Z takes them out again.
    fn merge_rebind_path(&mut self, path: &Path) {
        let other = match xml_doc::load(path) {
            Ok(other) => other,
            Err(err) => {
                self.notify(Level::Error, format!("{err:#}"));
                return;
            }
        };
        let Some(doc) = &mut self.document else {
            return;
        };
        let Some(rebind_root) = &mut doc.rebind_root else {
            return;
        };
        let before = rebind_root.clone();
        let added = merge_rebinds(rebind_root, &other);
        if added == 0 {
            self.notify(Level::Info, format!("{} has no bindings to add", path.display()));
            return;
        }
        doc.rebinds = read_rebinds(rebind_root);
        let first_merge = !std::mem::replace(&mut doc.merged, true);
        let unmatched = unmatched_actions(&read_rebinds(&other), &profile_actions(doc)).len();
        self.undo.push("merging a rebind file".into(), before, self.rebinds_modified);
        if let Some(step) = self.undo.0.last_mut() {
            step.first_merge = first_merge;
        }
        self.rebinds_modified = true;
        self.refresh_table();
        let note = format!(
            "Added {added} bindings from {}. Conflicts are shown in red. Use Save Rebind Copy As to keep this",
            path.display()
        );
        self.notify(Level::Success, note);
        if unmatched > 0 {
            self.warn_unmatched("After the merge, ");
        }
    }

    /// Opens the Unmatched Rebinds box for the rebinds of actions the profile lacks, as Tools >
    /// Resolve Unmatched Rebinds asks; or says that there are none.
    fn show_unmatched(&mut self) {
        if !self.has_profile() || !self.has_rebind_file() {
            return;
        }
        let rows = self.document.as_ref().map(unmatched_rows).unwrap_or_default();
        if rows.is_empty() {
            self.toasts.retain(|toast| !toast.review);
            self.notify(Level::Info, format!("All rebinds match the actions of the {MASTER_PROFILE}"));
        } else {
            self.unmatched = Some(rows);
        }
    }

    /// Whether a rebind file is open; warns when not.
    fn has_rebind_file(&mut self) -> bool {
        if !self.can_merge() {
            self.notify(
                Level::Warning,
                "No rebind file is open (File > Open Current Rebind File or Open Custom Rebind File...)",
            );
        }
        self.can_merge()
    }

    /// Makes a change the Unmatched Rebinds box asks for in the open rebind file, as one step
    /// that Ctrl+Z undoes. The box then lists what is left, and closes when nothing is.
    fn apply_unmatched(&mut self, change: UnmatchedChange) {
        let suggested: Vec<((String, String), (String, String))> = match &change {
            UnmatchedChange::ApplyAll => self
                .unmatched
                .iter()
                .flatten()
                .filter_map(|row| Some((row.key.clone(), row.suggestions.first()?.0.target.clone())))
                .collect(),
            _ => Vec::new(),
        };
        let Some(doc) = &mut self.document else {
            return;
        };
        let actions = profile_actions(doc);
        let Some(root) = &mut doc.rebind_root else {
            return;
        };
        let before = root.clone();
        let named = |(map, action): &(String, String)| format!("{map}/{action}");
        let (label, message) = match change {
            UnmatchedChange::Move(from, to) => {
                let moved = move_action_rebinds(root, &from, &to);
                (
                    format!("moving the rebinds of {}", from.1),
                    format!("Moved {moved} rebinds of {} to {}", named(&from), named(&to)),
                )
            }
            UnmatchedChange::Remove(key) => {
                let removed = remove_action(root, &key);
                (format!("removing the rebinds of {}", key.1), format!("Removed {removed} rebinds of {}", named(&key)))
            }
            UnmatchedChange::ApplyAll => {
                let moved: usize = suggested.iter().map(|(from, to)| move_action_rebinds(root, from, to)).sum();
                (
                    "moving rebinds to the suggested actions".into(),
                    format!("Moved {moved} rebinds of {} actions to the suggested ones", suggested.len()),
                )
            }
            UnmatchedChange::RemoveAll => {
                let removed = remove_unmatched(root, &actions);
                ("removing the unmatched rebinds".into(), format!("Removed {removed} actions not in the {MASTER_PROFILE}"))
            }
        };
        if *root == before {
            return;
        }
        doc.rebinds = read_rebinds(root);
        self.undo.push(label, before, self.rebinds_modified);
        self.rebinds_modified = true;
        self.refresh_table();
        self.notify(Level::Success, format!("{message}. Ctrl+Z undoes it. Save the rebind file to keep this"));
    }

    /// Starts going through the conflicting cells, editing one at a time.
    fn resolve_conflicts(&mut self) {
        if self.table.start_resolving() {
            self.notify(Level::Info, "Delete the controls you don't want. Tab: next, Shift+Tab: previous, Esc: stop");
        } else {
            self.notify(Level::Info, "There are no conflicting bindings");
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
        // Once the profile has every rebound action, nothing is left to warn about or resolve.
        let any_unmatched = self
            .document
            .as_ref()
            .is_some_and(|doc| !unmatched_actions(&doc.rebinds, &profile_actions(doc)).is_empty());
        if !any_unmatched {
            self.toasts.retain(|toast| !toast.review);
            self.unmatched = None;
        } else if self.unmatched.is_some() {
            self.unmatched = self.document.as_ref().map(unmatched_rows);
        }
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
        // The marks stay, so the search can step through them.
        self.table.clear_cell_selection();
        match place.row {
            Some(row) => {
                self.selected = Some(place.id.clone());
                self.scroll_table_to = Some(row);
            }
            None => self.move_in_tree(place.id.clone()),
        }
        self.reveal_in_tree = true;
    }

    /// Marks every place with matches, in the table and the tree, in place of earlier marks. A
    /// search that finds something joins the previous ones.
    fn mark_all(&mut self) {
        if self.search.regex().is_none() {
            return;
        }
        let places = self.search_places();
        self.search.set_counts(&places);
        let with_matches: Vec<&Place> = places.iter().filter(|place| place.occurrences > 0).collect();
        if !with_matches.is_empty()
            && let Err(err) = self.search.remember()
        {
            self.notify(Level::Error, format!("{err:#}"));
        }
        let rows = with_matches.iter().filter_map(|place| self.table.source_of(place.row?)).collect();
        self.table.set_marks(rows);
        self.marked = with_matches.iter().map(|place| place.id.clone()).collect();
        self.search.set_marked(self.marked.len());
    }

    /// Clears the marks of the tree, which go with the table's.
    fn clear_tree_marks(&mut self) {
        self.marked.clear();
        self.search.set_marked(0);
    }

    /// Draws the Search dialog and does what was asked in it.
    fn search_dialog(&mut self, ctx: &Context) {
        match self.search.show(ctx) {
            Some(SearchAction::Changed) => {
                // Marks belong to the search that made them.
                self.table.clear_marks();
                self.clear_tree_marks();
                self.update_search_counts();
            }
            Some(SearchAction::FindNext) => self.find(true),
            Some(SearchAction::FindPrevious) => self.find(false),
            Some(SearchAction::MarkAll) => self.mark_all(),
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

    /// Tree selection changed to `id`: move there, clearing the selected cells and the marks.
    fn select_in_tree(&mut self, id: String) {
        self.move_in_tree(id);
        self.table.clear_selection();
    }

    /// Selects `id` and scrolls the table to the action's row, or for an actionmap to its first
    /// row in the current sort order.
    fn move_in_tree(&mut self, id: String) {
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
        let Some((map, action)) = doc.rows.get(edit.source_ix).and_then(|path| action_key(&doc.root, path)) else {
            return;
        };
        let existing = doc.rebinds.get(&(map.clone(), action.clone())).cloned().unwrap_or_default();
        let inputs = match parse_cell_inputs(device, &edit.value, &existing) {
            Ok(inputs) => inputs,
            Err(message) => {
                self.notify(Level::Warning, message);
                return;
            }
        };
        let before = rebind_root.clone();
        set_device_rebinds(rebind_root, &map, &action, device, &inputs);
        // Retyping the same binding changes nothing.
        let rebinds = read_rebinds(rebind_root);
        if rebinds != doc.rebinds {
            self.undo.push(format!("{device} binding of {action}"), before, self.rebinds_modified);
            self.rebinds_modified = true;
        }
        doc.rebinds = rebinds;
    }

    /// Removes all rebinds of `device` from the actions in the table, as the context menu asks.
    /// The game then uses those actions' default bindings again. Ctrl+Z brings the rebinds back.
    fn clear_column(&mut self, device: &str) {
        let Some(doc) = &mut self.document else {
            return;
        };
        let Some(rebind_root) = &mut doc.rebind_root else {
            return;
        };
        let actions: Vec<(String, String)> = doc.rows.iter().filter_map(|path| action_key(&doc.root, path)).collect();
        let before = rebind_root.clone();
        let cleared = clear_device_rebinds(rebind_root, &doc.rebinds, &actions, device);
        if cleared == 0 {
            return;
        }
        doc.rebinds = read_rebinds(rebind_root);
        self.undo.push(format!("clearing the {device} column"), before, self.rebinds_modified);
        self.rebinds_modified = true;
        self.refresh_table();
        self.notify(
            Level::Success,
            format!(
                "Cleared {cleared} {device} bindings; those actions now use their defaults. \
                 Ctrl+Z brings them back. Save the rebind file to keep this"
            ),
        );
    }

    /// Ctrl+Z undoes the latest change to the rebinds, unless a text field has the keyboard, which
    /// then undoes its own typing.
    fn undo_key(&mut self, ctx: &Context) {
        let undo = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, Key::Z);
        if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|input| input.consume_shortcut(&undo)) {
            self.undo_change();
        }
    }

    /// Puts the open rebind file back as it was before its latest change.
    fn undo_change(&mut self) {
        let (Some(doc), Some(step)) = (&mut self.document, self.undo.0.pop()) else {
            self.notify(Level::Info, "Nothing to undo");
            return;
        };
        doc.rebinds = read_rebinds(&step.root);
        doc.rebind_root = Some(step.root);
        doc.merged &= !step.first_merge;
        self.rebinds_modified = step.modified;
        self.refresh_table();
        self.notify(Level::Info, format!("Undid {}", step.label));
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
            self.unless_unsaved(AfterPrompt::OpenProfile(path));
        }
    }

    /// Whether a profile is open, which rebind files are matched against; warns when not.
    fn has_profile(&mut self) -> bool {
        if self.document.is_none() {
            self.notify(
                Level::Warning,
                format!("Open the {MASTER_PROFILE} first (File > Open defaultProfile), then a rebind file"),
            );
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
        self.unless_unsaved(AfterPrompt::OpenRebind(dir.join(game_dir::CURRENT_REBIND)));
    }

    /// Asks for a rebind file, starting in the game's [`game_dir::MAPPINGS_DIR`] where it
    /// saves the user's mappings.
    fn open_custom_rebind_file(&mut self, frame: &eframe::Frame) {
        if !self.has_profile() {
            return;
        }
        if let Some(path) = self.pick_rebind_file("Open Custom Rebind File", frame) {
            self.unless_unsaved(AfterPrompt::OpenRebind(path));
        }
    }

    /// Asks for a rebind file whose bindings are added to the open one. The menu item is
    /// disabled without an open rebind file.
    fn merge_rebind_file(&mut self, frame: &eframe::Frame) {
        if !self.can_merge() {
            return;
        }
        if let Some(path) = self.pick_rebind_file("Merge Rebind File", frame) {
            self.merge_rebind_path(&path);
        }
    }

    /// Shows a file browser for a rebind file, titled `title`, starting in the game's mappings
    /// folder when it exists.
    fn pick_rebind_file(&self, title: &str, frame: &eframe::Frame) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(title)
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
        dialog.pick_file()
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
        if !self.can_save() {
            self.notify(Level::Warning, "After a merge, use Save Rebind Copy As to choose where to save");
            return;
        }
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
            Ok(()) => {
                self.rebinds_modified = false;
                self.undo.mark_saved();
                self.dialogs.push(Dialog::saved(format!("Saved {}", path.display()), path));
            }
            Err(err) => self.notify(Level::Error, format!("{err:#}")),
        }
    }

    /// The window was asked to close, by File > Exit, Ctrl+Q or its X button. With rebinds that
    /// are not saved, it stays open and asks what to do, in front of any other message box.
    fn on_close_requested(&mut self) {
        if !self.rebinds_modified {
            return;
        }
        self.ctx.send_viewport_cmd(ViewportCommand::CancelClose);
        self.ask_about_unsaved(AfterPrompt::Exit);
    }

    /// Does `after` right away, or with rebinds that are not saved, once the user has answered
    /// what to do with them.
    fn unless_unsaved(&mut self, after: AfterPrompt) {
        if self.rebinds_modified {
            self.ask_about_unsaved(after);
        } else {
            self.proceed(after);
        }
    }

    /// Asks what to do with the rebinds that are not saved, in front of any other message box,
    /// unless that is being asked already.
    fn ask_about_unsaved(&mut self, after: AfterPrompt) {
        let asking = self.dialogs.iter().any(|dialog| {
            matches!(dialog.confirm, Some((_, _, DialogAction::SaveThen(_) | DialogAction::SaveAsThen(_))))
        });
        if !asking {
            let path = self.document.as_ref().and_then(|doc| doc.rebind_path.as_deref());
            let dialog = unsaved_rebinds_warning(path.unwrap_or(Path::new("")), after, !self.can_save());
            self.dialogs.insert(0, dialog);
        }
    }

    /// Does what was asked, which discards any rebinds that are not saved. When a file does not
    /// open, the document stays, and so does `rebinds_modified`: only opening it clears that.
    fn proceed(&mut self, after: AfterPrompt) {
        match after {
            AfterPrompt::Exit => {
                self.rebinds_modified = false;
                self.ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            AfterPrompt::CloseFile => self.close_file(),
            AfterPrompt::OpenProfile(path) => {
                // A file picked by hand replaces a startup profile still waiting to open.
                self.startup_profile = None;
                self.startup_rebind = None;
                self.open_path(path);
            }
            AfterPrompt::OpenRebind(path) => self.open_rebind_path(path),
        }
    }

    /// Saves the rebind file and then does what was asked; if saving fails, says so and stops.
    fn save_then(&mut self, after: AfterPrompt) {
        if let Some(path) = self.rebind_path() {
            self.write_then(&path, after);
        }
    }

    /// Asks where to save the rebind file and then does what was asked. Cancelling the box
    /// stays in the application with the changes kept.
    fn save_as_then(&mut self, after: AfterPrompt, frame: &eframe::Frame) {
        if let Some(path) = self.pick_copy_path("Save Rebind File As", frame) {
            self.write_then(&path, after);
        }
    }

    /// Writes the open rebind file to `path` and does what was asked; if that fails, says so and
    /// stops.
    fn write_then(&mut self, path: &Path, after: AfterPrompt) {
        let Some(root) = self.document.as_ref().and_then(|doc| doc.rebind_root.as_ref()) else {
            return;
        };
        match xml_doc::save(root, path) {
            Ok(()) => {
                self.rebinds_modified = false;
                self.undo.mark_saved();
                self.proceed(after);
            }
            Err(err) => self.notify(Level::Error, format!("{err:#}")),
        }
    }

    /// Starts writing the rebind file to a new path, as a backup or a copy to edit later, by
    /// asking for the copy's header label, which names it in the game's list of control
    /// profiles; [`Self::save_rebind_copy_with_label`] goes on. The open rebind file stays the
    /// one Save Rebind File writes.
    fn save_rebind_copy_as(&mut self) {
        let Some(path) = self.rebind_path() else {
            return;
        };
        // Files in the game's profile format have no label; their file name stands in.
        let label = self
            .document
            .as_ref()
            .and_then(|doc| doc.rebind_root.as_ref())
            .and_then(header_label)
            .map(str::to_string)
            .unwrap_or_else(|| {
                let stem = path.file_stem().map(|stem| stem.to_string_lossy()).unwrap_or_default();
                stem.trim().chars().take(MAX_HEADER_LABEL).collect()
            });
        self.label_prompt = Some(LabelPrompt { text: label, focus: true });
    }

    /// Asks where to save the copy and writes it there with `label` in its header; the open
    /// rebind file is not changed.
    fn save_rebind_copy_with_label(&mut self, label: &str, frame: &eframe::Frame) {
        let Some(path) = self.pick_copy_path("Save Rebind Copy As", frame) else {
            return;
        };
        let Some(mut root) = self.document.as_ref().and_then(|doc| doc.rebind_root.clone()) else {
            return;
        };
        set_header_label(&mut root, label);
        match xml_doc::save(&root, &path) {
            Ok(()) => self.dialogs.push(Dialog::saved(format!("Saved copy to {}", path.display()), &path)),
            Err(err) => self.notify(Level::Error, format!("{err:#}")),
        }
    }

    /// Asks where to save a copy of the open rebind file, suggesting its folder and its name
    /// with `_copy` added; a warning when no rebind file is open.
    fn pick_copy_path(&mut self, title: &str, frame: &eframe::Frame) -> Option<PathBuf> {
        let rebind_path = self.rebind_path()?;
        let mut dialog = rfd::FileDialog::new()
            .set_title(title)
            .add_filter("XML files", &["xml"])
            .add_filter("All files", &["*"])
            .set_parent(frame);
        if let Some(dir) = rebind_path.parent() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(stem) = rebind_path.file_stem() {
            dialog = dialog.set_file_name(format!("{}_copy.xml", stem.to_string_lossy()));
        }
        dialog.save_file()
    }

    /// Writes a rebind file that clears every binding of the open profile: the game's
    /// [`game_dir::BLANK_LAYOUT`] with its header and devices kept, made to hold exactly
    /// the profile's actions (see [`complete_blank_layout`]). The file is saved, then a
    /// dialog offers to open it.
    fn create_blank_rebind_file(&mut self, frame: &eframe::Frame) {
        let Some(doc) = &self.document else {
            self.notify(Level::Warning, format!("Open the {MASTER_PROFILE} first (File > Open defaultProfile)"));
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
                let message = format!(
                    "Created {} from {}: added {added} actions from the {MASTER_PROFILE}, \
                     removed {removed} not in it.",
                    path.display(),
                    template_path.display()
                );
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
            self.notify(Level::Warning, format!("Open the {MASTER_PROFILE} first (File > Open defaultProfile)"));
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
            self.notify(Level::Warning, format!("Open the {MASTER_PROFILE} first (File > Open defaultProfile)"));
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
        self.rebinds_modified = false;
        self.undo = UndoStack::default();
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
        if filled > 0 {
            self.undo.push("autofilling the keyboard bindings".into(), rebind_root.clone(), self.rebinds_modified);
        }
        for ((map, action), combo) in unbound.iter().zip(&unused) {
            set_keyboard_rebind(rebind_root, map, action, &format!("kb1_{combo}"));
        }
        doc.rebinds = read_rebinds(rebind_root);
        self.rebinds_modified |= filled > 0;
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

    /// Shows the game's names for its controls in a window of its own, centered on the main
    /// window, bringing that window forward where it is when it is already open. The file comes
    /// from next to the open profile, else from the game folder.
    fn show_control_names(&mut self) {
        if self.control_names.is_some() {
            self.ctx.send_viewport_cmd_to(control_names::viewport_id(), ViewportCommand::Focus);
            return;
        }
        let file_name = Path::new(game_dir::CONTROL_NAMES).file_name().expect("a file name");
        let next_to_profile =
            self.document.as_ref().and_then(|doc| doc.path.parent()).map(|config| config.join(file_name));
        let in_game_dir = self.valid_game_dir().map(|dir| dir.join(game_dir::CONTROL_NAMES));
        let Some(path) = next_to_profile.into_iter().chain(in_game_dir).find(|path| path.is_file()) else {
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
        let root = match xml_doc::load(&path) {
            Ok(root) => root,
            Err(err) => {
                self.notify(Level::Error, format!("{err:#}"));
                return;
            }
        };
        let strings_path = find_strings_file(&path, self.game_dir.as_deref());
        let strings: Rc<Strings> = match &strings_path {
            Some(strings_path) => self.load_strings(strings_path),
            None => Rc::default(),
        };
        if strings.is_empty() {
            self.notify(Level::Warning, format!("No {STRINGS_FILE} texts found, so the names show their @keys"));
        }
        let names = ctrlxref::control_names::read(&root, &strings);
        let position = self.ctx.input(|input| {
            let main = input.viewport();
            Some(centered_position(main.outer_rect?, main.inner_rect?, control_names::SIZE))
        });
        self.control_names = Some(ControlNamesWindow::new(&names, position));
    }

    /// Shows the Show Control Names window in a native window of its own, while it is open.
    fn control_names_window(&mut self, ctx: &Context) {
        let Some(window) = &mut self.control_names else {
            return;
        };
        let mut builder = ViewportBuilder::default()
            .with_title(control_names::TITLE)
            .with_inner_size(control_names::SIZE)
            .with_min_inner_size([400., 300.]);
        if let Some(position) = window.position {
            builder = builder.with_position(position);
        }
        if let Some(icon) = &self.icon {
            builder = builder.with_icon(icon.clone());
        }
        let close = ctx.show_viewport_immediate(control_names::viewport_id(), builder, |ui, _| {
            window.show(ui);
            ui.input(|input| input.viewport().close_requested())
        });
        if close {
            self.control_names = None;
        }
    }

    /// Shows the help page `topic` in its own window, centered on the main window, bringing that
    /// window forward where it is when it is already open.
    fn show_help(&mut self, topic: HelpTopic) {
        if self.help_open.iter().any(|(open, _)| *open == topic) {
            self.ctx.send_viewport_cmd_to(topic.viewport_id(), ViewportCommand::Focus);
        } else {
            let position = self.ctx.input(|input| {
                let main = input.viewport();
                Some(centered_position(main.outer_rect?, main.inner_rect?, HELP_SIZE))
            });
            self.help_open.push((topic, position));
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
                    self.unless_unsaved(AfterPrompt::OpenRebind(path));
                }
            }
            DialogAction::SaveThen(after) => self.save_then(after),
            DialogAction::SaveAsThen(after) => self.save_as_after = Some(after),
            DialogAction::DiscardThen(after) => self.proceed(after),
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut picked = None;
        egui::MenuBar::new().ui(ui, |ui| {
            let can_extract = can_extract(self.game_dir.as_deref());
            picked = menus::menu_items(ui, self.theme, can_extract, self.can_merge(), self.can_save());
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
        let marked_fill = attribute_table::marked_color(ui);
        let mark = |text: RichText, marked: bool| if marked { text.background_color(marked_fill) } else { text };
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
                // Also for its actions' matches, which a collapsed actionmap hides.
                let map_marked = self.marked.contains(&map.id)
                    || (!self.marked.is_empty() && map.children.iter().any(|action| self.marked.contains(&action.id)));
                state
                    .show_header(ui, |ui| {
                        let response = ui
                            .selectable_label(self.selected.as_deref() == Some(&map.id), mark(map.text(), map_marked))
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
                            let text = mark(action.text(), self.marked.contains(&action.id));
                            let response = ui.selectable_label(is_selected, text).on_hover_text(action.name.as_str());
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
            self.undo_key(&ui.ctx().clone());
        }
        let scroll_to = self.scroll_table_to.take();
        let out = self.table.show(ui, selected, scroll_to);
        if self.table.take_marks_cleared() {
            self.clear_tree_marks();
        }
        if out.conflicts_resolved {
            self.notify(Level::Success, "No conflicting bindings are left. Save to keep this");
        }
        if let Some(column) = &out.clear_column {
            self.clear_column(column);
        }
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

    /// Shows the Save Rebind Copy As label box: the label starts selected, and a blank one or
    /// one longer than [`MAX_HEADER_LABEL`] characters cannot be saved. Enter saves, Escape
    /// cancels.
    fn label_prompt(&mut self, ctx: &Context) {
        if !self.dialogs.is_empty() {
            return;
        }
        let Some(prompt) = &mut self.label_prompt else {
            return;
        };
        let mut close = false;
        let mut accepted = None;
        let response = egui::Modal::new(Id::unique("copy label")).show(ctx, |ui| {
            ui.set_width(420.);
            ui.heading("Copy Label");
            ui.add_space(8.);
            ui.label(format!(
                "The name shown in the game's list of control profiles (1-{MAX_HEADER_LABEL} characters)."
            ));
            ui.add_space(8.);
            let error = header_label_error(&prompt.text);
            let mut output = TextEdit::singleline(&mut prompt.text)
                .id_salt("copy label text")
                .char_limit(MAX_HEADER_LABEL)
                .desired_width(f32::INFINITY)
                .show(ui);
            let id = output.response.response.id;
            // The modal's first frame only measures it, invisibly, and takes no focus; select
            // the label on the first frame it is shown.
            if prompt.focus && !ui.is_sizing_pass() {
                prompt.focus = false;
                output.response.response.request_focus();
                let all = CCursorRange::two(CCursor::new(0), CCursor::new(prompt.text.chars().count()));
                output.state.cursor.set_char_range(Some(all));
                output.state.store(ui.ctx(), id);
            }
            let entered = output.response.response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            ui.horizontal(|ui| {
                match error {
                    Some(error) => ui.label(RichText::new(error).color(ui.visuals().warn_fg_color)),
                    None => ui.label(""),
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("{}/{MAX_HEADER_LABEL}", prompt.text.trim().chars().count())).weak());
                });
            });
            ui.add_space(12.);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let save = ui.add_enabled(error.is_none(), egui::Button::new("Save Copy..."));
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if error.is_none() && (save.clicked() || entered) {
                    accepted = Some(prompt.text.trim().to_string());
                    close = true;
                } else if entered {
                    // Keep typing after Enter on a label that cannot be saved.
                    output.response.response.request_focus();
                }
            });
        });
        if close || response.should_close() {
            self.label_prompt = None;
        }
        if accepted.is_some() {
            self.pending_copy_label = accepted;
        }
    }

    /// Draws the notifications in the bottom-right corner, newest at the bottom; a click
    /// dismisses one. The warning about unmatched rebinds has buttons to review or dismiss it.
    fn toasts(&mut self, ctx: &Context) {
        let now = Instant::now();
        self.toasts.retain(|toast| toast.until.is_none_or(|until| until > now));
        if self.toasts.is_empty() {
            return;
        }
        let mut dismissed = None;
        let mut review = false;
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
                    let frame = egui::Frame::popup(ui.style()).fill(fill).show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&toast.text).color(Color32::WHITE))
                                .wrap()
                                .selectable(false),
                        );
                        if toast.review {
                            ui.add_space(4.);
                            ui.horizontal(|ui| {
                                if ui.button("Review...").on_hover_text("Move or remove these bindings").clicked() {
                                    review = true;
                                }
                                let hover = "Hide this warning; the bindings stay in the file. \
                                             Tools > Resolve Unmatched Rebinds... opens the list again";
                                if ui.button("Ignore and keep").on_hover_text(hover).clicked() {
                                    dismissed = Some(ix);
                                }
                            });
                        }
                    });
                    // A click on the whole box would take the buttons' clicks.
                    if !toast.review && frame.response.interact(egui::Sense::click()).clicked() {
                        dismissed = Some(ix);
                    }
                }
            });
        if let Some(ix) = dismissed {
            self.toasts.remove(ix);
        }
        if review {
            self.show_unmatched();
        }
        if let Some(next) = self.toasts.iter().filter_map(|toast| toast.until).min() {
            ctx.request_repaint_after(next.saturating_duration_since(now));
        }
    }

    /// Shows the Unmatched Rebinds box: each rebound action the profile lacks, with buttons that
    /// move its rebinds to a suggested profile action or remove them.
    fn unmatched_prompt(&mut self, ctx: &Context) {
        if !self.dialogs.is_empty() || self.label_prompt.is_some() {
            return;
        }
        let Some(rows) = &self.unmatched else {
            return;
        };
        let mut change = None;
        let mut close = false;
        let response = egui::Modal::new(Id::unique("unmatched rebinds")).show(ctx, |ui| {
            ui.set_width(720.);
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠").size(20.).color(ui.visuals().warn_fg_color));
                ui.heading("Unmatched Rebinds");
            });
            ui.add_space(8.);
            ui.label(format!(
                "{} rebound actions are not in the {MASTER_PROFILE} (defaultProfile.xml from Data.p4k), \
                 which is the master: the game ignores bindings of actions it lacks. The rebind file was \
                 probably made for an older game version, and these actions were renamed or moved to another \
                 actionmap since. Move their bindings to a suggested action of the {MASTER_PROFILE}, or remove \
                 them.",
                rows.len()
            ));
            ui.add_space(8.);
            ScrollArea::vertical().max_height(420.).auto_shrink([false, true]).show(ui, |ui| {
                for row in rows {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("{}/{}", row.key.0, row.key.1)).strong());
                        ui.label(RichText::new(row.inputs.join(BINDING_SEPARATOR)).monospace());
                    });
                    ui.horizontal_wrapped(|ui| {
                        for (suggestion, conflicts) in &row.suggestions {
                            let why = match suggestion.reason {
                                MatchReason::SameName => "same name".to_string(),
                                MatchReason::Similar(percent) => format!("~{percent}%"),
                            };
                            let (map, action) = &suggestion.target;
                            let warn = if *conflicts { "⚠ " } else { "" };
                            let hover = if *conflicts {
                                "Move the bindings to this action, which already has rebinds of the same device: \
                                 both are kept and shown in red, to choose from in the table"
                            } else {
                                "Move the bindings to this action"
                            };
                            if ui.button(format!("{warn}→ {map}/{action} ({why})")).on_hover_text(hover).clicked() {
                                change = Some(UnmatchedChange::Move(row.key.clone(), suggestion.target.clone()));
                            }
                        }
                        if row.suggestions.is_empty() {
                            ui.label(RichText::new("(no similar action)").weak());
                        }
                        if ui.button("Remove").on_hover_text("Remove these bindings from the rebind file").clicked() {
                            change = Some(UnmatchedChange::Remove(row.key.clone()));
                        }
                    });
                }
            });
            ui.add_space(12.);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let any_suggestion = rows.iter().any(|row| !row.suggestions.is_empty());
                if ui
                    .add_enabled(any_suggestion, egui::Button::new("Apply All Suggestions"))
                    .on_hover_text("Moves the bindings of each action to its first suggestion")
                    .clicked()
                {
                    change = Some(UnmatchedChange::ApplyAll);
                }
                if ui.button("Remove All").on_hover_text("Removes all these bindings from the rebind file").clicked() {
                    change = Some(UnmatchedChange::RemoveAll);
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
        if close || response.should_close() {
            self.unmatched = None;
        }
        if let Some(change) = change {
            self.apply_unmatched(change);
        }
    }

    /// Shows each open help page in a native window of its own.
    fn help_windows(&mut self, ctx: &Context) {
        for topic in HelpTopic::ALL {
            // Kept from when the window opened, so it stays where the user moves it.
            let Some(position) = self.help_open.iter().find(|(open, _)| *open == topic).map(|(_, position)| *position)
            else {
                continue;
            };
            let mut builder = ViewportBuilder::default()
                .with_title(topic.title())
                .with_inner_size(HELP_SIZE)
                .with_min_inner_size([400., 300.]);
            if let Some(position) = position {
                builder = builder.with_position(position);
            }
            if let Some(icon) = &self.icon {
                builder = builder.with_icon(icon.clone());
            }
            let cache = &mut self.help_cache;
            let close = ctx.show_viewport_immediate(topic.viewport_id(), builder, |ui, _| {
                help::show(ui, topic, cache);
                ui.input(|input| input.viewport().close_requested())
            });
            if close {
                self.help_open.retain(|(open, _)| *open != topic);
            }
        }
    }
}

impl eframe::App for CtrlXrefApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) {
            self.on_close_requested();
        }
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
        self.label_prompt(&ctx);
        self.unmatched_prompt(&ctx);
        self.toasts(&ctx);
        self.help_windows(&ctx);
        self.control_names_window(&ctx);
        if let Some(command) = command {
            self.run(command, frame);
        }
        if let Some(after) = self.save_as_after.take() {
            self.save_as_then(after, frame);
        }
        if let Some(label) = self.pending_copy_label.take() {
            self.save_rebind_copy_with_label(&label, frame);
        }
    }
}

/// `(actionmap name, action name)` of each of the profile's actions, in table order.
fn profile_actions(doc: &Document) -> Vec<(String, String)> {
    doc.rows.iter().filter_map(|path| action_key(&doc.root, path)).collect()
}

/// The rows of the Unmatched Rebinds box: the open rebinds of actions the profile lacks, sorted,
/// each with the profile actions suggested for them.
fn unmatched_rows(doc: &Document) -> Vec<UnmatchedRow> {
    let actions = profile_actions(doc);
    let bound_devices = |inputs: &[String]| -> HashSet<&'static str> {
        inputs.iter().filter(|input| !is_empty_rebind(input)).filter_map(|input| rebind_device(input)).collect()
    };
    unmatched_actions(&doc.rebinds, &actions)
        .into_iter()
        .map(|key| {
            let inputs = doc.rebinds.get(&key).cloned().unwrap_or_default();
            let devices = bound_devices(&inputs);
            let suggestions = suggest_targets(&key, &actions, MAX_SUGGESTIONS)
                .into_iter()
                .map(|suggestion| {
                    let has = doc.rebinds.get(&suggestion.target).map(|have| bound_devices(have)).unwrap_or_default();
                    let conflicts = !has.is_disjoint(&devices);
                    (suggestion, conflicts)
                })
                .collect();
            UnmatchedRow { key, inputs, suggestions }
        })
        .collect()
}

/// Where a window with content of `size` goes, centered on the main window, given the main
/// window with its frame (`outer`) and without (`inner`). The window's frame is taken to be like
/// the main window's. A window taller than the main one starts at the main window's top instead
/// of above it, so its title bar stays as reachable.
fn centered_position(outer: egui::Rect, inner: egui::Rect, size: egui::Vec2) -> egui::Pos2 {
    let framed = size + (outer.size() - inner.size());
    let centered = outer.center() - framed / 2.;
    egui::pos2(centered.x, centered.y.max(outer.top()))
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

/// Asks what to do with the changes to the rebind file at `path` before doing `after`. The game's
/// own current rebind file gets the warning that saving it has. Exiting offers Save As, since it is
/// not always clear which file is open, and so does everything after a `merged` file.
fn unsaved_rebinds_warning(path: &Path, after: AfterPrompt, merged: bool) -> Dialog {
    let mut message = format!("The rebindings have been modified but not saved:\n{}", path.display());
    if is_current_rebind(path) {
        message.push_str(
            "\n\nThis is your current actionmaps.xml. It will be overwritten by the game engine when you \
             modify controls inside the game.",
        );
    }
    message.push_str(&format!("\n\nSave them before {}?", after.question()));
    let mut dialog = Dialog::warning("Unsaved rebindings", message);
    dialog.extra = Some((after.discard_label(), DialogAction::DiscardThen(after.clone())));
    dialog.confirm = Some(if merged || after == AfterPrompt::Exit {
        ("Save As...", "Cancel", DialogAction::SaveAsThen(after))
    } else {
        ("Save", "Cancel", DialogAction::SaveThen(after))
    });
    dialog
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

pub(super) fn panel_header(ui: &mut egui::Ui, title: String) {
    ui.add_space(2.);
    ui.label(theme::heading_text(&title));
    ui.separator();
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
        AfterPrompt, DialogAction, CellStyle, DEFAULT_COLUMN, DEVICES, Rebinds, TableRow, action_key, actionmap_tree, action_paths, actionmap_item, count_elements, readable_name,
        find_actionmaps, is_current_rebind, parse_tree_id, read_rebinds,
        strings_candidates,
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

    #[test]
    fn control_names_come_from_next_to_the_profile_and_warn_when_missing() {
        let (mut app, dir, _) = app_with_rebinds("control-names");
        app.game_dir = None;
        app.toasts.clear();

        app.show_control_names();
        assert!(app.control_names.is_none());
        assert_eq!(app.toasts.len(), 1);
        assert!(app.toasts[0].level == super::Level::Warning);

        let file = dir.join("keybinding_localization.xml");
        std::fs::write(&file, r#"<LocalizedKeys><device name="keyboard"><Key name="a"/></device></LocalizedKeys>"#).unwrap();
        app.show_control_names();
        assert!(app.control_names.is_some());

        // Not valid XML, like a file that was not converted when it was extracted.
        app.control_names = None;
        app.toasts.clear();
        std::fs::write(&file, b"CryXmlB\0...").unwrap();
        app.show_control_names();
        assert!(app.control_names.is_none());
        assert!(app.toasts.iter().any(|toast| toast.level == super::Level::Error));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn help_windows_open_centered_on_the_main_window() {
        use eframe::egui::{Rect, pos2, vec2};
        // A frame of 8 at the sides and the bottom, and a 32 high title bar.
        let frame = |x: f32, y: f32, width: f32, height: f32| {
            let inner = Rect::from_min_size(pos2(x, y), vec2(width, height));
            let outer = Rect::from_min_max(inner.min - vec2(8., 32.), inner.max + vec2(8., 8.));
            (outer, inner)
        };

        // Taller than the help window, whose frame is taken to be the same size.
        let (outer, inner) = frame(100., 50., 1200., 900.);
        let position = super::centered_position(outer, inner, vec2(760., 720.));
        let help = Rect::from_min_size(position, vec2(760., 720.) + vec2(16., 40.));
        assert_eq!(help.center(), outer.center());

        // Lower than the help window: it starts at the same top, still centered sideways.
        let (outer, inner) = frame(100., 50., 1200., 400.);
        let position = super::centered_position(outer, inner, vec2(760., 720.));
        assert_eq!(position.y, outer.top());
        assert_eq!(position.x + (760. + 16.) / 2., outer.center().x);

        // Narrower: it overlaps both sides evenly.
        let (outer, inner) = frame(500., 50., 640., 900.);
        let position = super::centered_position(outer, inner, vec2(760., 720.));
        assert_eq!(position.x + (760. + 16.) / 2., outer.center().x);
    }

    /// A running app, with no window, with a profile and a rebind file from a new temp folder
    /// of its own, which it returns with the rebind file's path.
    fn app_with_rebinds(test: &str) -> (super::CtrlXrefApp, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ctrlxref-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let profile = dir.join("profile.xml");
        std::fs::write(&profile, r#"<p><actionmap name="m"><action name="a" keyboard="kb1_a"/><action name="b"/></actionmap></p>"#)
            .unwrap();
        let rebind = dir.join("rebinds.xml");
        std::fs::write(&rebind, r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="kb1_x"/></action></actionmap></ActionMaps>"#)
            .unwrap();
        let mut app = super::CtrlXrefApp::new(None, None, None, &eframe::egui::Context::default());
        app.startup_profile = None;
        app.open_path(profile);
        app.open_rebind_path(rebind.clone());
        assert!(app.toasts.iter().all(|toast| toast.level == super::Level::Success), "{:?}", app.toasts.len());
        (app, dir, rebind)
    }

    /// An edit of cell `column` of the action at source index `source_ix`.
    fn edit(app: &mut super::CtrlXrefApp, source_ix: usize, column: &str, value: &str) {
        app.apply_cell_edit(super::CellEdit { source_ix, column: column.into(), value: value.into() });
    }

    /// The close commands aside: whether a close request now asks about unsaved rebinds.
    fn close_asks(app: &mut super::CtrlXrefApp) -> bool {
        app.on_close_requested();
        asked(app) == Some(AfterPrompt::Exit)
    }

    /// What the question about unsaved rebinds, if it is up, is going to do once answered.
    fn asked(app: &super::CtrlXrefApp) -> Option<AfterPrompt> {
        match app.dialogs.first().and_then(|dialog| dialog.confirm.as_ref()) {
            Some((_, _, DialogAction::SaveThen(after) | DialogAction::SaveAsThen(after))) => Some(after.clone()),
            _ => None,
        }
    }

    /// Answers the question about unsaved rebinds: saving, or going on without that.
    fn answer(app: &mut super::CtrlXrefApp, save: bool) {
        let after = asked(app).expect("the question is up");
        app.dialogs.remove(0);
        app.run_dialog_action(if save { DialogAction::SaveThen(after) } else { DialogAction::DiscardThen(after) });
    }

    /// The open rebinds, as sorted `action: input` texts.
    fn rebinds_of(app: &super::CtrlXrefApp) -> Vec<String> {
        let doc = app.document.as_ref().unwrap();
        let mut all: Vec<String> = doc
            .rebinds
            .iter()
            .flat_map(|((_, action), inputs)| inputs.iter().map(move |input| format!("{action}: {input}")))
            .collect();
        all.sort();
        all
    }

    #[test]
    fn clearing_a_column_removes_only_that_device_and_undo_brings_it_back() {
        let (mut app, dir, _) = app_with_rebinds("clear-column");
        edit(&mut app, 1, "keyboard", "kb1_b | kb1_c");
        edit(&mut app, 0, "mouse", "mo1_m");
        let before = rebinds_of(&app);
        assert_eq!(before, ["a: kb1_x", "a: mo1_m", "b: kb1_b", "b: kb1_c"]);
        let root_before = app.document.as_ref().unwrap().rebind_root.clone();

        app.clear_column("keyboard");
        assert_eq!(rebinds_of(&app), ["a: mo1_m"]);
        assert!(app.rebinds_modified);

        app.undo_change();
        assert_eq!(rebinds_of(&app), before);
        // The whole file is back as it was, order and all.
        assert_eq!(app.document.as_ref().unwrap().rebind_root, root_before);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn clearing_an_empty_column_is_no_change() {
        let (mut app, dir, _) = app_with_rebinds("clear-empty");
        app.clear_column("gamepad");
        assert!(!app.rebinds_modified);
        assert!(app.undo.0.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn clearing_leaves_rebinds_of_actions_not_in_the_profile() {
        let (mut app, dir, rebind) = app_with_rebinds("clear-unmatched");
        std::fs::write(
            &rebind,
            r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="kb1_x"/></action><action name="gone"><rebind input="kb1_g"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        app.open_rebind_path(rebind);
        app.clear_column("keyboard");
        assert_eq!(rebinds_of(&app), ["gone: kb1_g"]);
        std::fs::remove_dir_all(dir).ok();
    }

    /// Opens a rebind file in `app` with an action moved to another actionmap (`old/b`, which the
    /// profile has as `m/b`) and one the profile has nothing like (`m/gone`).
    fn open_unmatched(app: &mut super::CtrlXrefApp, rebind: &Path) {
        std::fs::write(
            rebind,
            r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="kb1_x"/></action><action name="gone"><rebind input="kb1_g"/></action></actionmap><actionmap name="old"><action name="b"><rebind input="kb1_b" multiTap="2"/></action></actionmap></ActionMaps>"#,
        )
        .unwrap();
        app.toasts.clear();
        app.open_rebind_path(rebind.to_path_buf());
    }

    #[test]
    fn unmatched_rebinds_warn_until_resolved_with_suggestions() {
        let (mut app, dir, rebind) = app_with_rebinds("unmatched-warn");
        open_unmatched(&mut app, &rebind);
        let sticky: Vec<_> = app.toasts.iter().filter(|toast| toast.review).collect();
        assert_eq!(sticky.len(), 1);
        assert!(sticky[0].until.is_none() && sticky[0].level == super::Level::Warning);

        app.show_unmatched();
        let rows = app.unmatched.as_ref().expect("the box is open");
        let keys: Vec<_> = rows.iter().map(|row| format!("{}/{}", row.key.0, row.key.1)).collect();
        assert_eq!(keys, ["m/gone", "old/b"]);
        assert!(rows[0].suggestions.is_empty());
        assert_eq!(rows[1].suggestions[0].0.target, ("m".to_string(), "b".to_string()));
        assert_eq!(rows[1].suggestions[0].0.reason, super::MatchReason::SameName);

        // Moving puts the binding in the profile's row, and the box lists what is left.
        app.apply_unmatched(super::UnmatchedChange::ApplyAll);
        assert_eq!(rebinds_of(&app), ["a: kb1_x", "b: kb1_b", "gone: kb1_g"]);
        assert!(app.rebinds_modified);
        assert_eq!(app.unmatched.as_ref().unwrap().len(), 1);
        assert!(app.toasts.iter().any(|toast| toast.review));
        let xml = xml_doc::to_string(app.document.as_ref().unwrap().rebind_root.as_ref().unwrap()).unwrap();
        assert!(xml.contains("multiTap"), "{xml}");

        // Removing the last one closes the box and the warning.
        app.apply_unmatched(super::UnmatchedChange::Remove(("m".into(), "gone".into())));
        assert_eq!(rebinds_of(&app), ["a: kb1_x", "b: kb1_b"]);
        assert!(app.unmatched.is_none());
        assert!(!app.toasts.iter().any(|toast| toast.review));

        // Each was one step back.
        app.undo_change();
        app.undo_change();
        assert_eq!(rebinds_of(&app), ["a: kb1_x", "b: kb1_b", "gone: kb1_g"]);
        assert!(!app.rebinds_modified);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn remove_all_leaves_only_the_profiles_actions() {
        let (mut app, dir, rebind) = app_with_rebinds("unmatched-remove-all");
        open_unmatched(&mut app, &rebind);
        app.show_unmatched();
        app.apply_unmatched(super::UnmatchedChange::RemoveAll);
        assert_eq!(rebinds_of(&app), ["a: kb1_x"]);
        assert!(app.unmatched.is_none());
        assert!(!app.toasts.iter().any(|toast| toast.review));
        assert_eq!(app.undo.0.len(), 1);

        // With nothing unmatched, the menu item only says so.
        app.show_unmatched();
        assert!(app.unmatched.is_none());
        assert!(app.toasts.last().is_some_and(|toast| toast.level == super::Level::Info));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn undo_steps_back_through_every_kind_of_change() {
        let (mut app, dir, _) = app_with_rebinds("undo-steps");
        let start = rebinds_of(&app);
        edit(&mut app, 0, "keyboard", "kb1_y");
        let edited = rebinds_of(&app);
        app.autofill_keyboard();
        let filled = rebinds_of(&app);
        assert_ne!(filled, edited);
        app.clear_column("keyboard");
        assert_eq!(app.undo.0.len(), 3);

        app.undo_change();
        assert_eq!(rebinds_of(&app), filled);
        app.undo_change();
        assert_eq!(rebinds_of(&app), edited);
        assert!(app.rebinds_modified);
        app.undo_change();
        assert_eq!(rebinds_of(&app), start);
        // Back where the file was opened: nothing to save.
        assert!(!app.rebinds_modified);

        // Nothing left, and that changes nothing.
        app.undo_change();
        assert_eq!(rebinds_of(&app), start);
        assert!(!app.rebinds_modified);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn merging_adds_to_the_open_file_and_undo_takes_it_out() {
        let (mut app, dir, rebind) = app_with_rebinds("merge");
        let other = dir.join("other.xml");
        std::fs::write(&other, r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="kb1_y"/></action></actionmap></ActionMaps>"#)
            .unwrap();
        app.merge_rebind_path(&other);
        assert_eq!(rebinds_of(&app), ["a: kb1_x", "a: kb1_y"]);
        assert!(app.rebinds_modified);
        // The rebind file stays the one that saving writes.
        assert_eq!(app.document.as_ref().unwrap().rebind_path.as_ref(), Some(&rebind));
        app.undo_change();
        assert_eq!(rebinds_of(&app), ["a: kb1_x"]);
        assert!(!app.rebinds_modified);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `app` with a file holding another binding of action `a` merged into its rebind file.
    fn merge_other(app: &mut super::CtrlXrefApp, dir: &Path, input: &str) {
        let other = dir.join("other.xml");
        let xml = format!(r#"<ActionMaps><actionmap name="m"><action name="a"><rebind input="{input}"/></action></actionmap></ActionMaps>"#);
        std::fs::write(&other, xml).unwrap();
        app.merge_rebind_path(&other);
    }

    #[test]
    fn a_merge_turns_save_off_until_it_is_undone_or_another_file_is_opened() {
        let (mut app, dir, rebind) = app_with_rebinds("merge-save");
        assert!(app.can_save());
        merge_other(&mut app, &dir, "kb1_y");
        merge_other(&mut app, &dir, "kb1_z");
        assert!(!app.can_save());

        // Not even with Ctrl+S, whose command doesn't go through the menu.
        app.save_rebind_file();
        assert!(app.dialogs.is_empty());
        assert!(!std::fs::read_to_string(&rebind).unwrap().contains("kb1_y"));

        // The first merge has to be undone too.
        app.undo_change();
        assert!(!app.can_save());
        app.undo_change();
        assert!(app.can_save());

        merge_other(&mut app, &dir, "kb1_y");
        assert!(!app.can_save());
        app.open_rebind_path(rebind);
        assert!(app.can_save());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn merging_nothing_new_leaves_save_on() {
        let (mut app, dir, _) = app_with_rebinds("merge-nothing");
        merge_other(&mut app, &dir, "kb1_x");
        assert!(app.can_save());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn after_a_merge_every_question_offers_save_as() {
        let (mut app, dir, _) = app_with_rebinds("merge-question");
        edit(&mut app, 0, "keyboard", "kb1_y");
        app.unless_unsaved(AfterPrompt::CloseFile);
        assert!(matches!(app.dialogs[0].confirm, Some(("Save", _, DialogAction::SaveThen(_)))));
        app.dialogs.clear();
        merge_other(&mut app, &dir, "kb1_z");
        app.unless_unsaved(AfterPrompt::CloseFile);
        assert!(matches!(app.dialogs[0].confirm, Some(("Save As...", _, DialogAction::SaveAsThen(_)))));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_cell_edit_cleans_up_the_text_and_keeps_the_prefix() {
        let (mut app, dir, _) = app_with_rebinds("clean-edit");
        edit(&mut app, 1, "joystick", "js3_button1 | js3_button2");
        // The second control's prefix got damaged and a separator was left behind.
        edit(&mut app, 1, "joystick", "js3_button1 | s3_button2 |");
        assert_eq!(rebinds_of(&app), ["a: kb1_x", "b: js3_button1", "b: js3_button2"]);
        edit(&mut app, 1, "joystick", "| js3_button2");
        assert_eq!(rebinds_of(&app), ["a: kb1_x", "b: js3_button2"]);
        // What can't be made valid warns and changes nothing.
        edit(&mut app, 1, "joystick", "js3");
        assert_eq!(rebinds_of(&app), ["a: kb1_x", "b: js3_button2"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn merging_is_possible_only_with_an_open_rebind_file() {
        let (mut app, dir, _) = app_with_rebinds("can-merge");
        assert!(app.can_merge());
        app.open_path(dir.join("profile.xml"));
        assert!(!app.can_merge());
        app.open_rebind_path(dir.join("missing.xml"));
        assert!(!app.can_merge());
        app.open_rebind_path(dir.join("rebinds.xml"));
        assert!(app.can_merge());
        app.close_file();
        assert!(!app.can_merge());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn undoing_past_a_save_counts_as_a_change_to_save() {
        let (mut app, dir, rebind) = app_with_rebinds("undo-save");
        app.clear_column("keyboard");
        app.write_rebind_file(&rebind);
        assert!(!app.rebinds_modified);
        assert!(!std::fs::read_to_string(&rebind).unwrap().contains("kb1_x"));

        // The file no longer has it, so bringing it back is something to save.
        app.undo_change();
        assert_eq!(rebinds_of(&app), ["a: kb1_x"]);
        assert!(app.rebinds_modified);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn opening_or_closing_a_file_forgets_the_undo_steps() {
        let (mut app, dir, rebind) = app_with_rebinds("undo-forget");
        app.clear_column("keyboard");
        app.open_rebind_path(rebind);
        assert!(app.undo.0.is_empty());
        app.clear_column("keyboard");
        app.close_file();
        assert!(app.undo.0.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn undo_keeps_only_the_latest_steps() {
        let mut undo = super::UndoStack::default();
        for ix in 0..super::MAX_UNDO + 5 {
            undo.push(ix.to_string(), xmltree::Element::new("a"), false);
        }
        assert_eq!(undo.0.len(), super::MAX_UNDO);
        assert_eq!(undo.0[0].label, "5");
    }

    #[test]
    fn closing_asks_about_unsaved_rebinds() {
        let (mut app, dir, rebind) = app_with_rebinds("close-prompt");
        edit(&mut app, 0, "keyboard", "kb1_y");

        // Cancel is the dialog's own: it only takes the question away.
        app.unless_unsaved(AfterPrompt::CloseFile);
        assert_eq!(asked(&app), Some(AfterPrompt::CloseFile));
        assert!(app.document.is_some());
        let dialog = &app.dialogs[0];
        assert!(dialog.message.ends_with("Save them before closing the file?"), "{}", dialog.message);
        assert_eq!(dialog.extra.as_ref().map(|(label, _)| *label), Some("Close Without Saving"));

        // Without saving.
        answer(&mut app, false);
        assert!(app.document.is_none());
        assert!(!app.rebinds_modified);
        assert!(!std::fs::read_to_string(&rebind).unwrap().contains("kb1_y"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn saving_at_the_question_saves_and_goes_on() {
        let (mut app, dir, rebind) = app_with_rebinds("close-save");
        edit(&mut app, 0, "keyboard", "kb1_y");
        app.unless_unsaved(AfterPrompt::CloseFile);
        answer(&mut app, true);
        assert!(app.document.is_none());
        assert!(std::fs::read_to_string(&rebind).unwrap().contains("kb1_y"));
        // No other message box gets in the way.
        assert!(app.dialogs.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn without_changes_files_close_and_open_right_away() {
        let (mut app, dir, rebind) = app_with_rebinds("no-prompt");
        app.unless_unsaved(AfterPrompt::OpenRebind(rebind));
        assert!(app.dialogs.is_empty());
        app.unless_unsaved(AfterPrompt::CloseFile);
        assert!(app.dialogs.is_empty());
        assert!(app.document.is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn opening_another_rebind_file_asks_and_then_opens_it() {
        let (mut app, dir, rebind) = app_with_rebinds("open-rebind-prompt");
        let other = dir.join("other.xml");
        std::fs::write(&other, r#"<ActionMaps><actionmap name="m"><action name="b"><rebind input="kb1_o"/></action></actionmap></ActionMaps>"#)
            .unwrap();
        edit(&mut app, 0, "keyboard", "kb1_y");

        app.unless_unsaved(AfterPrompt::OpenRebind(other.clone()));
        assert_eq!(asked(&app), Some(AfterPrompt::OpenRebind(other.clone())));
        let dialog = &app.dialogs[0];
        assert!(dialog.message.ends_with("Save them before opening the other file?"), "{}", dialog.message);
        assert_eq!(dialog.extra.as_ref().map(|(label, _)| *label), Some("Open Without Saving"));
        // Still the old file, until answered.
        assert_eq!(app.document.as_ref().unwrap().rebind_path.as_ref(), Some(&rebind));

        answer(&mut app, true);
        assert!(std::fs::read_to_string(&rebind).unwrap().contains("kb1_y"));
        assert_eq!(app.document.as_ref().unwrap().rebind_path.as_ref(), Some(&other));
        assert!(!app.rebinds_modified);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn opening_another_profile_asks_and_then_opens_it() {
        let (mut app, dir, rebind) = app_with_rebinds("open-profile-prompt");
        let other = dir.join("other_profile.xml");
        std::fs::write(&other, r#"<p><actionmap name="n"><action name="c"/></actionmap></p>"#).unwrap();
        edit(&mut app, 0, "keyboard", "kb1_y");

        app.unless_unsaved(AfterPrompt::OpenProfile(other.clone()));
        assert_eq!(asked(&app), Some(AfterPrompt::OpenProfile(other.clone())));
        answer(&mut app, false);
        let doc = app.document.as_ref().unwrap();
        assert_eq!(doc.path, other);
        assert!(doc.rebind_root.is_none());
        assert!(!app.rebinds_modified);
        assert!(!std::fs::read_to_string(&rebind).unwrap().contains("kb1_y"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_file_that_does_not_open_keeps_the_changes() {
        let (mut app, dir, rebind) = app_with_rebinds("failed-open");
        edit(&mut app, 0, "keyboard", "kb1_y");
        let missing = dir.join("missing.xml");

        for after in [AfterPrompt::OpenRebind(missing.clone()), AfterPrompt::OpenProfile(missing.clone())] {
            app.unless_unsaved(after);
            answer(&mut app, false);
            let doc = app.document.as_ref().unwrap();
            assert_eq!(doc.rebind_path.as_ref(), Some(&rebind));
            assert!(doc.rebinds.values().flatten().any(|input| input == "kb1_y"));
            // So the next exit still asks.
            assert!(app.rebinds_modified);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_blank_layouts_open_button_asks_too() {
        let (mut app, dir, _) = app_with_rebinds("created-open");
        let created = dir.join("created.xml");
        std::fs::write(&created, "<ActionMaps/>").unwrap();
        edit(&mut app, 0, "keyboard", "kb1_y");
        app.run_dialog_action(DialogAction::OpenRebindFile(created.clone()));
        assert_eq!(asked(&app), Some(AfterPrompt::OpenRebind(created)));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn only_real_changes_to_the_rebinds_ask_before_exiting() {
        let (mut app, dir, _) = app_with_rebinds("modified-flag");
        assert!(!close_asks(&mut app));

        // The same binding again, typed without its prefix.
        edit(&mut app, 0, "keyboard", "x");
        assert!(!app.rebinds_modified);

        edit(&mut app, 0, "keyboard", "kb1_y");
        assert!(app.rebinds_modified);
        assert!(close_asks(&mut app));

        // Asking again doesn't stack the question.
        assert!(close_asks(&mut app));
        assert_eq!(app.dialogs.len(), 1);

        // Reopening the rebind file drops the changes.
        let path = app.document.as_ref().unwrap().rebind_path.clone().unwrap();
        app.dialogs.clear();
        app.open_rebind_path(path);
        assert!(!app.rebinds_modified);
        assert!(!close_asks(&mut app));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn autofill_counts_as_a_change_and_saving_clears_it() {
        let (mut app, dir, rebind) = app_with_rebinds("autofill-flag");
        app.autofill_keyboard();
        assert!(app.rebinds_modified);

        app.write_rebind_file(&rebind);
        assert!(!app.rebinds_modified);
        assert!(std::fs::read_to_string(&rebind).unwrap().contains(r#"name="b""#));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_failed_save_stays_modified() {
        let (mut app, dir, _) = app_with_rebinds("failed-save-flag");
        edit(&mut app, 0, "keyboard", "kb1_y");
        // A folder can't be written as a file.
        app.write_rebind_file(&dir);
        assert!(app.rebinds_modified);
        app.document.as_mut().unwrap().rebind_path = Some(dir.clone());
        app.save_then(AfterPrompt::CloseFile);
        assert!(app.rebinds_modified);
        // It didn't go on.
        assert!(app.document.is_some());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_exit_question_saves_or_leaves_the_file() {
        let (mut app, dir, rebind) = app_with_rebinds("exit-actions");
        edit(&mut app, 0, "keyboard", "kb1_y");
        assert!(close_asks(&mut app));
        let dialog = &app.dialogs[0];
        assert_eq!(dialog.confirm.as_ref().map(|(ok, cancel, _)| (*ok, *cancel)), Some(("Save As...", "Cancel")));
        assert_eq!(dialog.extra.as_ref().map(|(label, _)| *label), Some("Exit Without Saving"));

        // Without saving: the file stays as it was, and the next close request goes through.
        app.run_dialog_action(DialogAction::DiscardThen(AfterPrompt::Exit));
        assert!(!app.rebinds_modified);
        assert!(!std::fs::read_to_string(&rebind).unwrap().contains("kb1_y"));

        // Save As: the box is asked for, the chosen file has the change, the open one doesn't,
        // and no other message box gets in the way of exiting.
        app.dialogs.clear();
        edit(&mut app, 0, "keyboard", "kb1_z");
        app.run_dialog_action(DialogAction::SaveAsThen(AfterPrompt::Exit));
        assert_eq!(app.save_as_after.take(), Some(AfterPrompt::Exit));
        let chosen = dir.join("chosen.xml");
        app.write_then(&chosen, AfterPrompt::Exit);
        assert!(!app.rebinds_modified);
        assert!(std::fs::read_to_string(&chosen).unwrap().contains("kb1_z"));
        assert!(!std::fs::read_to_string(&rebind).unwrap().contains("kb1_z"));
        assert!(app.dialogs.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn the_exit_question_warns_about_the_games_own_file() {
        let own =
            super::unsaved_rebinds_warning(Path::new(r"C:\SC\LIVE\USER\Client\0\Profiles\default\actionmaps.xml"), AfterPrompt::Exit, false);
        assert!(own.message.contains("overwritten by the game engine"));
        let other = super::unsaved_rebinds_warning(Path::new(r"C:\SC\mappings\layout.xml"), AfterPrompt::CloseFile, false);
        assert!(!other.message.contains("overwritten"));
        assert!(other.message.contains("layout.xml"));
    }

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
