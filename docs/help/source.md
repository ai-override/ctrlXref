# Source menu

The Source menu tells ctrlXref where Star Citizen is installed and extracts the default
control profile from the game's data.

## Locate Star Citizen Game Folder...

Picks the Star Citizen installation folder, the one that holds `LIVE\Data.p4k`. The
default location is:

`C:\Program Files\Roberts Space Industries\StarCitizen\`

ctrlXref checks the folder. A valid folder is remembered for the next start. If it is not
a game folder, ctrlXref doesn't save it and lists what is OK and what is missing:

- the folder is named `StarCitizen`
- it has a `LIVE\Data.p4k` of at least 1 GB
- the extraction tools below, which extracting the defaults needs

The `Data` folder doesn't have to exist yet. It is created under the selected
`StarCitizen` folder when the files are extracted from the `Data.p4k` package.

ctrlXref works with the game's **LIVE** version only; PTU and other test channels are not
handled. To use the same controls on the PTU, copy your rebind files from
`<game folder>\LIVE\user\client\0\controls\mappings` to the same folder under `PTU` yourself.
A PTU build may have new, changed or removed actionmaps and actions, and ctrlXref doesn't
deal with those.

With the game folder set, ctrlXref can:

- open the extracted `Data\Libs\Config\defaultProfile.xml` by itself at startup
- open the game's current rebind file (**File > Open Current Rebind File**)
- start the custom rebind file browser in the game's mappings folder
- show action names from the game's English texts, `Data\Localization\english\global.ini`,
  instead of keys such as `@ui_COFlightPitch`

### Extraction tools

The default profile is packed inside `Data.p4k`. Extracting it needs the **unp4k** tools
in the game folder:

- `unp4k.exe`
- `unforge.cli.exe`
- `x64\libzstd.dll`

If any of them is missing, ctrlXref lists the missing files. Copy the unp4k tools into
the game folder, then locate the folder again.

## Extract Defaults from Data.p4k Package

This item is shown only when the game folder is set and the extraction tools are in it.

It unpacks these files from `LIVE\Data.p4k` into the game's `Data` folder:

- `defaultProfile.xml`, the default controls
- `layout_all_blank.xml`
- `keybinding_localization.xml`
- `global.ini`, the game's English texts

Then it converts the XML files from the game's binary format (CryXML) into plain XML that
ctrlXref can read. When extraction finishes, ctrlXref opens the default profile.

Extraction runs in the background and can take a while, since `Data.p4k` is large. The
tools' full output goes to `ctrlxref_extract.log` in the game folder, for finding out why
an extraction failed.

Extract again after a game patch so that ctrlXref shows the new defaults.

If the `Data` folder is missing at startup, ctrlXref offers to run the extraction.
