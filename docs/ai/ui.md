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
| `button` | an action | `Primary(label)`: what the pane is for, the accent fill. `Secondary(label)`: outlined and filled, beside a primary. `Icon(svg)`: outlined, reads as a button (the rails' collapse arrows). `Ghost(Face)`: no outline at rest, a header's small words (`Stop`, `Resume`) and icons (`+`) |
| `pill` | a short label in a fill | `Key { lit, small }`: a key cap, lit while held. `Mode(accent)`: the status bar's mode |
| `lifted` | a dialog over the panes (the new-task box) | none: a faint ring, a close and a deep shadow, a light along its top |
| `well` | a field set into a `lifted` dialog | its edge's accent |
| `chip` | an attachment in the new-task box: a meeting, the workspace in front, a folder | an accent for its text, or none; the caller adds its × |
| `list_row` | a row of a list | `Normal`: a sidebar row, full colour. `Dim`: a row under a heading (a checkout), muted until hover or the keys |
| `section_heading` | a group's heading in a list | open (▾) or folded (▸), its name and count; a click folds it |
| `panel_header` | the top of a panel (the sidebar) | its title in capitals; the caller adds its actions, last its collapse arrow (`Button::Icon`) |
| `rail_cell` | a cell of the left or right rail | `Side::Left`/`Right`; `open`: raised in the panel's background with an accent bar on the window's edge and no line toward the panel, joined to it |
| `tooltip` | text under the pointer | `Hint`: a tooltip. `Drag`: what a dragged pane or tab carries |
| `notice_line` | a toast card's line | by `notice::Level`: its colours and glyph |
| `divider` | the grab of the line between two panes | by `Axis`: `Right` a vertical line, `Down` a horizontal one. `DIVIDER_GRAB` across (a pane's border and padding each side), the resize pointer; hover lights a line down its middle, `Pressed` (dragging) brighter |
| `shimmer` | an agent's working words (the drawer's working line, a running task's row) | by `phase`: a brighter band sweeping across, or none (still: Reduce Motion, waiting on the user). Colours only, `MUTED` to `TEXT`, never amber; `Sweep` asks for its frames while one is drawn |

## Rules

- **One primary button per pane.** It is what the pane is for. Everything else is secondary,
  ghost or icon.
- **Keys appear in tooltips only.** `Collapse into the rail · ⌘⇧L`, never a key on the control.
- **No explanatory sentences.** Make it clear up front: a label, an icon, a state's look. A
  tooltip names what a control does, in a few words.
- **Untrusted text is marked.** Text a model, a meeting or another app wrote is drawn as data
  (quoted, labelled or in its own muted style), never as ottospace's own words or instructions.
- **Every button is a real control with a sneak label.** A clickable element has an id, a
  control mark and a click; sneak mode reaches it as a hand would.

## Gallery

`{"cmd":"ui_gallery","on":true}` (a test run only, `control::test_refusal`) draws every component
in every state over the window. `scripts/shot ui-gallery` pictures it, so `shots.yml` compares a
change to any component. A new component or variant goes in `ui/gallery.rs`.
