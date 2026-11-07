//! The GUI, drawn with egui through OpenGL (glow); no DirectX.

mod app;
mod attribute_table;
mod help;
mod menus;
mod search;
mod theme;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, IconData, ViewportBuilder};

use app::CtrlXrefApp;

/// Font family for bold text: Segoe UI Bold when Windows has it, else the default font.
pub const BOLD: &str = "bold";

/// Starts the GUI, optionally opening `initial_file` and then `initial_rebind_file` right away.
pub fn run(initial_file: Option<PathBuf>, initial_rebind_file: Option<PathBuf>) {
    let icon = window_icon();
    let mut viewport = ViewportBuilder::default()
        .with_title("ctrlXref")
        .with_inner_size([1200., 800.])
        .with_min_inner_size([640., 400.]);
    if let Some(icon) = &icon {
        viewport = viewport.with_icon(icon.clone());
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };
    let result = eframe::run_native(
        "ctrlXref",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(CtrlXrefApp::new(initial_file, initial_rebind_file, icon, &cc.egui_ctx)))
        }),
    );
    if let Err(err) = result {
        eprintln!("ctrlXref: {err}");
    }
}

/// The application icon, from the same `.ico` that build.rs embeds into the executable.
fn window_icon() -> Option<Arc<IconData>> {
    let image =
        image::load_from_memory_with_format(include_bytes!("../../assets/icon/ctrlxref.ico"), image::ImageFormat::Ico)
            .ok()?
            .into_rgba8();
    let (width, height) = image.dimensions();
    Some(Arc::new(IconData { rgba: image.into_raw(), width, height }))
}

/// Uses Segoe UI, the Windows UI font, when it is installed, with a bold face for the
/// [`BOLD`] family; egui's default fonts stay as fallbacks (symbols, emoji) and are used
/// alone elsewhere. The [`theme::HEADING`] family is the bundled Electrolize, with those
/// fonts for the characters it lacks.
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let load = |name: &str| std::fs::read(PathBuf::from(r"C:\Windows\Fonts").join(name)).ok();
    let mut bold = fallback.clone();
    if let (Some(regular), Some(bold_data)) = (load("segoeui.ttf"), load("segoeuib.ttf")) {
        fonts.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(regular)));
        fonts.font_data.insert("segoe-bold".into(), Arc::new(FontData::from_owned(bold_data)));
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "segoe".into());
        }
        bold.insert(0, "segoe-bold".into());
    }
    fonts.families.insert(FontFamily::Name(BOLD.into()), bold);
    fonts.font_data.insert(
        "electrolize".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Electrolize-Regular.ttf"))),
    );
    let mut heading = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    heading.insert(0, "electrolize".into());
    fonts.families.insert(FontFamily::Name(theme::HEADING.into()), heading);
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The symbols typed in the code: the warning sign, the theme button's sun and moon, and the
    /// check mark of the theme menu.
    const SYMBOLS: [char; 4] = ['⚠', '☀', '🌙', '✔'];

    #[test]
    fn every_font_family_can_draw_the_symbols() {
        let ctx = egui::Context::default();
        setup_fonts(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();

        let families =
            [FontFamily::Proportional, FontFamily::Name(BOLD.into()), FontFamily::Name(theme::HEADING.into())];
        for family in families {
            let font = egui::FontId::new(14., family.clone());
            for symbol in SYMBOLS {
                assert!(ctx.fonts_mut(|fonts| fonts.has_glyph(&font, symbol)), "{family:?} has no {symbol}");
            }
        }
    }
}
