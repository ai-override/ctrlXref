//! The Unmatched Rebinds repair log: a text file next to the rebind file, named like it with
//! the extension `.err`. The box appends to it as it opens, listing the unmatched actions and
//! their bindings, and then a line for each change, so the log tells what was repaired and how.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The extension of the repair log, in place of the rebind file's `.xml`.
pub const EXTENSION: &str = "err";

/// The repair log of the rebind file at `rebind`: the same name with [`EXTENSION`] in place of
/// `.xml`, or added when the name has no `.xml`, so `layout.4.0.new` does not lose its `.new`.
pub fn path_for(rebind: &Path) -> PathBuf {
    if rebind.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("xml")) {
        rebind.with_extension(EXTENSION)
    } else {
        let mut name = rebind.as_os_str().to_owned();
        name.push(".");
        name.push(EXTENSION);
        PathBuf::from(name)
    }
}

/// Adds `text` to the end of the log at `path`, creating it when missing.
pub fn append(path: &Path, text: &str) -> io::Result<()> {
    OpenOptions::new().create(true).append(true).open(path)?.write_all(text.as_bytes())
}

/// `time` in UTC as `YYYY-MM-DD hh:mm:ss UTC`.
pub fn timestamp(time: SystemTime) -> String {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm), as in build.rs.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rest / 3600, rest / 60 % 60, rest % 60)
}

/// `text` as one log line, stamped with `time`.
pub fn line(time: SystemTime, text: &str) -> String {
    format!("[{}] {text}\n", timestamp(time))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn log_is_named_after_the_rebind_file() {
        assert_eq!(
            path_for(Path::new(r"C:\maps\GameGlass Keyboard 4.0.new.xml")),
            PathBuf::from(r"C:\maps\GameGlass Keyboard 4.0.new.err")
        );
        assert_eq!(path_for(Path::new("actionmaps.XML")), PathBuf::from("actionmaps.err"));
        assert_eq!(
            path_for(Path::new(r"C:\maps\GameGlass Keyboard 4.0.new")),
            PathBuf::from(r"C:\maps\GameGlass Keyboard 4.0.new.err")
        );
    }

    #[test]
    fn timestamps_are_utc() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01 00:00:00 UTC");
        // 2026-11-14 13:45:02 UTC
        let time = UNIX_EPOCH + Duration::from_secs(1_794_663_902);
        assert_eq!(timestamp(time), "2026-11-14 13:45:02 UTC");
        assert_eq!(line(time, "Moved"), "[2026-11-14 13:45:02 UTC] Moved\n");
    }

    #[test]
    fn append_adds_to_the_end() {
        let dir = std::env::temp_dir().join(format!("ctrlxref-repair-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.err");
        append(&path, "one\n").unwrap();
        append(&path, "two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        std::fs::remove_dir_all(dir).ok();
    }
}
