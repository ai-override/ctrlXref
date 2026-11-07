//! The Search dialog and the search's part of the status bar.

use ctrlxref::search::{self, Found, Place};
use eframe::egui::text::CCursor;
use eframe::egui::text_selection::CCursorRange;
use eframe::egui::{self, Color32, Key, RichText, ScrollArea, TextEdit};
use regex::Regex;

/// Background of the status bar's warning, like a warning notification's.
const WARNING_BG: Color32 = Color32::from_rgb(0x9a, 0x58, 0x00);

/// What the user did in the dialog, for the owner to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchAction {
    /// The search text changed, so the counts must be updated.
    Changed,
    FindNext,
    FindPrevious,
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
        }
    }

    /// Opens the dialog. The first time, it starts with the latest search, so Find Next works
    /// right away. Returns whether the counts must be updated.
    pub fn open(&mut self, focus: bool) -> bool {
        let opening = !self.open;
        self.open = true;
        self.focus |= focus;
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
        let mut action = None;
        let mut open = true;
        let mut close = false;
        egui::Window::new("Search")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-440., 70.))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Find:");
                    let mut output = TextEdit::singleline(&mut self.text)
                        .id_salt("search text")
                        .hint_text("Regular expression")
                        .desired_width(300.)
                        .show(ui);
                    let id = output.response.response.id;
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
                        self.compile();
                        action = Some(SearchAction::Changed);
                    }
                });
                ui.add_space(4.);
                ui.horizontal(|ui| {
                    let can_search = !self.is_blank() && self.regex().is_some();
                    if ui.add_enabled(can_search, egui::Button::new("Find Next")).clicked() {
                        action = Some(SearchAction::FindNext);
                    }
                    if ui.add_enabled(can_search, egui::Button::new("Find Previous")).clicked() {
                        action = Some(SearchAction::FindPrevious);
                    }
                });
                if let Pattern::Invalid(error) = &self.pattern {
                    ui.add_space(4.);
                    ui.colored_label(ui.visuals().warn_fg_color, error);
                }
            });
        if !open || close {
            self.open = false;
            self.found = None;
        }
        action
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
    fn the_text_decides_the_pattern() {
        assert!(with("  ", (0, 0), None).is_blank());
        assert!(with("(", (0, 0), None).regex().is_none());
        assert!(!with("(", (0, 0), None).is_blank());
        assert!(with("a.b", (0, 0), None).regex().is_some());
    }

    #[test]
    fn changing_the_text_forgets_where_the_search_was() {
        let mut dialog = with("a", (1, 1), Some(Found { place: 0, rank: 1, wrapped: false }));
        dialog.text = "b".to_string();
        dialog.compile();
        assert_eq!(dialog.found, None);
    }
}
