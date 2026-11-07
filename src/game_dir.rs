//! Finding the Star Citizen installation folder and remembering it between runs.

use std::path::{Path, PathBuf};

use anyhow::Result;

/// Where the RSI launcher installs the game by default.
pub const DEFAULT: &str = r"C:\Program Files\Roberts Space Industries\StarCitizen\";

/// A real `Data.p4k` holds the whole game, tens of gigabytes; anything smaller is a stub
/// or a partial download.
const MIN_P4K_SIZE: u64 = 1024 * 1024 * 1024;

/// The name the RSI launcher gives the installation folder.
const FOLDER_NAME: &str = "StarCitizen";

/// Whether `dir` is a game folder: it is named [`FOLDER_NAME`] (in any case) and has
/// `LIVE\Data.p4k` of at least [`MIN_P4K_SIZE`].
pub fn is_game_dir(dir: &Path) -> bool {
    has_folder_name(dir) && p4k_size(dir).is_some_and(|size| size >= MIN_P4K_SIZE)
}

fn has_folder_name(dir: &Path) -> bool {
    dir.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case(FOLDER_NAME))
}

/// The size of `dir`'s `LIVE\Data.p4k`, if it has that file.
fn p4k_size(dir: &Path) -> Option<u64> {
    std::fs::metadata(dir.join("LIVE").join("Data.p4k"))
        .ok()
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
}

/// One line per [`is_game_dir`] requirement, starting `OK:` or `Missing:` for `dir`.
pub fn folder_checks(dir: &Path) -> Vec<String> {
    let name = if has_folder_name(dir) {
        format!("OK: folder named {FOLDER_NAME}")
    } else {
        let actual = dir.file_name().unwrap_or_default().to_string_lossy();
        format!("Missing: folder named {FOLDER_NAME} (the selected folder is named \"{actual}\")")
    };
    const P4K: &str = r"LIVE\Data.p4k of at least 1 GB";
    let p4k = match p4k_size(dir) {
        Some(size) if size >= MIN_P4K_SIZE => format!("OK: {P4K}"),
        Some(size) => format!(
            "Missing: {P4K} (found only {} MB, a stub or a partial download)",
            size / (1024 * 1024)
        ),
        None => format!("Missing: {P4K}"),
    };
    vec![name, p4k]
}

/// One line per [`EXTRACTION_TOOLS`] file, starting `OK:` or `Missing:` for `dir`.
pub fn tool_checks(dir: &Path) -> Vec<String> {
    let missing = missing_extraction_tools(dir);
    EXTRACTION_TOOLS
        .into_iter()
        .map(|tool| {
            let status = if missing.contains(&tool) { "Missing" } else { "OK" };
            format!("{status}: {tool}")
        })
        .collect()
}

/// Files of the tools that unpack `Data.p4k` and convert its binary XML, relative to the game
/// folder. unp4k cannot decompress the package without its zstd library.
pub const EXTRACTION_TOOLS: [&str; 3] = ["unp4k.exe", "unforge.cli.exe", r"x64\libzstd.dll"];

/// The [`EXTRACTION_TOOLS`] files that `dir` lacks.
pub fn missing_extraction_tools(dir: &Path) -> Vec<&'static str> {
    EXTRACTION_TOOLS
        .into_iter()
        .filter(|tool| !dir.join(tool).is_file())
        .collect()
}

/// Whether `dir` has all [`EXTRACTION_TOOLS`].
pub fn has_extraction_tools(dir: &Path) -> bool {
    missing_extraction_tools(dir).is_empty()
}

/// The control profile opened at startup when none is given, relative to the game folder,
/// where Extract Defaults unpacks it.
pub const DEFAULT_PROFILE: &str = r"Data\Libs\Config\defaultProfile.xml";

/// The game's names for its controls, relative to the game folder, where Extract Defaults
/// unpacks them next to the default profile.
pub const CONTROL_NAMES: &str = r"Data\Libs\Config\keybinding_localization.xml";

/// The English localization texts the profiles' `@key` labels refer to, relative to the
/// game folder, where Extract Defaults unpacks them.
pub const ENGLISH_STRINGS: &str = r"Data\Localization\english\global.ini";

/// The game's blank control layout, relative to the game folder, where Extract Defaults
/// unpacks it. It clears every binding it lists, but it lists only part of the actions.
pub const BLANK_LAYOUT: &str = r"Data\Libs\Config\Mappings\layout_all_blank.xml";

/// The rebind file the game last used, relative to the game folder.
pub const CURRENT_REBIND: &str = r"LIVE\user\client\0\Profiles\default\actionmaps.xml";

/// Where the game saves the user's exported rebind mappings, relative to the game folder.
pub const MAPPINGS_DIR: &str = r"LIVE\user\client\0\controls\mappings";

/// Whether `Data.p4k` has been extracted into `dir`, i.e. `dir` has a `Data` folder.
pub fn is_extracted(dir: &Path) -> bool {
    dir.join("Data").is_dir()
}

/// The profile file for the startup argument `arg`, relative to the game folder `dir`, or
/// [`DEFAULT_PROFILE`] when there is none. An absolute `arg` is used as it is.
pub fn profile_path(dir: &Path, arg: Option<&Path>) -> PathBuf {
    dir.join(arg.unwrap_or(Path::new(DEFAULT_PROFILE)))
}

#[cfg(windows)]
const REG_KEY: &str = r"Software\ctrlXref";
#[cfg(windows)]
const REG_VALUE: &str = "GameDir";

/// The game folder saved by [`save`], if any.
#[cfg(windows)]
pub fn load_saved() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(REG_KEY).ok()?;
    let dir: String = key.get_value(REG_VALUE).ok()?;
    Some(PathBuf::from(dir))
}

