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

/// The launcher's build manifest of the installed game, relative to the game folder. Its
/// JSON `Data.Version` is the game build, e.g. `4.10.200.58746`, and `Data.RequestedP4ChangeNum`
/// the changelist, e.g. `13165946`. The build's third part is a build counter, not the patch:
/// `4.10.200` is release `4.10.3`, and the release name is not in any game file.
pub const BUILD_MANIFEST: &str = r"LIVE\build_manifest.id";

/// The RSI Launcher's log, relative to `%APPDATA%`. Its install lines name the release, e.g.
/// `Delta update completed (SC LIVE 4.10.3-live.13165946)`.
pub const LAUNCHER_LOG: &str = r"rsilauncher\logs\log.log";

/// The installed game's version, as far as it can be known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameVersion {
    /// The game build from [`BUILD_MANIFEST`], e.g. `4.10.200.58746`.
    pub build: String,
    /// The changelist from [`BUILD_MANIFEST`], e.g. `13165946`.
    pub changelist: Option<String>,
    /// The release the launcher installed for [`Self::changelist`], e.g. `4.10.3`, from the
    /// [`LAUNCHER_LOG`].
    pub release: Option<String>,
}

/// The installed game's version in `dir`'s [`BUILD_MANIFEST`], with the release name from the
/// [`LAUNCHER_LOG`] when it lists the manifest's changelist.
pub fn game_version(dir: &Path) -> Option<GameVersion> {
    let text = std::fs::read_to_string(dir.join(BUILD_MANIFEST)).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let data = json.get("Data")?;
    let build = data.get("Version")?.as_str()?.to_string();
    let changelist = data
        .get("RequestedP4ChangeNum")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|changelist| !changelist.is_empty())
        .map(str::to_string);
    let release = changelist.as_deref().and_then(|changelist| {
        let log = PathBuf::from(std::env::var_os("APPDATA")?).join(LAUNCHER_LOG);
        let bytes = std::fs::read(log).ok()?;
        launcher_release(&String::from_utf8_lossy(&bytes), changelist)
    });
    Some(GameVersion { build, changelist, release })
}

/// The release, e.g. `4.10.3`, of the last LIVE install of `changelist` in the launcher's log
/// text: lines like `Installing Star Citizen LIVE 4.10.3-live.13165946 at ...` or
/// `(SC LIVE 4.10.3-live.13165946)`. The last one wins, as a changelist can be released again
/// under a new name.
pub fn launcher_release(log: &str, changelist: &str) -> Option<String> {
    const MARKERS: [&str; 2] = ["SC LIVE ", "Star Citizen LIVE "];
    let mut found = None;
    for line in log.lines() {
        for marker in MARKERS {
            let mut rest = line;
            while let Some(at) = rest.find(marker) {
                rest = &rest[at + marker.len()..];
                let token = rest.split(|c: char| c.is_whitespace() || c == ')').next().unwrap_or("");
                let Some((release, number)) = token.split_once("-live.") else { continue };
                let numeric = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
                if number == changelist && release.split('.').all(numeric) {
                    found = Some(release.to_string());
                }
            }
        }
    }
    found
}

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
    fn launcher_release_matches_changelist() {
        let log = r#"{ "t":"2026-06-03 20:06:35.000", "[main][info] ": "[Pipeline] Installing Star Citizen LIVE 4.8.0-live.11875683 at C:\\SC (type: update)"  },
{ "t":"2026-06-03 20:07:00.000", "[main][info] ": "[PatcherPhase] Delta update completed (SC LIVE 4.8.1-live.11875683) in C:\\SC\\LIVE"  },
{ "t":"2026-10-14 20:46:41.000", "[main][info] ": "[Pipeline] Installing Star Citizen LIVE 4.10.2-live.12913470 at C:\\SC (type: update)"  },
{ "t":"2026-11-12 19:43:38.756", "[main][info] ": "[PatcherPhase] Delta update completed (SC LIVE 4.10.3-live.13165946) in C:\\SC\\LIVE"  },"#;
        assert_eq!(launcher_release(log, "13165946").as_deref(), Some("4.10.3"));
        assert_eq!(launcher_release(log, "12913470").as_deref(), Some("4.10.2"));
        assert_eq!(launcher_release(log, "11875683").as_deref(), Some("4.8.1"));
        assert_eq!(launcher_release(log, "1316594"), None);
        assert_eq!(launcher_release("SC LIVE garbage-live.13165946", "13165946"), None);
        assert_eq!(launcher_release("", "13165946"), None);
    }

    #[test]
    fn game_version_reads_build_manifest() {
        let (root, dir) = temp_game_dir("manifest", "StarCitizen", None);
        assert_eq!(game_version(&dir), None);
        std::fs::write(
            dir.join(BUILD_MANIFEST),
            r#"{ "Data": { "Branch": "sc-alpha-4.10.0", "Version": "4.10.200.58746" } }"#,
        )
        .unwrap();
        let version = game_version(&dir).unwrap();
        assert_eq!(version.build, "4.10.200.58746");
        assert_eq!(version.changelist, None);
        assert_eq!(version.release, None);
        std::fs::write(
            dir.join(BUILD_MANIFEST),
            r#"{ "Data": { "Version": "4.10.200.58746", "RequestedP4ChangeNum": "13165946" } }"#,
        )
        .unwrap();
        assert_eq!(game_version(&dir).unwrap().changelist.as_deref(), Some("13165946"));
        std::fs::write(dir.join(BUILD_MANIFEST), "not json").unwrap();
        assert_eq!(game_version(&dir), None);
        std::fs::remove_dir_all(root).unwrap();
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
