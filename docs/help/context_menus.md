# Context menus

Right-clicking the table opens a menu of what can be done where you clicked. Changes to
bindings go to the open rebind file only, never to `defaultProfile.xml`. **Ctrl+Z** undoes
them, and nothing is written to disk until you save the rebind file.

## Column headers

Right-click a column title to choose its sort order. A plain click on the title goes
through the same three in turn.

- **Sort Ascending**: A to Z.
- **Sort Descending**: Z to A.
- **Document Order**: the order of `defaultProfile.xml`.

The current choice is shown highlighted.

## Any table cell

- **Copy to clipboard**: copies the selected cells, or the marked rows, or else the row you
  right-clicked, like **Ctrl+C**. The text is tab-separated, so it pastes into Excel and other
  tables. Right-clicking inside the selection keeps it; right-clicking elsewhere selects that
  row first, like a click.
- **Select All**: selects every cell, like **Ctrl+A**. Copying it includes the column headers.

## Device columns

The **keyboard**, **mouse**, **joystick** and **gamepad** columns have more items below a
separator. They need an open rebind file, and they work on the actions shown in the table.

### Clear All Column Bindings

Removes all of that device's bindings from the rebind file. This does not unbind those
actions: the game uses their default bindings again. It is greyed out when the column is
empty.

### Clear all kb1_ empty bindings

The item names the column's empty binding: `kb1_` keyboard, `mo1_` mouse, `jsx_` joystick
(any stick) and `gp1_` gamepad. It removes only the empty bindings, like `kb1_ ` or
`js2_ `, and keeps the real ones. It is greyed out when the column has none.

The game's control editor often leaves empty bindings behind by mistake. But an empty binding
is also how the game stores a default binding you unbound on purpose. So a warning lists the
actions first: **OK** removes the empty bindings and brings those actions' defaults back into
effect, **Cancel** changes nothing.

### Clone default bindings

In the keyboard, mouse and gamepad columns. Copies each action's default binding for that
device from `defaultProfile.xml` into the rebind file, with the device prefix: `w` becomes
`kb1_w`. Actions with several defaults get all of them.

- An action that already has a binding in the column keeps it and gets no default. This
  includes an empty binding like `kb1_ `: to give those actions their defaults, use **Clear
  all kb1_ empty bindings** first.
- `defaultProfile.xml` lists some mouse controls as keyboard bindings, like
  `keyboard="mouse1"`. Those are left out of the keyboard column and cloned into the mouse
  column instead, as `mo1_mouse1`.
- Actions whose default for the device is blank get nothing.

### Clone default bindings to

In the joystick column, instead of **Clone default bindings**. The joystick defaults don't
say which stick they are for, so pick it from the submenu, `js1_` to `js8_`. The defaults
are cloned to that stick: `button1` becomes `js2_button1` for `js2_`. An action that already
has a binding on any stick keeps it.

### Remove by * wildcards...

Removes the column's bindings that match a pattern you type. It is greyed out when the column
is empty. A box opens with a text field:

- The pattern must match the **whole** binding. `*` stands for any text, or none, and can be
  at the start, the end or in the middle. Case doesn't matter.
- As you type, the box shows how many bindings match and lists them with their actions, so
  you can check before removing anything.
- **Go** (or **Enter**) removes the matching bindings; it is greyed out while nothing matches.
  **Cancel** (or **Escape**) closes the box without changes.
- Only bindings of this column are removed; the action's other bindings stay.

| Pattern | Removes | Keeps |
|---|---|---|
| `*button*` | `js4_button24`, `js1_button3` | `js1_x` |
| `*_lalt` | `kb1_lalt` | `kb1_lalt+i` |
| `kb1_lalt+*` | `kb1_lalt+i`, `kb1_lalt+f1` | `kb1_lalt` |
| `js2_*` | every binding of stick 2 | `js1_button1` |
| `kb1_` | the empty binding `kb1_ ` | `kb1_w` |

**Ctrl+Z** brings the removed bindings back if the pattern caught more than you meant.

### Change Joystick number...

In the joystick column. Moves the bindings of one stick to another number, for example after
Windows has numbered your sticks differently. It opens a box with two lists:

- **From** lists only the stick numbers the column's bindings use now, like `js1_` and
  `js3_`. When there is just one, it is already selected.
- **To** lists all numbers, `js1_` to `js8_`; the ones in use are marked **(in use)**.

**Change** moves every binding of the From stick to the To stick: `js1_button3` becomes
`js2_button3` for `js1_` to `js2_`. Only the stick number changes; each binding keeps its other
settings, like `activationMode`. **Cancel** changes nothing.

If the To stick already has bindings, the two are merged and a warning in the box says so.
When an action then has the same binding twice, it is kept once. To swap two sticks, move one
to an unused number first: `js1_` to `js8_`, `js2_` to `js1_`, then `js8_` to `js2_`.

**Ctrl+Z** changes the numbers back.
