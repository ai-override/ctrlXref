//! The dark theme, modelled on the table of the Star Citizen Ship Matrix
//! (<https://robertsspaceindustries.com/en/ship-matrix>), and the selection color of the
//! light theme. Colors noted as "site" are from that page's `#statsapp` stylesheet rules.

use eframe::egui::{self, Color32, Context, FontFamily, RichText, Stroke, Theme, Ui};

/// Font family for headings: Electrolize, as on the site. See `setup_fonts`.
pub const HEADING: &str = "heading";

/// Site: black at 90%, over a dark page.
const BACKGROUND: Color32 = Color32::from_rgb(0x02, 0x05, 0x09);
/// Popups and menus, a little lighter than the panels so they stand out. Not on the site.
const WINDOW: Color32 = Color32::from_rgb(0x07, 0x12, 0x1c);
/// Site: `.statbox p`.
const TEXT: Color32 = Color32::from_rgb(0x88, 0xa3, 0xad);
/// Site: `.statbox.role`.
const TEXT_WEAK: Color32 = Color32::from_rgb(0x54, 0x82, 0xa0);
/// Headers, strong text and the default profile's bindings. Site: `.statbox.title`.
pub const ACCENT: Color32 = Color32::from_rgb(0x6f, 0xd8, 0xff);
/// Site: `.statbox.title > p > a`.
const LINK: Color32 = Color32::from_rgb(0x00, 0xde, 0xff);
/// Hovered headings and warnings. Site: `.statbox.heading:hover`.
const AMBER: Color32 = Color32::from_rgb(0xff, 0xae, 0x00);
/// Site: `.statboxcontainer` border.
const LINE: Color32 = Color32::from_rgb(0x20, 0x4c, 0x7a);

/// The site's table blue, `rgb(29, 63, 98)`, at `alpha`.
const fn navy(alpha: u8) -> Color32 {
    premultiplied(0x1d, 0x3f, 0x62, alpha)
}

/// The site's highlight blue, `rgb(0, 132, 255)`, at `alpha`.
const fn highlight(alpha: u8) -> Color32 {
    premultiplied(0x00, 0x84, 0xff, alpha)
}

/// egui colors are premultiplied; this keeps the site's `rgba()` values readable.
const fn premultiplied(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    const fn scale(channel: u8, alpha: u8) -> u8 {
        (channel as u32 * alpha as u32 / 255) as u8
    }
    Color32::from_rgba_premultiplied(scale(r, a), scale(g, a), scale(b, a), a)
}

/// 30%: site `.statbox:nth-child(odd)`.
const STRIPE: Color32 = navy(0x4d);
/// 60%: site `.statbox` bottom border.
const ROW_DIVIDER: Color32 = navy(0x99);
/// 35%: site `#statsheadings .statboxcontainer`.
const HEADER_BG: Color32 = navy(0x59);
/// 40%: site `#statsheadings .sortable:hover`.
const HEADER_HOVER_BG: Color32 = highlight(0x66);
/// `rgba(3, 102, 15, .5)`: site `.statbox.active .highlighter`.
const HEADER_SORTED_BG: Color32 = premultiplied(0x03, 0x66, 0x0f, 0x80);
/// `rgba(0, 122, 255, .75)`: site `.ship.selected`.
const SELECTION: Color32 = premultiplied(0x00, 0x7a, 0xff, 0xbf);
/// A hovered row; the site has none, so the highlight blue kept fainter than the selection.
const HOVER: Color32 = highlight(0x2e);

/// Installs the dark theme and the light theme's selection color.
pub fn apply(ctx: &Context) {
    ctx.style_mut_of(Theme::Dark, |style| dark_visuals(&mut style.visuals));
    // Stands out more than egui's default: blue-500 at 30%.
    ctx.style_mut_of(Theme::Light, |style| {
        style.visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(0x3b, 0x82, 0xf6, 0x4d);
    });
}

