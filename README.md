# ctrlXref Star Citizen Control Bindings Cross-reference Utility


A Windows desktop tool for viewing and modifying [Star Citizen](https://robertsspaceindustries.com/) control bindings. ctrlXref cross-references the game's default control profile with your own custom rebind files. You will be able to see and modify every action and its bindings in a single table. Merging multiple custom rebind files from various sources is now supported for creating a new custom binding with all your preferred bindings.
### What it's not
ctrlXref does not identify as [Star Citizen](https://robertsspaceindustries.com/) control profile editor as the application is lacking proper modeling of various devices and the controls they provide. You can manipulate the controls manually inside the table cells but there's no dialogs, dropdowns, key or button press, etc. support. You are encouraged to use the Game integrated control editor or some other 3rd party solutions (as the great: [Star Citizen Profile Editor](https://github.com/Osiris-DevWorks/sc-profile-editor)) to actually create your control settings.  

> [!NOTE]
> ctrlXref is an unofficial fan project and is not affiliated with or endorsed by Cloud Imperium Games.

> [!WARNING]
> Work In Progress! This application is still a early development release with testing of the functionality hardly started.
> Make manual backups of your controls under `StarCitizen\LIVE\user\client\0\controls\mappings\` before using ctrlXref 
> as modifications made by this application are ***not reversible*** and might not work within the actual game.

> [!NOTE] 
> - TODO Update / improve the integrated Help documentation.
> - TODO ***PDF*** File export?
> - TODO Printer support (probably doable using the ***Light theme*** quite easily)?


## Features

- Browse all ***\<actionmap>*** and ***\<action>*** elements from `defaultProfile.xml`, using the game's own `English labels.`
- See default bindings side by side with your rebinds for keyboard, mouse, joysticks and gamepad.
- Edit bindings directly in the table, with possible conflicts shown in red.
- Merge multiple custom rebind files to create your preferred control setup and confirm which controls are truly bound to what action.
- Conflicting controls manual resolving support (TODO improved edit features later).
- List the game's control name list with examples in a sortable table.
- Select cells with a mouse drag or `Ctrl+A` and copy them with `Ctrl+C`, as tab-separated text that pastes into Excel and other tables.
- Search the table and the tree with regular expressions (`Ctrl+F`, `F3`, `Shift+F3`), keeping previous searches in a dropdown list.
- Open the game's current `actionmaps.xml` or any mappings file you exported from the game.
- Create a blank rebind file that "clears" every binding, for starting from scratch. ***Behavior inside the game UNTESTED***.
- List unused key combinations and allows "sequential" assignment of them to actions with no keyboard binding.
- Export the table to a `.csv` file for use in a spreadsheet in the current table sorting order, or to a structured `.json` file that follows the tree.
- Switch between ***dark*** and ***light*** themes (Light theme lacking the game styling and feel).

ctrlXref writes only to rebind files. It never changes `Data.p4k` or `defaultProfile.xml`. Saving the current bindings `actionmaps.xml` spawns a Warning as the file will be overwritten by the game engine on configuration modifications.

## Requirements

- Windows 10 or later 64bit
- A Star Citizen installation (or a valid Star Citizen Game `Data.p4k` under `StarCitizen\LIVE\` folder)
- The [unp4k tools](#install-the-unp4k-tools), to extract the default controls from the game data
- [Rust](https://www.rust-lang.org/tools/install) (current stable, with the MSVC toolchain), to build from source

## Getting started

### Build

```powershell
git clone https://github.com/ai-override/ctrlXref.git
cd ctrlXref
cargo build --release
```

The application is built as `target\release\ctrlxref.exe`.

### Install the unp4k tools

The game's default controls are packed inside `LIVE\Data.p4k`, so ctrlXref uses tools from the [unp4k](https://github.com/dolkensp/unp4k) project to get them out:

| File | Why it is needed |
| --- | --- |
| `unp4k.exe` | Unpacks the selected files from the `Data.p4k` archive. |
| `unforge.cli.exe` | Converts the unpacked XML files from the game's binary CryXML format into plain XML that ctrlXref can read. |
| `x64\libzstd.dll` | Lets unp4k decompress the contents of the archive. |

To install them:

1. Download the Windows package from the [unp4k releases](https://github.com/dolkensp/unp4k/releases) page, for example `unp4k-suite-win-x64-v4.0.87.zip`. ctrlXref has been used with v4.0.87.
2. Copy the **contents** of the zip's `publish` folder directly into your `StarCitizen` installation folder, so that the files end up like this:

```text
StarCitizen\
├── LIVE\
│   └── Data.p4k
├── x64\
│   └── libzstd.dll
├── unp4k.exe
└── unforge.cli.exe
```

ctrlXref looks for the tools only in this location. The default installation folder is `C:\Program Files\Roberts Space Industries\StarCitizen\`. The tools also write into this folder, so your account needs write access to it.

> [!NOTE]
> Older unp4k releases name the converter `unforge.exe`. ctrlXref expects `unforge.cli.exe`, so use a release that includes it.

### Extract the default controls

1. Start `ctrlxref.exe`.
2. Choose **Source > Locate Star Citizen Game Folder...** and select your `StarCitizen` folder. ctrlXref remembers it for the next start.
3. Choose **Source > Extract Defaults from Data.p4k Package**. If the `Data` folder doesn't exist yet, ctrlXref offers to do this at startup.

When extraction finishes, ctrlXref opens the default profile. Extract again after each game patch to pick up the new defaults. The tools' full output is saved to `ctrlxref_extract.log` in the game folder, which helps when an extraction fails.

## Why only part of `Data.p4k` is extracted

ctrlXref extracts only the files needed and creates just those parts of the `Data` folder structure under `StarCitizen`:

```text
StarCitizen\Data\
├── Libs\Config\
│   ├── defaultProfile.xml             Default controls
│   ├── keybinding_localization.xml    Device and key names
│   └── Mappings\
│       └── layout_all_blank.xml       The game's blank control layout
└── Localization\english\
    └── global.ini                     English texts for the action labels
```

This keeps the folder compact and saves a lot of time. A full extraction can take from several minutes to hours in the worst case, depending on your disk and the size of the game. As an example, one 150 GB `Data.p4k` unpacked to almost 1.5 million files taking up about 240 GB.

***BUG*** inside the `unp4k.exe` also unpacks the large game database, `Game2.dcb`, whichever files are requested. ctrlXref deletes it before the conversion step, as the rest of the process doesn't need it.

> [!TIP]
> The conversion step processes everything in the `Data` folder, so it takes longer if the folder already holds a full extraction.

## Usage

Once the game folder is set, ctrlXref opens the extracted default profile at startup. Use **File > Open Current Rebind File** to load your active bindings, or **File > Open Custom Rebind File...** to load an exported mappings file. Edits stay in memory until you save them.

> [!WARNING]
> The game overwrites its current `actionmaps.xml` when you change controls in-game. Use **File > Save Rebind Copy As...** to keep a backup of your bindings.

You can also pass the files to open on the command line:

```text
ctrlxref.exe [profile.xml [rebind.xml]]
```

| Argument | Description |
| --- | --- |
| `profile.xml` | Profile to open, relative to the game folder or as an absolute path. Defaults to `Data\Libs\Config\defaultProfile.xml`. |
| `rebind.xml` | Rebind file to open, relative to the current directory. |

The full documentation is built into the application and is also available in [`docs/help`](docs/help):

- [File menu](docs/help/file.md)
- [Source menu](docs/help/source.md)
- [Tools menu](docs/help/tools.md)

## Development

```
cargo run       # Run a debug build
cargo test      # Run the tests
```

`version.txt` holds the application version. `build.rs` raises its patch number on the first build of each day with source changes, so the file regularly shows as modified.

## License

ctrlXref is released under the [MIT License](LICENSE). You are free to use, copy, modify, merge, publish, distribute, sublicense and sell copies of the software, as long as the copyright notice and license text are included in all copies or substantial portions of it. The software is provided "as is", without warranty of any kind. See the [`LICENSE`](LICENSE) file for the full terms.

Third-party components keep their own licenses:

- [unp4k](https://github.com/dolkensp/unp4k) is a separate project under the MIT license. It isn't included in this repository.
- The Electrolize font is under the SIL Open Font License. See [`assets/fonts/OFL.txt`](assets/fonts/OFL.txt).


### Donations
If you find this utility useful, consider making a small donation helping me with my huge AI expenses and my extensive Coffee consumption: [classloader](https://paypal.me/cassoverride?target=_blank)

[comment]: # (Check the functionality of the paypal.me link as there are constant/random errors in PayPal account settings modifications and many submits and button presses return Try Again Later Errors and Warnings)

### Discord
My Discord: [primordial](https://discord.com/users/primordial_23184)

## Release
For all of us lazy gamers, here's the pre-built exe. No installation required or even supported.
[Latest Release](https://github.com/ai-override/ctrlXref/releases)


## Screenshots
- ![Action Table Column Sorting](https://github.com/ai-override/ctrlXref/tree/main/assets/images/ctrlXref_sorting_1387x775.jpg)
- ![Conflicting Controls dindings Resolving](ctrlXref_resolve_1387x775.jpg)
- ![Autofilling Unbound Keyboard Controls](https://github.com/ai-override/ctrlXref/tree/main/assets/images/ctrlXref_autofill_1387x754.jpg)