/// Saves `dir` under `HKEY_CURRENT_USER\Software\ctrlXref` for [`load_saved`].
#[cfg(windows)]
pub fn save(dir: &Path) -> Result<()> {
    use anyhow::Context;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(REG_KEY)
        .with_context(|| format!(r"cannot open registry key HKCU\{REG_KEY}"))?;
    key.set_value(REG_VALUE, &dir.as_os_str())
        .with_context(|| format!(r"cannot write HKCU\{REG_KEY}\{REG_VALUE}"))
}

#[cfg(not(windows))]
pub fn load_saved() -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
pub fn save(_dir: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Makes `<temp>/ctrlxref-<test>-<pid>/<folder_name>` with an optional `LIVE\Data.p4k` of
    /// `p4k_size` bytes. Returns the test's own root, to remove afterwards, and the game folder.
    fn temp_game_dir(test: &str, folder_name: &str, p4k_size: Option<u64>) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("ctrlxref-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(folder_name);
        std::fs::create_dir_all(dir.join("LIVE")).unwrap();
        if let Some(size) = p4k_size {
            let file = std::fs::File::create(dir.join("LIVE").join("Data.p4k")).unwrap();
            file.set_len(size).unwrap();
        }
        (root, dir)
    }

    #[test]
    fn needs_live_data_p4k() {
        let (root, dir) = temp_game_dir("no-p4k", "StarCitizen", None);
        assert!(!is_game_dir(&dir));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_small_data_p4k() {
        let (root, dir) = temp_game_dir("small-p4k", "StarCitizen", Some(MIN_P4K_SIZE - 1));
        assert!(!is_game_dir(&dir));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_large_data_p4k() {
        let (root, dir) = temp_game_dir("large-p4k", "StarCitizen", Some(MIN_P4K_SIZE));
        assert!(is_game_dir(&dir));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn name_is_case_insensitive() {
        let (root, dir) = temp_game_dir("lowercase", "starcitizen", Some(MIN_P4K_SIZE));
        assert!(is_game_dir(&dir));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_other_folder_name() {
        let (root, dir) = temp_game_dir("other-name", "Games", Some(MIN_P4K_SIZE));
        assert!(!is_game_dir(&dir));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_checks_report_each_requirement() {
        let (root, dir) = temp_game_dir("checks-ok", "StarCitizen", Some(MIN_P4K_SIZE));
        assert_eq!(
            folder_checks(&dir),
            ["OK: folder named StarCitizen", r"OK: LIVE\Data.p4k of at least 1 GB"]
        );
        std::fs::remove_dir_all(root).unwrap();

        let (root, dir) = temp_game_dir("checks-name", "Games", None);
        assert_eq!(
            folder_checks(&dir),
            [
                "Missing: folder named StarCitizen (the selected folder is named \"Games\")",
                r"Missing: LIVE\Data.p4k of at least 1 GB"
            ]
        );
        std::fs::remove_dir_all(root).unwrap();

        let (root, dir) = temp_game_dir("checks-small", "StarCitizen", Some(5 * 1024 * 1024));
        assert_eq!(
            folder_checks(&dir)[1],
            r"Missing: LIVE\Data.p4k of at least 1 GB (found only 5 MB, a stub or a partial download)"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tool_checks_report_each_file() {
        let (root, dir) = temp_game_dir("tool-checks", "StarCitizen", None);
        std::fs::File::create(dir.join("unp4k.exe")).unwrap();
        assert_eq!(
            tool_checks(&dir),
            ["OK: unp4k.exe", "Missing: unforge.cli.exe", r"Missing: x64\libzstd.dll"]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn profile_path_is_relative_to_game_dir() {
        let game = Path::new(r"D:\Games\StarCitizen");
        assert_eq!(
            profile_path(game, None),
            Path::new(r"D:\Games\StarCitizen\Data\Libs\Config\defaultProfile.xml")
        );
        assert_eq!(
            profile_path(game, Some(Path::new(r"LIVE\user\client\0\Profiles\default\actionmaps.xml"))),
            Path::new(r"D:\Games\StarCitizen\LIVE\user\client\0\Profiles\default\actionmaps.xml")
        );
    }

    #[cfg(windows)]
    #[test]
    fn profile_path_keeps_absolute_arg() {
        let arg = Path::new(r"E:\xml\defaultProfile.xml");
        assert_eq!(profile_path(Path::new(r"D:\Games\StarCitizen"), Some(arg)), arg);
    }

    #[test]
    fn extracted_needs_data_folder() {
        let (root, dir) = temp_game_dir("extracted", "StarCitizen", None);
        assert!(!is_extracted(&dir));
        std::fs::create_dir(dir.join("Data")).unwrap();
        assert!(is_extracted(&dir));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extraction_needs_all_tool_files() {
        let (root, dir) = temp_game_dir("tools", "StarCitizen", None);
        assert_eq!(missing_extraction_tools(&dir), EXTRACTION_TOOLS);
        std::fs::File::create(dir.join("unp4k.exe")).unwrap();
        std::fs::File::create(dir.join("unforge.cli.exe")).unwrap();
        assert_eq!(missing_extraction_tools(&dir), [r"x64\libzstd.dll"]);
        assert!(!has_extraction_tools(&dir));
        std::fs::create_dir(dir.join("x64")).unwrap();
        std::fs::File::create(dir.join("x64").join("libzstd.dll")).unwrap();
        assert!(has_extraction_tools(&dir));
        std::fs::remove_dir_all(root).unwrap();
    }
}
