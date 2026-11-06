# Tools menu

The Tools menu finds free keyboard combinations, assigns them to actions that have no
keyboard binding, and exports the table to a spreadsheet file.

All tools need the profile to be open (**File > Open defaultProfile**).

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
