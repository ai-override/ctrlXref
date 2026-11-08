# Tools menu

The Tools menu finds free keyboard combinations, lists the game's names for its controls, assigns
combinations to actions that have no keyboard binding, and exports the actions to a spreadsheet file or a structured JSON file.

All tools except **Show Control Names** need the profile to be open (**File > Open
defaultProfile**).

## Key combinations

A key combination here is one or two modifiers plus one key:

- **Two-key**: one modifier and a key, for example `lalt+k`.
- **Three-key**: two modifiers of different kinds and a key, for example `lctrl+lshift+k`.

The modifiers are `lalt`, `ralt`, `lctrl`, `rctrl`, `lshift` and `rshift`. Left and right
count as different modifiers.

Combinations that Windows uses, such as Alt+F4, Alt+Tab, Ctrl+Esc, Ctrl+Shift+Esc and
Ctrl+Alt+Del, are left out.

## Unused Key Combinations...

Lists the key combinations that no binding uses, grouped by modifier. The summary at the top
says how many combinations are unused and which files' bindings were counted:

- the open profile
- the open rebind file
- the game's current rebind file, when it is not the open one

The **Copy** button copies the summary and the whole list.

## Resolve Control Conflicts

Goes through the conflicting cells, the red ones, from the top of the table to the bottom in the
current sort order, opening each one for editing. Delete the controls you don't want.

- **Tab** or **Enter** keeps what the cell now holds and goes to the next conflict, and
  **Shift+Tab** to the previous one, also when the conflict is left unresolved. From the last
  conflict, **Tab** goes around to the first one.
- Leftover ` | ` separators are removed when you leave a cell, and a binding whose device prefix
  got partly deleted gets it back, so `s3_button1` stays `js3_button1`. A prefix with no control
  left, like `js3`, shows a warning and the cell keeps its old value.
- **Esc** stops and leaves the current cell as it was. Clicking elsewhere also stops, keeping
  what you typed.
- It ends by itself when no conflicts are left. Changes are written only when you save the
  rebind file.

## Show Control Names

Opens a window that lists the game's names for its controls, centered on the main window. It
reads `keybinding_localization.xml` next to the open profile, or else
`<game folder>\Data\Libs\Config\keybinding_localization.xml`.

- The columns are **name**, **localizationString**, **shortLocString** and **Device**. Their `@key`
  references are replaced with the English texts of `global.ini`. A reference the game has no text
  for stays as it is. Few controls have a short name. **Device** says `joystick` without a number.
- **EXAMPLE** shows each control written as in the actions table's device columns, such as
  `js1_button21`, `mo1_maxis_x`, `gp1_a` or `kb1_f1`. Keyboard modifiers are shown in a combination
  (`kb1_ralt+o`), and so are the mouse controls in the keyboard's list (`kb1_lctrl+mouse3`).
- Of the joysticks, only `joystick2` is listed, since the file repeats the same list for all four
  and `joystick1` has a typo in it. So their texts say **(Input 2)**. The same name can still be on
  several devices, such as `a` on the keyboard and the control pad; **Device** tells them apart.
- Click a column header to sort: ascending, descending, then the file's order. Right-click it to
  choose.
- Selecting and copying work as in the main table: drag, **Ctrl+A**, **Ctrl+C** and the
  right-click menu.

## Autofill All Unbound Keyboard Controls

Gives every action without a keyboard binding the next unused key combination, in the
order of the table. The new bindings are added to the open rebind file, so open one first
(**File > Open Current Rebind File** or **Open Custom Rebind File...**).

When there are more unbound actions than unused combinations, the remaining actions stay
unbound. The message afterwards says how many actions were filled.

> The new bindings are only in memory until you save. Use **File > Save Rebind File** to
> keep them, or **File > Save Rebind Copy As...** to try them out in a copy first.

## Export to .csv...

Saves the table as a `.csv` file that spreadsheet programs such as Excel open. A save dialog
asks for the file name; it suggests the name of the open rebind file (or the profile) with
`.csv`. The dialog starts in the game's mappings folder,
`<game folder>\LIVE\user\client\0\controls\mappings`, or next to the open file when the
game folder isn't set or that folder doesn't exist. After saving, **Open in Explorer** opens
that folder in File Explorer with the file selected.

- The rows are in the table's current sort order. Sort by a column first to export in that
  order.
- The first column, **actionmap**, holds each action's actionmap name. The table's columns
  follow, as shown, with rebinds in place of the defaults they replace.
- Fields are separated by the Windows list separator (**Settings > Time & language >
  Region > Additional settings > List separator**), usually `,` or `;` depending on your
  locale, so the spreadsheet splits them into columns. Fields that contain the separator,
  such as several bindings in one cell, are put in quotes.
- Blank cells, like the profile's `keyboard=" "`, are written empty (`,,` or `;;`).
- The file is UTF-8 encoded.

## Export to .json...

Saves the actionmaps and their actions as a structured `.json` file. The save dialog works
like the one of **Export to .csv...**, and suggests the same name with `.json`.

- The file follows the tree on the left: a list of actionmaps, each with its list of actions,
  in the order of the tree. The table's sort order doesn't affect it.
- An actionmap has its `name`, its `UILabel` and its `actions`. An action has the table's
  columns as keys, as shown, with rebinds in their device columns. Several bindings in one
  cell stay one text, separated by ` | `.
- Empty values are left out, including blank ones like the profile's `keyboard=" "`, and so
  is `actions` for an actionmap without actions.
- Empty rebinds like `kb1_ ` are kept, as in the table: in the game they unbind that device's
  default.
- The file is UTF-8 encoded.
