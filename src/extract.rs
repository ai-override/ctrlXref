//! Extracting the default control profiles from the game's `Data.p4k` with the unp4k tools,
//! as a sequence of steps run in the game folder. Mirrors the `extract.bat` shipped next to
//! the tools.

use std::ffi::OsString;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};

/// Files unpacked from `Data.p4k`; the XML ones are serialized CryXML until unforge runs.
const PACKED_FILES: [&str; 4] = [
    "defaultProfile.xml",
    "layout_all_blank.xml",
    "keybinding_localization.xml",
    "global.ini",
];

/// How a serialized CryXML file starts; unforge rewrites these as plain XML.
const CRYXML_MAGIC: &[u8] = b"CryXml";

/// The tools' full output, written into the work directory for diagnosing failures.
pub const LOG_FILE: &str = "ctrlxref_extract.log";

/// One step of the extraction.
pub struct Step {
    /// What the step does, for progress and error messages.
    pub label: String,
    action: Action,
    /// What must be true afterwards; the tools can exit successfully without doing anything.
    expect: Option<Expect>,
}

enum Action {
    /// Runs `program` with `args`.
    Run { program: PathBuf, args: Vec<OsString> },
    /// Deletes a file, if it is there.
    Delete(PathBuf),
}

enum Expect {
    /// A file with this name exists under the `Data` folder.
    Extracted(&'static str),
    /// No extracted XML file is still serialized CryXML.
    Deserialized,
}

/// The extraction steps for `game_dir`, in the order they must run.
pub fn steps(game_dir: &Path) -> Vec<Step> {
    let p4k = game_dir.join("LIVE").join("Data.p4k");
    let unp4k = game_dir.join("unp4k.exe");
    let data = game_dir.join("Data");
    let mut steps: Vec<Step> = PACKED_FILES
        .iter()
        .map(|&file| Step {
            label: format!("Extract {file} from Data.p4k"),
            action: Action::Run {
                program: unp4k.clone(),
                args: vec![p4k.clone().into(), file.into()],
            },
            expect: Some(Expect::Extracted(file)),
        })
        .collect();
    // unp4k extracts the game database whatever the filter, and unforge would convert it too.
    steps.push(Step {
        label: r"Delete Data\Game2.dcb".into(),
        action: Action::Delete(data.join("Game2.dcb")),
        expect: None,
    });
    steps.push(Step {
        label: "De-serialize the extracted files".into(),
        action: Action::Run {
            program: game_dir.join("unforge.cli.exe"),
            args: vec![data.into()],
        },
        expect: Some(Expect::Deserialized),
    });
    steps
}

/// Runs `steps` in order in `work_dir`, stopping at the first that fails. The tools' output
/// goes to [`LOG_FILE`] in `work_dir`.
pub fn run_all(steps: &[Step], work_dir: &Path) -> Result<()> {
    let log_path = work_dir.join(LOG_FILE);
    let mut log = File::create(&log_path).ok();
    let result = steps.iter().try_for_each(|step| {
        if let Some(log) = &mut log {
            let _ = writeln!(log, "==== {}", step.label);
        }
        step.run(work_dir, log.as_mut()).with_context(|| step.label.clone())
    });
    match (result, log) {
        (Err(err), Some(_)) => Err(err.context(format!("see {} for the tools' output", log_path.display()))),
        (result, _) => result,
    }
}

impl Step {
    fn run(&self, work_dir: &Path, log: Option<&mut File>) -> Result<()> {
        let output = match &self.action {
            Action::Run { program, args } => run_program(program, args, work_dir, log)?,
            Action::Delete(path) => {
                match std::fs::remove_file(path) {
                    Err(err) if err.kind() != ErrorKind::NotFound => {
                        return Err(err).with_context(|| format!("cannot delete {}", path.display()));
                    }
                    _ => return Ok(()),
                }
            }
        };
        let data = work_dir.join("Data");
        match self.expect {
            Some(Expect::Extracted(file)) if find_file(&data, file).is_none() => {
                bail!("{file} was not extracted into {}\n{}", data.display(), last_lines(&output))
            }
            Some(Expect::Deserialized) => {
                for file in PACKED_FILES.iter().filter(|file| file.ends_with(".xml")) {
                    let path = find_file(&data, file)
                        .with_context(|| format!("{file} is missing from {}", data.display()))?;
                    if starts_with(&path, CRYXML_MAGIC)? {
                        bail!("{} is still serialized\n{}", path.display(), last_lines(&output));
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn run_program(program: &Path, args: &[OsString], work_dir: &Path, mut log: Option<&mut File>) -> Result<Output> {
    let mut command = Command::new(program);
    command.args(args).current_dir(work_dir).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // The tools are console programs; don't flash a console window over the app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    if let Some(log) = log.as_deref_mut() {
        let _ = writeln!(log, "> {command:?}");
    }
    let output = command.output();
    if let Some(log) = log {
        match &output {
            Ok(output) => {
                let _ = writeln!(log, "exit: {}", output.status);
                let _ = log.write_all(&output.stdout);
                let _ = log.write_all(&output.stderr);
            }
            Err(err) => {
                let _ = writeln!(log, "cannot start: {err}");
            }
        }
    }
    let output = output.with_context(|| format!("cannot start {}", program.display()))?;
    if !output.status.success() {
        bail!("{} exited with {}\n{}", program.display(), output.status, last_lines(&output));
    }
    Ok(output)
}

/// The first file named `name` (in any case) under `dir`, searching subfolders.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if entry.file_name().eq_ignore_ascii_case(name) {
            return Some(path);
        }
    }
    None
}

fn starts_with(path: &Path, prefix: &[u8]) -> Result<bool> {
    let mut head = vec![0; prefix.len()];
    let mut file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let read = file.read(&mut head)?;
    Ok(read == prefix.len() && head == prefix)
}

/// The end of the command's error output, or of its normal output when that is empty.
fn last_lines(output: &Output) -> String {
    let text = match output.stderr.iter().any(|b| !b.is_ascii_whitespace()) {
        true => &output.stderr,
        false => &output.stdout,
    };
    let text = String::from_utf8_lossy(text);
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(5)..].join("\n")
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn cmd(label: &str, script: &str, expect: Option<Expect>) -> Step {
        Step {
            label: label.into(),
            action: Action::Run {
                program: "cmd".into(),
                args: vec!["/c".into(), script.into()],
            },
            expect,
        }
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ctrlxref-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn steps_follow_extract_bat() {
        let game = Path::new(r"D:\Games\StarCitizen");
        let p4k = OsString::from(r"D:\Games\StarCitizen\LIVE\Data.p4k");
        let steps = steps(game);
        assert_eq!(steps.len(), 6);
        for (step, file) in steps.iter().zip(PACKED_FILES) {
            let Action::Run { program, args } = &step.action else {
                panic!("{} is not a command", step.label);
            };
            assert_eq!(program, &game.join("unp4k.exe"));
            assert_eq!(args, &[p4k.clone(), file.into()]);
        }
        assert!(matches!(
            &steps[4].action,
            Action::Delete(path) if path == Path::new(r"D:\Games\StarCitizen\Data\Game2.dcb")
        ));
        let Action::Run { program, args } = &steps[5].action else {
            panic!("last step is not a command");
        };
        assert_eq!(program, &game.join("unforge.cli.exe"));
        assert_eq!(args, &[OsString::from(r"D:\Games\StarCitizen\Data")]);
    }

    #[test]
    fn delete_ignores_missing_file() {
        let dir = temp_dir("delete");
        let file = dir.join("Game2.dcb");
        let step = Step {
            label: "delete".into(),
            action: Action::Delete(file.clone()),
            expect: None,
        };
        step.run(&dir, None).unwrap();
        std::fs::write(&file, "x").unwrap();
        step.run(&dir, None).unwrap();
        assert!(!file.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stops_at_failing_step_and_logs_output() {
        let dir = temp_dir("steps");
        let steps = [
            cmd("first", "echo first> first.txt", None),
            cmd("second", "echo broken 1>&2 & exit 3", None),
            cmd("third", "echo third> third.txt", None),
        ];
        let message = format!("{:#}", run_all(&steps, &dir).unwrap_err());
        assert!(message.contains("second: "), "{message}");
        assert!(message.contains("broken"), "{message}");
        assert!(message.contains(LOG_FILE), "{message}");
        assert!(dir.join("first.txt").is_file());
        assert!(!dir.join("third.txt").exists());
        let log = std::fs::read_to_string(dir.join(LOG_FILE)).unwrap();
        assert!(log.contains("==== second") && log.contains("broken"), "{log}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn successful_exit_without_extracted_file_fails() {
        let dir = temp_dir("not-extracted");
        let step = cmd("extract", "echo 0 files matched", Some(Expect::Extracted("defaultProfile.xml")));
        let message = format!("{:#}", step.run(&dir, None).unwrap_err());
        assert!(message.contains("defaultProfile.xml was not extracted"), "{message}");
        assert!(message.contains("0 files matched"), "{message}");

        let config = dir.join("Data").join("Libs").join("Config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("defaultProfile.xml"), "x").unwrap();
        step.run(&dir, None).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn serialized_xml_after_unforge_fails() {
        let dir = temp_dir("serialized");
        let config = dir.join("Data").join("Libs").join("Config");
        std::fs::create_dir_all(&config).unwrap();
        for file in PACKED_FILES.iter().filter(|file| file.ends_with(".xml")) {
            std::fs::write(config.join(file), "<ActionMaps/>").unwrap();
        }
        let step = cmd("unforge", "rem", Some(Expect::Deserialized));
        step.run(&dir, None).unwrap();
        std::fs::write(config.join("defaultProfile.xml"), b"CryXmlB\0...").unwrap();
        let message = format!("{:#}", step.run(&dir, None).unwrap_err());
        assert!(message.contains("still serialized"), "{message}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
