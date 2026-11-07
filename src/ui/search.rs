//! The Search dialog and the search's part of the status bar.

use ctrlxref::search::{self, Found, Place};
use eframe::egui::text::CCursor;
use eframe::egui::text_selection::CCursorRange;
use eframe::egui::{self, Color32, Key, RichText, ScrollArea, TextEdit};
use regex::Regex;

/// Background of the status bar's warning, like a warning notification's.
const WARNING_BG: Color32 = Color32::from_rgb(0x9a, 0x58, 0x00);

/// How much of the dialog's background shows while the user is elsewhere: enough to tell
/// that values are under it, while they stay hard to read.
const INACTIVE_FILL: f32 = 0.82;

/// How much of the dialog's own text and controls shows while the user is elsewhere.
const INACTIVE_CONTENT: f32 = 0.75;

/// What the user did in the dialog, for the owner to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchAction {
    /// The search text changed, so the counts must be updated.
    Changed,
    FindNext,
    FindPrevious,
    MarkAll,
}

/// How the search text turned out.
enum Pattern {
    Blank,
    Invalid(String),
    Valid(Regex),
}

pub struct SearchDialog {
    open: bool,
    text: String,
    pattern: Pattern,
    /// Previous searches, latest first, loaded when the dialog first opens.
    history: Option<Vec<String>>,
    /// Set until the text field has taken the keyboard focus.
    focus: bool,
    /// Matches in all places, and how many places have matches.
    counts: (usize, usize),
    /// Where the last search stopped, until the text changes.
    found: Option<Found>,
    /// Whether the user last worked in the dialog, as opposed to elsewhere in the window. F3 and
    /// Shift+F3 leave it as it is. While it is not, the dialog is partly transparent.
    active: bool,
    /// Where the dialog was last drawn, to tell where clicks land.
    rect: Option<egui::Rect>,
    /// How many places Mark All has marked.
    marked: usize,
}

