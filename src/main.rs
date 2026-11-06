// Release builds are GUI-only; debug builds keep a console for log output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;

/// Usage: `ctrlxref [profile.xml [rebind.xml]]`
///
/// `profile.xml` is relative to the Star Citizen game folder (an absolute path also works) and
/// defaults to the extracted `Data\Libs\Config\defaultProfile.xml`; `rebind.xml` is relative
/// to the current directory.
fn main() {
    let mut args = std::env::args_os().skip(1).map(Into::into);
    ui::run(args.next(), args.next());
}
