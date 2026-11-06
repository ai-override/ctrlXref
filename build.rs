//! Build steps:
//!
//! - **Version.** `version.txt` holds the application version and the day it was set, as
//!   `major.minor.patch YYYY-MM-DD`. On the first build of a new day, the patch number goes
//!   up by one and the file is rewritten, so each day of work gets a version of its own. The
//!   build script reruns only when the sources change, so days without changes don't count.
//!   The version reaches the code as `CTRLXREF_VERSION`. Edit the file by hand to start a new
//!   major or minor version, e.g. `0.6.0 2026-12-01`.
//! - **Icon.** Embeds the application icon into the Windows executable as resource 1, so it
//!   shows in Explorer; the windows load the same `.ico` for their title bar and taskbar. `assets/icon/ctrlxref.ico` is rendered from `ctrlxref.svg` (48 px and up) and
//!   `ctrlxref-small.svg` (16-32 px) in the same folder.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

const VERSION_FILE: &str = "version.txt";

fn main() {
    for path in ["src", "docs", "assets", "Cargo.toml", VERSION_FILE] {
        println!("cargo:rerun-if-changed={path}");
    }

    let version = daily_version();
    let text = format!("{}.{}.{}", version[0], version[1], version[2]);
    println!("cargo:rustc-env=CTRLXREF_VERSION={text}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let packed = (version[0] << 48) | (version[1] << 32) | (version[2] << 16);
        let mut res = winresource::WindowsResource::new();
        res.set_icon_with_id("assets/icon/ctrlxref.ico", "1")
            .set("FileVersion", &text)
            .set("ProductVersion", &text)
            .set_version_info(winresource::VersionInfo::FILEVERSION, packed)
            .set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);
        res.compile().expect("failed to embed the application icon");
    }
}

/// Reads `version.txt`, moving to the next patch number when its day is not today.
fn daily_version() -> [u64; 3] {
    let text = fs::read_to_string(VERSION_FILE).expect("cannot read version.txt");
    let mut fields = text.split_whitespace();
    let numbers: Vec<u64> = fields
        .next()
        .expect("version.txt is empty")
        .split('.')
        .map(|part| part.parse().expect("version.txt: version must be major.minor.patch"))
        .collect();
    let [major, minor, mut patch] = numbers[..] else {
        panic!("version.txt: version must be major.minor.patch");
    };
    let day = fields.next().unwrap_or("");
    let today = today();
    if day != today {
        if !day.is_empty() {
            patch += 1;
        }
        fs::write(VERSION_FILE, format!("{major}.{minor}.{patch} {today}\n"))
            .expect("cannot write version.txt");
    }
    [major, minor, patch]
}

/// Today's date (UTC) as `YYYY-MM-DD`.
fn today() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).expect("clock before 1970").as_secs();
    let days = (secs / 86_400) as i64;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}