impl SearchDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            text: String::new(),
            pattern: Pattern::Blank,
            history: None,
            focus: false,
            counts: (0, 0),
            found: None,
            active: false,
            rect: None,
            marked: 0,
        }
    }

    /// Notes how many places are marked, for the status bar.
    pub fn set_marked(&mut self, marked: usize) {
        self.marked = marked;
    }

    /// Opens the dialog, with the keyboard in its text field when `focus`. The first time, it
    /// starts with the latest search, so Find Next works right away. Returns whether the counts
    /// must be updated.
    pub fn open(&mut self, focus: bool) -> bool {
        let opening = !self.open;
        self.open = true;
        self.focus |= focus;
        self.active = focus || (self.active && !opening);
        if self.history.is_none() {
            let history = search::load_history();
            if let Some(latest) = history.first() {
                self.text = latest.clone();
                self.compile();
            }
            self.history = Some(history);
        }
        opening
    }

    /// The search's regular expression, when the text is a valid one.
    pub fn regex(&self) -> Option<&Regex> {
        match &self.pattern {
            Pattern::Valid(regex) => Some(regex),
            _ => None,
        }
    }

    /// Whether there is nothing to search for yet.
    pub fn is_blank(&self) -> bool {
        matches!(self.pattern, Pattern::Blank)
    }

    /// Takes the counts from `places`, which were searched for the current text.
    pub fn set_counts(&mut self, places: &[Place]) {
        let with_matches = places.iter().filter(|place| place.occurrences > 0);
        self.counts = (with_matches.clone().map(|place| place.occurrences).sum(), with_matches.count());
    }

    /// Notes where a search stopped.
    pub fn set_found(&mut self, found: Option<Found>) {
        self.found = found;
    }

    /// Adds the current text to the previous searches and saves them.
    pub fn remember(&mut self) -> anyhow::Result<()> {
        let history = self.history.get_or_insert_with(search::load_history);
        if history.first() == Some(&self.text) {
            return Ok(());
        }
        search::remember(history, &self.text);
        search::save_history(history)
    }

    fn compile(&mut self) {
        self.found = None;
        self.pattern = if self.text.trim().is_empty() {
            Pattern::Blank
        } else {
            match search::compile(&self.text) {
                Ok(regex) => Pattern::Valid(regex),
                Err(err) => Pattern::Invalid(err.to_string()),
            }
        };
    }

    /// Draws the dialog when it is open. Enter finds the next match and Shift+Enter the previous
    /// one; Escape closes it.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<SearchAction> {
        if !self.open {
            return None;
        }
        self.follow_clicks(ctx);
        let mut action = None;
        let mut open = true;
        let mut close = false;
        let mut frame = egui::Frame::window(&ctx.global_style());
        if !self.active {
            frame.fill = frame.fill.gamma_multiply(INACTIVE_FILL);
            // A shadow would darken what is under the dialog.
            frame.shadow = egui::Shadow::NONE;
        }
        let shown = egui::Window::new("Search")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .frame(frame)
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-440., 70.))
            .show(ctx, |ui| {
                if !self.active {
                    ui.multiply_opacity(INACTIVE_CONTENT);
                }
                ui.horizontal(|ui| {
                    ui.label("Find:");
                    let mut output = TextEdit::singleline(&mut self.text)
                        .id_salt("search text")
                        .hint_text("Regular expression")
                        .desired_width(300.)
                        .show(ui);
                    let id = output.response.response.id;
                    if output.response.response.gained_focus() {
                        self.active = true;
                    }
                    if std::mem::take(&mut self.focus) {
                        output.response.response.request_focus();
                        let all = CCursorRange::two(CCursor::new(0), CCursor::new(self.text.chars().count()));
                        output.state.cursor.set_char_range(Some(all));
                        output.state.store(ui.ctx(), id);
                    }
                    if output.response.response.changed() {
                        self.compile();
                        action = Some(SearchAction::Changed);
                    }
                    if output.response.response.lost_focus() {
                        let (enter, escape, shift) = ui.input(|input| {
                            (input.key_pressed(Key::Enter), input.key_pressed(Key::Escape), input.modifiers.shift)
                        });
                        if enter {
                            self.focus = true;
                            action = Some(if shift { SearchAction::FindPrevious } else { SearchAction::FindNext });
                        }
                        close = escape;
                    }
                    if let Some(picked) = self.history_button(ui) {
                        self.text = picked;
                        self.focus = true;
                        // Its list is outside the dialog, but this is work in the dialog.
                        self.active = true;
                        self.compile();
                        action = Some(SearchAction::Changed);
                    }
                });
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    let can_search = !self.is_blank() && self.regex().is_some();
                    if ui.add_enabled(can_search, egui::Button::new("<< Find Prev")).clicked() {
                        action = Some(SearchAction::FindPrevious);
                    }
                    if ui.add_enabled(can_search, egui::Button::new("Find Next >>")).clicked() {
                        action = Some(SearchAction::FindNext);
                    }
                    if ui.add_enabled(can_search, egui::Button::new("Mark All")).clicked() {
                        action = Some(SearchAction::MarkAll);
                    }
                });
                if let Pattern::Invalid(error) = &self.pattern {
                    ui.add_space(4.);
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
            });
        self.rect = shown.map(|shown| shown.response.rect);
        if !open || close {
            self.open = false;
            self.found = None;
            self.active = false;
        }
        action
    }

    /// A click in the dialog makes it active and a click elsewhere inactive.
    fn follow_clicks(&mut self, ctx: &egui::Context) {
        let pressed = ctx.input(|input| input.pointer.any_pressed().then(|| input.pointer.interact_pos()).flatten());
        if let (Some(pos), Some(rect)) = (pressed, self.rect) {
            self.active = rect.contains(pos);
        }
    }

    /// The button that drops down the list of previous searches; returns the one picked.
    fn history_button(&self, ui: &mut egui::Ui) -> Option<String> {
        let history = self.history.as_deref().unwrap_or_default();
        let mut picked = None;
        ui.add_enabled_ui(!history.is_empty(), |ui| {
            let response = ui
                .menu_button("     ", |ui| {
                    ScrollArea::vertical().max_height(300.).show(ui, |ui| {
                        for previous in history {
                            if ui.add(egui::Button::new(previous.as_str()).truncate()).clicked() {
                                picked = Some(previous.clone());
                                ui.close();
                            }
                        }
                    });
                })
                .response
                .on_hover_text("Previous searches");
            paint_down_arrow(ui, &response);
        });
        picked
    }

    /// The search's part of the status bar, while the dialog is open and has text: the number
    /// of occurrences, or a highlighted warning when there are none.
    pub fn status(&self, ui: &mut egui::Ui) {
        if !self.open {
            return;
        }
        let warning = |ui: &mut egui::Ui, text: String| {
            ui.label(RichText::new(format!(" {text} ")).small().strong().color(Color32::WHITE).background_color(WARNING_BG));
        };
        match &self.pattern {
            Pattern::Blank => {}
            Pattern::Invalid(_) => warning(ui, "Invalid search pattern".to_string()),
            Pattern::Valid(_) if self.counts.0 == 0 => warning(ui, format!("No matches for \"{}\"", self.text)),
            Pattern::Valid(_) => {
                let strong = ui.visuals().strong_text_color();
                ui.label(RichText::new(self.count_text()).small().strong().color(strong));
            }
        }
    }

    fn count_text(&self) -> String {
        let (occurrences, places) = self.counts;
        let mut text = format!(
            "\"{}\": {occurrences} {} in {places} {}",
            self.text,
            if occurrences == 1 { "occurrence" } else { "occurrences" },
            if places == 1 { "place" } else { "places" },
        );
        if let Some(found) = self.found {
            text.push_str(&format!("  -  at {} of {places}", found.rank));
            if found.wrapped {
                text.push_str(" (wrapped around)");
            }
        }
        if self.marked > 0 {
            text.push_str(&format!("  -  {} marked", self.marked));
        }
        text
    }
}

