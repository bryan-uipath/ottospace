# Dock strip: a home for an app's small accessory window

Not built. A concept for windows such as Zoom's share control bar, which no pane holds today.

The owner, 2026-11-19, after a real Zoom call while sharing: Zoom's floating share control bar
(Audio / Video / Participants / Chat / Share / Stop share) "would be neat to be able to 'dock'
either at the top or bottom and have a pane for it too. Wonder if there could be a concept for
that (or if we should just have an existing pane)."

## The window

From [zoom-ax.md](1-initial-brainstorm/zoom-ax.md) (measured 2026-11-12, one Zoom version):

| Window | Subrole | Size | There |
|---|---|---|---|
| `zoom share toolbar window` | `AXSystemDialog` | 961 × 58 | Only while the user shares: 16 buttons |
| `zoom share statusbar window` | `AXSystemDialog` | 334 × 22 | Only while sharing: "You are screen sharing" |

- **A built-in `ignore` names both** (`rules::built_in`, `zoom share*`): never offered, not in the
  picker, never moved. The reason recorded: "moving them breaks those controls". That was not
  measured: no one moved the bar and pressed a button. It is the first thing to measure (slice 0).
- **It floats above other windows** (a system dialog; its window level is not measured) and Zoom
  places it itself, at the top of the shared display, over whatever is there.
- **Its size is its own.** Whether it takes another width or height when asked is not measured.
- **It comes and goes with the share.** Zoom's main meeting window is not listed for the whole
  share and comes back after ([slots.md](slots.md) §3, Away). The built-in Zoom rule's `meeting`
  selectors name the toolbar, so a slot already uses it as the sign that a share is on.

## How hosting copes with a size today

A tiled pane asks its window for the pane's size; an app may refuse.

- **A window that refuses** (the canary's `fixed` and `min`) is shown at the size it kept, unscaled,
  at the pane's top left: parked, its picture cropped to the pane; live, the window itself there
  (`Hosted::show`;
  `e2e_apps.rs::a_window_that_refuses_the_pane_size_is_shown_unscaled_at_the_pane_top_left`).
- **The width it kept** is remembered as its minimum (`rules::refused_width`, `Hosted::min_width`)
  and only logged: no layout rule uses it.
- **Nothing sizes a pane from its window.** Splits are ratios; a pane is as big as the tree says.

## Options

**(a) A dock strip.** A pane pinned to the top or bottom edge of the workspace (or of one pane),
outside the tiling ratios: as tall as the hosted window's own height (58 pt for the share bar),
as wide as the workspace, the window centred in it at its own width. The other panes tile in the
rest. It never takes focus by itself: a click in it enters it as any app pane, and leaving gives
the keyboard back. Live while the pointer is in it, parked (a picture) otherwise, as every app pane.
- Costs: a new layout piece (a strip beside the tree, not a leaf in it: `Node` stays a tree of
  ratios), the panes' bounds less the strip, a pane kind or slot that knows its edge, `layout.json`.
- Gives: the bar has a fixed place that covers nothing; its height is the window's.

**(b) An ordinary small pane.** Split a pane off and put the bar in it.
- What breaks: the ratio gives it a height the bar does not want (a down split halves the pane:
  hundreds of points for a 58 pt bar), so it either stretches (if Zoom lets it) or sits at the top
  left of a mostly empty pane (as a refusing window does today). The user drags the divider to fit
  it, and again after every resize of ottospace's window. No new code but lifting the ignore.
- Gives: something today, from the slice that lifts the ignore and offers the bar.

**(c) Leave it floating.** As now. Zoom puts it over the top of the shared display, over a pane's
header or ottospace's top row; it stays above every window. Costs nothing; covers what it lies over.

## Behaviour (for (a))

- **Appears with the window, goes with it.** While no such window is listed the strip takes no
  room: the panes do not move for it. Not a waiting slot that shows "Waiting for…" in a strip of
  its own: a share starting or ending would otherwise leave an empty band. The strip keeps its
  selector and edge, so the next share's bar comes back to it (as a slot's Away return), offered
  the first time, then taken without asking when the user said so (open question 2).
- **Its level.** Zoom keeps the bar above all windows. Parked, ottospace draws a picture of it and
  the real bar sits on the parking spot; live, the real bar lies over the strip. Whether Zoom puts
  it back on top of the shared display by itself (it may re-place it at each share, or when it
  thinks the display changed) is to measure.
- **How the person asks.** The same offer as any new window: `Zoom opened a window · ◫ split
  right · ⊟ split down · ⤒ dock top · ⤓ dock bottom · ×`, the dock choices shown only for a window
  shorter than a set height (say 120 pt). Never docked by itself.
- **Displays.** A strip belongs to its tab's window, so to the display that window is on, not to
  the shared display. Sharing the display ottospace is on: the strip is there. Sharing another:
  the bar is pulled off the shared display into ottospace's (which is what the owner asked for, and
  the reason to measure first). A tab folded out ([multi-monitor.md](multi-monitor.md)) keeps its
  strip in its own window.
- **The status bar.** `zoom share statusbar window` (334 × 22) stays ignored: it is a label, not
  controls.

## Slices

0. **Measure, by hand, structure only.** In a real share, with the bar ignored: move it with AX to
   another spot and resize it; then press each of a few buttons with the mouse (Chat, Participants)
   and see that they still work; note its window level and whether Zoom moves it back. No button
   pressed by ottospace. Says whether the ignore's reason holds.
1. **(b), if slice 0 says moving is safe.** Narrow the built-in `zoom share*` ignore to the status
   bar (and annotation windows); the bar is offered and listed in the picker like any window.
   Evidence: core test of the rule, e2e-apps with a canary window 900 × 58.
2. **The strip in the core.** A strip per tab (`top`/`bottom`, a selector, the pane in it),
   bounds from the window's height passed in an action, `layout.json`; the offer's dock choices.
   Property makers; no UI.
3. **The strip on screen.** Drawing, hosting into it, live and parked, e2e with a fixture window.
4. **Comes and goes.** Away and return with the share, as a slot; the remembered "dock it".

## Recommendation

(a), after slice 0, with slice 1's (b) as the first step. The bar's whole point is a fixed small
size; a tiling pane fights that every resize, and a strip that takes no room while no share is on
moves nothing the rest of the time. If slice 0 shows moving the bar breaks it, stop at (c) and
keep the ignore.

## Open questions for the owner

1. Workspace-wide strip, or one pinned to a pane's edge (above the meeting's pane)? The note
   assumes workspace-wide: one bar, one place.
2. After the first dock, should the next share's bar go to the strip without asking (as a slot's
   Away return does), or be offered each time?
3. Sharing another display: is pulling the bar onto ottospace's display what you want, or should
   the strip only hold it when you share ottospace's display?
4. Any other window you'd dock (Zoom's reactions bar, a recording control, a timer)? That sets the
   height limit for offering "dock".
