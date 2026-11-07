//! Application commands, key bindings and the File / Source / Tools / Help menu structure.

use eframe::egui::{self, Button, Context, Key, KeyboardShortcut, Modifiers, Ui};

use super::help::HelpTopic;

/// Everything the menus, the key bindings and the theme button can ask the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    OpenFile,
    OpenCurrentRebindFile,
    OpenCustomRebindFile,
    SaveRebindFile,
    SaveRebindCopyAs,
    CreateBlankRebindFile,
    CloseFile,
    Quit,
    Theme(ThemeChoice),
    ToggleTheme,
    LocateGameFolder,
    ExtractDefaults,
    ShowUnusedCombos,
    AutofillKeyboard,
    ExportCsv,
    ExportJson,
    Find,
    FindNext,
    FindPrevious,
    Help(HelpTopic),
    About,
}

/// The theme the user picked; `System` follows the OS light/dark appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeChoice {
    Light,
    Dark,
    System,
}

const CTRL_SHIFT: Modifiers = Modifiers::CTRL.plus(Modifiers::SHIFT);

/// Key bindings. The Shift variants come first: a shortcut without Shift also matches
/// with Shift held.
const SHORTCUTS: [(Modifiers, Key, Command); 10] = [
    (CTRL_SHIFT, Key::O, Command::OpenCustomRebindFile),
    (CTRL_SHIFT, Key::S, Command::SaveRebindCopyAs),
    (Modifiers::SHIFT, Key::F3, Command::FindPrevious),
    (Modifiers::NONE, Key::F3, Command::FindNext),
    (Modifiers::CTRL, Key::F, Command::Find),
    (Modifiers::CTRL, Key::O, Command::OpenFile),
    (Modifiers::CTRL, Key::R, Command::OpenCurrentRebindFile),
    (Modifiers::CTRL, Key::S, Command::SaveRebindFile),
    (Modifiers::CTRL, Key::W, Command::CloseFile),
    (Modifiers::CTRL, Key::Q, Command::Quit),
];

/// The command of a key binding pressed this frame, if any; the key press is consumed.
pub fn shortcut(ctx: &Context) -> Option<Command> {
    ctx.input_mut(|input| {
        SHORTCUTS
            .iter()
            .find(|(modifiers, key, _)| input.consume_shortcut(&KeyboardShortcut::new(*modifiers, *key)))
            .map(|(_, _, command)| *command)
    })
}

/// Shows the menus in a menu bar, with a check mark on the active theme and Extract
/// Defaults only when `can_extract`. Returns the picked command.
pub fn menu_items(ui: &mut Ui, theme: ThemeChoice, can_extract: bool) -> Option<Command> {
    let mut picked = None;
    let mut item = |ui: &mut Ui, label: &str, command: Command| {
        let mut button = Button::new(label);
        if let Some((modifiers, key, _)) = SHORTCUTS.iter().find(|(_, _, c)| *c == command) {
            button = button.shortcut_text(ui.ctx().format_shortcut(&KeyboardShortcut::new(*modifiers, *key)));
        }
        if ui.add(button).clicked() {
            picked = Some(command);
        }
    };
    ui.menu_button("File", |ui| {
        item(ui, "Open defaultProfile", Command::OpenFile);
        item(ui, "Open Current Rebind File", Command::OpenCurrentRebindFile);
        item(ui, "Open Custom Rebind File...", Command::OpenCustomRebindFile);
        item(ui, "Save Rebind File", Command::SaveRebindFile);
        item(ui, "Save Rebind Copy As...", Command::SaveRebindCopyAs);
        item(ui, "Create Blank Rebind File...", Command::CreateBlankRebindFile);
        item(ui, "Close", Command::CloseFile);
        ui.separator();
        ui.menu_button("Theme", |ui| {
            for (label, choice) in [
                ("Light", ThemeChoice::Light),
                ("Dark", ThemeChoice::Dark),
                ("System", ThemeChoice::System),
            ] {
                let label = if theme == choice { format!("✔ {label}") } else { format!("     {label}") };
                item(ui, &label, Command::Theme(choice));
            }
        });
        ui.separator();
        item(ui, "Exit", Command::Quit);
    });
    ui.menu_button("Source", |ui| {
        item(ui, "Locate Star Citizen Game Folder...", Command::LocateGameFolder);
        if can_extract {
            item(ui, "Extract Defaults from Data.p4k Package", Command::ExtractDefaults);
        }
    });
    ui.menu_button("Tools", |ui| {
        item(ui, "Unused Key Combinations...", Command::ShowUnusedCombos);
        item(ui, "Autofill All Unbound Keyboard Controls", Command::AutofillKeyboard);
        ui.separator();
        item(ui, "Export to .csv...", Command::ExportCsv);
        item(ui, "Export to .json...", Command::ExportJson);
    });
    ui.menu_button("Help", |ui| {
        item(ui, "File Menu", Command::Help(HelpTopic::File));
        item(ui, "Source Menu", Command::Help(HelpTopic::Source));
        item(ui, "Tools Menu", Command::Help(HelpTopic::Tools));
        ui.separator();
        item(ui, "About ctrlXref", Command::About);
    });
    picked
}

/// The egui theme preference for `choice`.
pub fn preference(choice: ThemeChoice) -> egui::ThemePreference {
    match choice {
        ThemeChoice::Light => egui::ThemePreference::Light,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
        ThemeChoice::System => egui::ThemePreference::System,
    }
}