/// Draws a down arrow in the middle of a button. Painted instead of typed, since the fonts have
/// no glyph for it.
fn paint_down_arrow(ui: &egui::Ui, button: &egui::Response) {
    let center = button.rect.center();
    let (half, height) = (4.5, 5.);
    let color = ui.style().interact(button).fg_stroke.color;
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            center + egui::vec2(-half, -height / 2.),
            center + egui::vec2(half, -height / 2.),
            center + egui::vec2(0., height / 2.),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(text: &str, counts: (usize, usize), found: Option<Found>) -> SearchDialog {
        let mut dialog = SearchDialog::new();
        dialog.history = Some(Vec::new());
        dialog.text = text.to_string();
        dialog.compile();
        dialog.counts = counts;
        dialog.found = found;
        dialog
    }

    #[test]
    fn the_status_counts_occurrences_and_tells_of_wraps() {
        assert_eq!(with("a", (1, 1), None).count_text(), "\"a\": 1 occurrence in 1 place");
        let found = Found { place: 7, rank: 3, wrapped: false };
        assert_eq!(with("a", (12, 8), Some(found)).count_text(), "\"a\": 12 occurrences in 8 places  -  at 3 of 8");
        let found = Found { place: 0, rank: 1, wrapped: true };
        assert_eq!(
            with("a", (12, 8), Some(found)).count_text(),
            "\"a\": 12 occurrences in 8 places  -  at 1 of 8 (wrapped around)"
        );
    }

    #[test]
    fn the_status_counts_what_is_marked() {
        let mut dialog = with("a", (12, 8), None);
        dialog.set_marked(8);
        assert_eq!(dialog.count_text(), "\"a\": 12 occurrences in 8 places  -  8 marked");
    }

    #[test]
    fn the_text_decides_the_pattern() {
        assert!(with("  ", (0, 0), None).is_blank());
        assert!(with("(", (0, 0), None).regex().is_none());
        assert!(!with("(", (0, 0), None).is_blank());
        assert!(with("a.b", (0, 0), None).regex().is_some());
    }

    /// Draws `dialog` for one frame with a click at `pressed`, if any.
    fn frame(ctx: &egui::Context, dialog: &mut SearchDialog, pressed: Option<egui::Pos2>) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200., 800.));
        let mut events = Vec::new();
        if let Some(pos) = pressed {
            events.push(egui::Event::PointerMoved(pos));
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            dialog.show(ui.ctx());
        });
        output.textures_delta.clear();
        if let Some(pos) = pressed {
            let release = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            };
            let input = egui::RawInput { screen_rect: Some(screen), events: vec![release], ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                dialog.show(ui.ctx());
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn the_dialog_is_active_only_while_the_user_works_in_it() {
        let ctx = egui::Context::default();
        crate::ui::setup_fonts(&ctx);
        let mut dialog = with("a", (1, 1), None);
        let elsewhere = egui::pos2(100., 700.);

        // Ctrl+F.
        dialog.open(true);
        assert!(dialog.active);
        frame(&ctx, &mut dialog, None);
        frame(&ctx, &mut dialog, None);
        assert!(dialog.active);
        let inside = dialog.rect.unwrap().center();

        // A click elsewhere, and F3 and Shift+F3 keep it inactive.
        frame(&ctx, &mut dialog, Some(elsewhere));
        assert!(!dialog.active);
        dialog.open(false);
        frame(&ctx, &mut dialog, None);
        assert!(!dialog.active);

        // A click in it, and F3 keeps it active.
        frame(&ctx, &mut dialog, Some(inside));
        assert!(dialog.active);
        dialog.open(false);
        frame(&ctx, &mut dialog, None);
        assert!(dialog.active);

        // Ctrl+F brings it back from elsewhere.
        frame(&ctx, &mut dialog, Some(elsewhere));
        assert!(!dialog.active);
        dialog.open(true);
        assert!(dialog.active);
    }

    #[test]
    fn f3_opens_a_closed_dialog_inactive() {
        let mut dialog = with("a", (1, 1), None);
        dialog.open(true);
        dialog.open = false;
        dialog.open(false);
        assert!(!dialog.active);
    }

    #[test]
    fn changing_the_text_forgets_where_the_search_was() {
        let mut dialog = with("a", (1, 1), Some(Found { place: 0, rank: 1, wrapped: false }));
        dialog.text = "b".to_string();
        dialog.compile();
        assert_eq!(dialog.found, None);
    }
}