fn dark_visuals(visuals: &mut egui::Visuals) {
    visuals.panel_fill = BACKGROUND;
    visuals.extreme_bg_color = Color32::BLACK;
    visuals.window_fill = WINDOW;
    visuals.window_stroke = Stroke::new(1., LINE);
    visuals.faint_bg_color = STRIPE;
    visuals.code_bg_color = WINDOW;
    visuals.weak_text_color = Some(TEXT_WEAK);
    visuals.hyperlink_color = LINK;
    visuals.warn_fg_color = AMBER;
    visuals.selection.bg_fill = SELECTION;
    visuals.selection.stroke = Stroke::new(1., Color32::WHITE);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = BACKGROUND;
    widgets.noninteractive.weak_bg_fill = BACKGROUND;
    widgets.noninteractive.bg_stroke = Stroke::new(1., LINE);
    widgets.noninteractive.fg_stroke = Stroke::new(1., TEXT);
    // Buttons and the like; `strong_text_color` is `active`'s text color.
    widgets.inactive.bg_fill = navy(0x80);
    widgets.inactive.weak_bg_fill = navy(0x80);
    widgets.inactive.fg_stroke = Stroke::new(1., TEXT);
    widgets.hovered.bg_fill = HOVER;
    widgets.hovered.weak_bg_fill = HOVER;
    widgets.hovered.bg_stroke = Stroke::new(1., LINE);
    widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    widgets.active.bg_fill = HEADER_HOVER_BG;
    widgets.active.weak_bg_fill = HEADER_HOVER_BG;
    widgets.active.bg_stroke = Stroke::new(1., LINE);
    widgets.active.fg_stroke = Stroke::new(1.5, ACCENT);
    widgets.open.bg_fill = HOVER;
    widgets.open.weak_bg_fill = HOVER;
    widgets.open.bg_stroke = Stroke::new(1., LINE);
    widgets.open.fg_stroke = Stroke::new(1., ACCENT);
}

/// `text` as a heading of the dark theme: uppercase Electrolize, in the strong text color.
pub fn heading_text(text: &str) -> RichText {
    exact_heading_text(&text.to_uppercase())
}

/// `text` in the dark theme's heading font and strong text color, its case kept.
pub fn exact_heading_text(text: &str) -> RichText {
    RichText::new(text).family(FontFamily::Name(HEADING.into())).strong()
}

/// The area of the cell `ui` draws, padding included.
fn cell_rect(ui: &Ui) -> egui::Rect {
    ui.max_rect().expand2(ui.spacing().item_spacing * 0.5)
}

/// Paints the background of a table header cell in the dark theme, and returns the text
/// color for its title: amber when the pointer is on the cell.
pub fn paint_header_cell(ui: &Ui, sorted: bool) -> Color32 {
    let rect = cell_rect(ui);
    let hovered = ui.rect_contains_pointer(rect);
    ui.painter().rect_filled(rect, 0., HEADER_BG);
    if sorted {
        ui.painter().rect_filled(rect, 0., HEADER_SORTED_BG);
    }
    if hovered {
        ui.painter().rect_filled(rect, 0., HEADER_HOVER_BG);
    }
    if hovered { AMBER } else { ACCENT }
}

/// Paints the line under a table cell in the dark theme.
pub fn paint_row_divider(ui: &Ui) {
    let rect = cell_rect(ui);
    // Half a pixel up, so the cell's clip rectangle does not cut the line.
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1., ROW_DIVIDER));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_styles_the_dark_theme_and_only_the_selection_of_the_light_theme() {
        let ctx = Context::default();
        apply(&ctx);

        let dark = ctx.style_of(Theme::Dark);
        assert_eq!(dark.visuals.panel_fill, BACKGROUND);
        assert_eq!(dark.visuals.faint_bg_color, STRIPE);
        assert_eq!(dark.visuals.text_color(), TEXT);
        assert_eq!(dark.visuals.strong_text_color(), ACCENT);
        assert_eq!(dark.visuals.selection.bg_fill, SELECTION);

        let light = ctx.style_of(Theme::Light);
        let defaults = egui::Visuals::light();
        assert_eq!(light.visuals.panel_fill, defaults.panel_fill);
        assert_eq!(light.visuals.faint_bg_color, defaults.faint_bg_color);
        assert_eq!(light.visuals.text_color(), defaults.text_color());
    }

    #[test]
    fn premultiplied_scales_the_channels_by_alpha() {
        assert_eq!(premultiplied(0xff, 0x80, 0x00, 0x80), Color32::from_rgba_premultiplied(0x80, 0x40, 0x00, 0x80));
    }
}
