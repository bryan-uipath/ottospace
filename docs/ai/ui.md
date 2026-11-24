# UI components

The shell's small design system: `crates/ottospace/src/ui/`. One builder per component, its
variants an argument. Colours, spacing (`SPACE_*`), type sizes (`TEXT_*`), radii (`RADIUS_*`) and
control sizes (`CONTROL`, `ICON*`) are named in `crates/ottospace-core/src/theme.rs`; a component
takes those, not GPUI's `px_2`/`text_xs`.

A builder draws the look only. The caller adds the control mark (`self.controls.mark(..)`: its
sneak label and its rect in the state), the tooltip and the click.

## States

`ui::State`, shared by every control:

| State | Look |
|---|---|
| `Rest` | live: hover and press show as the pointer makes them |
| `Hover`, `Pressed` | that look, drawn still (the gallery) |
| `Focus` | the keys are on it (list mode, a rail's keys): list mode's colour, a ring or outline |
| `Disabled` | 40 % opacity, no hover, no pointer cursor |

## Components

| Builder | For | Variants |
|---|---|---|
| `button` | an action | `Primary(label)`: what the pane is for, the accent fill. `Secondary(label)`: outlined and filled, beside a primary. `Icon(svg)`: outlined, reads as a button (the rails' collapse arrows). `Ghost(Face)`: no outline at rest, a header's small words (`Stop`, `Resume`) and icons (`+`). `Danger(label)`: a destructive act beside others, red words, no fill. `DangerPrimary(label)`: what a dialog that exists to destroy is for, red-filled |
| `pill` | a short label in a fill | `Key { lit, small }`: a key cap, lit while held. `Mode(accent)`: the status bar's mode |
| `lifted` | a dialog over the panes (the new-task box) | none: a faint ring, a close and a deep shadow, a light along its top |
| `well` | a field set into a `lifted` dialog | its edge's accent |
| `picker` | a choice in the new-task box's edge (project, repository, agent) | its accent; `open` while its list shows; one line, cut with … |
| `chip` | an attachment in the new-task box: a meeting, the workspace in front, a folder | an accent for its text, or none; the caller adds its × |
| `list_row` | a row of a list | `Normal`: a sidebar row, full colour. `Dim`: a row under a heading (a checkout), muted until hover or the keys |
| `section_heading` | a group's heading in a list | open (▾) or folded (▸), its name and count; a click folds it |
| `panel_header` | the top of a panel (the sidebar) | its title in capitals; the caller adds its actions, last its collapse arrow (`Button::Icon`) |
| `rail_cell` | a cell of the left or right rail | `Side::Left`/`Right`; `open`: raised in the panel's background with an accent bar on the window's edge and no line toward the panel, joined to it |
| `tooltip` | text under the pointer | `Hint`: a tooltip. `Drag`: what a dragged pane or tab carries |
| `notice_line` | a toast card's line | by `notice::Level`: its colours and glyph |
| `divider` | the grab of the line between two panes | by `Axis`: `Right` a vertical line, `Down` a horizontal one. `DIVIDER_GRAB` across (a pane's border and padding each side), the resize pointer; hover lights a line down its middle, `Pressed` (dragging) brighter |
| `shimmer` | an agent's working words (the drawer's working line, a running task's row) | by `phase`: a brighter band sweeping across, or none (still: Reduce Motion, waiting on the user). Colours only, `MUTED` to `TEXT`, never amber; `Sweep` asks for its frames while one is drawn |

## Dialogs

`ui::dialog`: every centred dialog is one `frame` (`QUESTION` 360 or `FORM` 520 wide): a title (a
question or a verb phrase, 14/600), one muted sentence of purpose, × top right, the body, then the
action row.

| Builder | For |
|---|---|
| `frame` | the dialog: title, purpose, × (`close`), body, `actions` |
| `close` | × top right: the same as Esc and Cancel; never drawn with a key |
| `actions` | the row: a destructive act alone at the left, a status line (what ↵ does, what matches), the buttons, the primary right-most |
| `action` | a `button`, 26 high, its keys as caps after its label in its own ink |
| `segments`, `segment` | two to four options, one chosen (a lifted fill, white), each with its letter |
| `consequence` | the line under a choice: what the chosen option does |
| `choice_row` | a checkbox or radio row: the box is the control, the value it acts on muted |
| `text_box` | a field in a dialog; beside `segments` of is / contains / starts with, a title test |
| `row` | a form's row: a muted label right-aligned in a 96 column, the value on one left edge |
| `summary` | the rule or question in one sentence (15/500), first in the body |
| `window_list` | the windows open now, those a rule takes ticked |

- **One primary**, filled cyan, right-most, with its key. Cyan in a dialog is the primary's alone:
  never selection (a lifted fill), never information.
- **Focus is a ring** in list mode's colour; selection is a fill. Both can sit on one control.
- **Destructive**: red words, no fill, at the far left, never the default. A dialog that exists to
  destroy (delete a worktree) has a red primary that asks ⌘↵; ↵ alone does nothing there.
- **× and Esc** close every dialog and mean Cancel. Neither is drawn as a key. A question keeps its
  Cancel button (the answers are its content); a form drops it.
- **Letters on options** work only while no text field has the keys.
- **Words**: sentence case; a button names its result (`Keep slot`, `Quit`), never `OK`.
- **Backdrop**: one dim, 0.45 (`DIM`), under every centred dialog and panel (the palette, the
  key reference, the clone dialog); 0.2 (`DIM_ANCHORED`) under a list anchored to its place (⌘E).

The new-task box (`lifted`, `well`, `picker`, `chip`) is its own design (#394–#401) and not yet on
this system: its keys line and legend stay.

## Rules

- **One primary button per pane.** It is what the pane is for. Everything else is secondary,
  ghost or icon.
- **Keys appear in tooltips**, `Collapse into the rail · ⌘⇧L`, never on the control; except in a
  dialog, where each key sits on the control it presses as a cap (`ui::dialog::action`), and no
  footer line repeats them.
- **No explanatory sentences.** Make it clear up front: a label, an icon, a state's look. A
  tooltip names what a control does, in a few words. A dialog has its one sentence of purpose, and
  a choice its one consequence line.
- **Untrusted text is marked.** Text a model, a meeting or another app wrote is drawn as data
  (quoted, labelled or in its own muted style), never as ottospace's own words or instructions.
- **Every button is a real control with a sneak label.** A clickable element has an id, a
  control mark and a click; sneak mode reaches it as a hand would.

## Gallery

`{"cmd":"ui_gallery","on":true}` (a test run only, `control::test_refusal`) draws every component
in every state over the window; with `"page":"dialogs"` the dialogs' parts, `"examples"` a question
and a form whole. `scripts/shot ui-gallery`
and `scripts/shot ui-dialogs` picture them, so `shots.yml` compares a change to any component. A new
component or variant goes in `ui/gallery.rs`.
