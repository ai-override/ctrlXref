# File menu

The File menu opens the game's control profile and a rebind file on top of it, saves the
rebind file, and switches the color theme.

ctrlXref works with two kinds of files:

- **defaultProfile** (`defaultProfile.xml`): the game's default controls, with every
  actionmap and action. ctrlXref shows its actionmaps in the tree on the left and its
  actions in the table on the right. The tree shows the game's own titles from its English
  texts. Hover over a row to see its technical name. A title in italics is made from the
  technical name, because the game has no text for it. When several rows share a title, the
  technical name follows it in parentheses.
- **Rebind file** (`actionmaps.xml`, or a mappings file you exported): your changes to the
  default controls. ctrlXref matches its bindings to the profile's actions by actionmap and
  action name and shows them in the table's rebind column.

Open the profile first. The rebind commands need it, since they match the rebind file
against it.

## Open defaultProfile (Ctrl+O)

Picks a `defaultProfile.xml` to open and shows its actionmaps and actions. The file
browser starts in the game's `<game folder>\Data\Libs\Config` folder with
`defaultProfile.xml` already filled in, so pressing **Open** opens the extracted default
profile. If that folder doesn't exist yet, it starts in the game folder.

When the game folder is set (**Source > Locate Star Citizen Game Folder...**), ctrlXref
opens the extracted `Data\Libs\Config\defaultProfile.xml` of that folder by itself at
startup. Use this command to open a different profile.

## Open Current Rebind File (Ctrl+R)

Opens the rebind file the game uses now:

`<game folder>\LIVE\user\client\0\Profiles\default\actionmaps.xml`

It needs the game folder to be set.

## Open Custom Rebind File... (Ctrl+Shift+O)

Picks any rebind file to open, for example a mappings file you exported from the game. The
file browser starts in the game's mappings folder,
`<game folder>\LIVE\user\client\0\controls\mappings`, when it exists.

## Editing bindings in the table

With a rebind file open, double-click an empty **keyboard**, **mouse**, **joystick** or
**gamepad** cell, or one that shows a rebind, to edit it. Press **Enter** or click
elsewhere to keep the change, or **Escape** to cancel it. The other columns come from the
profile and can't be edited.

- Separate several bindings with commas or vertical bars: `up, numpad8` or
  `up | numpad8`. Each one becomes a `<rebind input="..."/>` element of its own. Cells
  with several bindings show them separated by ` | `.
- Bindings typed without a device prefix get the column's prefix: `kb1_` for keyboard,
  `mo1_` mouse, `js1_` joystick and `gp1_` gamepad. So `up` in the keyboard column becomes
  `kb1_up`. A binding with another device's prefix, like `js1_button2` in the keyboard
  column, is refused.
- Edits only ever change the rebind file. `defaultProfile.xml` is never written.
- The **DEFAULT from: Data.p4k R/O** column, in light blue, shows the default bindings that come from the
  game engine through `defaultProfile.xml`, for all devices. It is read-only:
  double-clicking it does nothing. The device columns show only the rebind file's
  bindings, so every device cell can be edited.
- Clearing a cell that shows a rebind removes those rebinds, and the action's default for
  that device applies again.
- For an action that the rebind file doesn't have yet, an `<action>` element (and its
  `<actionmap>`, if needed) is added. Clearing all of an action's bindings removes its
  `<action>` element again.
- A cell with more than one binding for the same device is shown in red, as a possible
  conflict. Empty rebinds such as `js1_ | js2_ | js3_` bind nothing and don't count.

The edits stay in memory until you save the rebind file.

## Selecting and copying cells in the table

Copied cells paste into Excel and other tables as separate cells.

- **Drag** with the mouse to select a rectangle of cells. Dragging near or past the table's
  edge scrolls it. The tree stays on the row where the drag started.
- **Ctrl+A** selects the whole table.
- **Ctrl+C** copies the selected cells in the order shown. With no cells selected, it copies
  the selected row.
- Only **Ctrl+A** copies the column headers too.
- Clicking a cell, using the tree or the arrow keys, sorting, or changing the table clears the
  selection.

The text is tab-separated, with a line for each row. Tabs and line breaks inside a cell become
spaces, and blank cells stay empty, so the rows and columns stay lined up. Excel may read a
value that starts with `=`, `+`, `-` or `@` as a formula.

A Java `JTable` can only paste if its application adds that: `JTable` copies by itself but
doesn't paste. Handlers for this usually split on tabs and line breaks, which fits this text.

## Save Rebind File (Ctrl+S)

Writes the open rebind file back to where it was opened from, including the bindings that
**Tools > Autofill All Unbound Keyboard Controls** added.

When the open file is the game's current rebind file,
`<game folder>\LIVE\user\client\0\Profiles\default\actionmaps.xml` (or the same file of
another channel, such as PTU), a warning asks first: the game engine overwrites that file
when you modify controls inside the game. Choose **Save** to write it anyway or **Cancel**
to leave it unchanged. To keep your bindings safe, save a copy with **Save Rebind Copy
As...** as well.

After saving, a message offers to show the file: **Open in Explorer** opens its folder in
File Explorer with the file selected. **Save Rebind Copy As...**, **Create Blank Rebind
File...**, **Tools > Export to .csv...** and **Tools > Export to .json...** offer the same.

## Save Rebind Copy As... (Ctrl+Shift+S)

Writes the open rebind file to a new file, as a backup or a copy to edit later. The name
suggested is the original name with `_copy` added. After this, **Save Rebind File** still
writes the original file, not the copy.

## Create Blank Rebind File...

Writes a rebind file that clears **every** binding of the open profile, for starting your
controls from scratch. It needs the profile to be open.

The game ships a blank layout, `Data\Libs\Config\Mappings\layout_all_blank.xml`, but it
lists only part of the actions, and some of those are out of date. ctrlXref reads it (from
next to the open profile, else from the game folder; **Source > Extract Defaults from
Data.p4k Package** unpacks it) and keeps its `CustomisationUIHeader` with the `Devices`
list. The created file then holds exactly the profile's actions, since `defaultProfile.xml`
is the more up-to-date of the two:

- Each profile action the layout lacks is added with the same blank `<rebind>` elements the
  layout's own actions have. Missing actionmaps are added after the existing ones.
- Layout actions that aren't in the profile are removed, and so are actionmaps left empty.

The file browser starts in the game's mappings folder with
`layout_all_blank_complete.xml` filled in. After saving, a message offers **Open Rebind
File** to open the new file for editing, and **Open in Explorer** to show it in File
Explorer. Opening it replaces a rebind file already open, and changes to that file that were
not saved are lost. **Close** leaves the new file unopened; you can open it later with
**Open Custom Rebind File...**.

Empty rebinds such as `kb1_ ` (or the blank layout's `device="keyboard" input=" "`) are
kept as they are when a rebind file is opened and saved: in the game they unbind that
device's default binding. The table shows them with their device prefix, like `kb1_ `,
and editing the cell replaces them.

## Close (Ctrl+W)

Closes the profile and the rebind file and empties the tree and the table. Changes that
were not saved are lost.

## Theme

- **Light** and **Dark** use that theme. Dark takes its colors and heading font from the
  Ship Matrix on the Roberts Space Industries website.
- **System** follows the Windows light or dark setting and changes with it.

The sun / moon button at the right end of the title bar switches between light and dark.

## Exit (Ctrl+Q)

Closes ctrlXref. Changes that were not saved are lost.
