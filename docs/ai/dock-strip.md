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
| `zoom share statusbar window` | `AXSystemDialog` | 334 × 22 | Only while sharing: "You are screen sharing", 3 buttons |

- **A built-in `ignore` names both** (`rules::built_in`, `zoom share*`): by default never offered,
  not in the picker, never moved. A user's `manage` rule undoes it (`rules::first_of`), and the
  picker then lists the bar (`rules::pickable`). The reason recorded: "moving them breaks those
  controls" (zoom-ax.md); no test that shows it is written down. It is the first thing to measure
  (slice 0).
- **Not a slot candidate even then.** With no rule, `rules::manages` is true only for a standard
  window, and a slot's candidates must pass it (`slot::Look::candidates`; slots.md rule 4).
- **Where it sits is the owner's observation** (it floated over the panes in the call): zoom-ax.md
  leaves positions out, and its window level is not measured.
- **Stop share** is not among the toolbar's 16 buttons (zoom-ax.md): it may be on the status bar.
- **Its size is its own.** Whether it takes another width or height when asked is not measured.
- **It comes and goes with the share.** Zoom's main meeting window is not listed for the whole
  share and comes back after ([slots.md](slots.md) §3, Away). The built-in Zoom rule's `meeting`
  selectors name the toolbar; a slot's Away held by it is planned (slots.md slice 5), and today
  Away ends by time alone.

## How hosting copes with a size today

A tiled pane asks its window for the pane's size; an app may refuse.

- **A window that refuses** (the canary's `fixed` and `min`) is shown at the size it kept, unscaled,
  at the pane's top left: parked, its picture cropped to the pane; live, the window itself there
  (`Hosted::show`;
  `e2e_apps.rs::a_window_that_refuses_the_pane_size_is_shown_unscaled_at_the_pane_top_left`).
- **The width it kept** is remembered as its minimum (`rules::refused_width`, `Hosted::min_width`)
  and only logged: no layout rule uses it.
- **`fit` (⌘⌃0) moves the splits** around a pane to its window's size, once, as far as each side's
  10% allows (reference.md). Splits stay ratios: a resize of ottospace's window undoes it.

## Options

**(a) A dock strip.** A pane pinned to the top or bottom edge of the workspace (or of one pane),
outside the tiling ratios: as tall as the hosted window's own height (58 pt for the share bar),
as wide as the workspace, the window centred in it at its own width. The other panes tile in the
rest. It never takes focus by itself: a click in it enters it as any app pane, and leaving gives
the keyboard back. Entered and left as any app pane (a click or a scroll enters, the return key
or a click on ottospace leaves); parked (a picture) otherwise.
- Costs: a new layout piece (a strip beside the tree, not a leaf in it: `Node` stays a tree of
  ratios), the panes' bounds less the strip, a pane kind or slot that knows its edge, `layout.json`.
- Gives: the bar has a fixed place that covers nothing; its height is the window's.

**(b) An ordinary small pane.** Split a pane off and put the bar in it.
- What breaks: the ratio gives it a height the bar does not want (a down split halves the pane:
  hundreds of points for a 58 pt bar), so it either stretches (if Zoom lets it) or sits at the top
  left of a mostly empty pane (as a refusing window does today). `fit` or the divider sizes it, and
  again after every resize of ottospace's window. Needs only the ignore lifted (or the user's
  `manage` rule, today) for the offer and the picker; a slot of it also needs slots.md rule 4 to
  admit a non-standard window.
- Gives: something today, from the slice that lifts the ignore and offers the bar.

**(c) Leave it floating.** As now. Zoom puts it over the top of the shared display, over a pane's
header or ottospace's top row; it stays above every window. Costs nothing; covers what it lies over.

## Behaviour (for (a))

- **Appears with the window, goes with it.** While no such window is listed the strip takes no
  room: the panes do not move for it. Not a waiting slot that shows "Waiting for…" in a strip of
  its own: a share starting or ending would otherwise leave an empty band. The strip keeps its
  selector and edge, so the next share's bar is offered there. Taken without asking only if the
  user turns that on for the strip (as a slot's `Automatic`, with its rules: the tab on show, never
  the window the user is in); open question 2.
- **Its level.** Zoom keeps the bar above all windows. Parked, ottospace draws a picture of it and
  the real bar sits on the parking spot; live, the real bar lies over the strip. Whether Zoom puts
  it back on top of the shared display by itself (it may re-place it at each share, or when it
  thinks the display changed) is to measure.
- **How the person asks.** The same offer as any new window: `Zoom opened a window · ◫ split
  right · ⊟ split down · ⤒ dock top · ⤓ dock bottom · ×`, the dock choices shown only for a window
  shorter than a set height (say 120 pt). Never docked by itself.
- **Displays.** A strip belongs to the main window's tab, so to the display that window is on, not
  to the shared display. Sharing the display ottospace is on: the strip is there. Sharing another:
  docking pulls the bar off the shared display into ottospace's (open question 3). A tab holding an
  app pane may fold out ([multi-monitor.md](multi-monitor.md)); a strip in a window out is not
  designed yet.
- **The status bar.** `zoom share statusbar window` (334 × 22, 3 buttons) stays ignored at first:
  only the toolbar is in scope. Where Stop share lives decides whether it joins (slice 0).

## Slices

0. **Measure, by hand, structure only.** In a real share, with the bar ignored: move it with AX to
   another spot and resize it; then press each of a few buttons with the mouse (Chat, Participants)
   and see that they still work; note where Zoom puts it, its window level, whether Zoom moves it
   back, and which window holds Stop share. No button pressed by ottospace. Says whether the
   ignore's reason holds. Before that, the owner can try (b) with a `manage` rule of the exact title.
1. **(b), if slice 0 says moving is safe.** Narrow the built-in `zoom share*` ignore to
   `zoom share statusbar*`; the bar is offered and listed in the picker. Evidence: core test of the
   rule, e2e-apps with a canary window 900 × 58.
2. **The strip in the core.** A strip per tab (`top`/`bottom`, a selector, the pane in it),
   bounds from the window's height passed in an action, `layout.json`; the offer's dock choices.
   The selector never in the state (`#[serde(skip)]`, slots.md rule 5), held by a property check.
   Property makers; no UI.
3. **The strip on screen.** Drawing and bounds by e2e with a fixture window; hosting into it, live,
   parked and focus by e2e-apps with the canary (CI only).
4. **Comes and goes.** The offer again at the next share; the opt-in "take it", with slots'
   admission of a non-standard window (rule 4).

## Recommendation

(a), after slice 0, with slice 1's (b) as the first step. The bar's whole point is a fixed small
size; a tiling pane fights that every resize, and a strip that takes no room while no share is on
moves nothing the rest of the time. If slice 0 shows moving the bar breaks it, stop at (c) and
keep the ignore.

## Open questions for the owner

1. Workspace-wide strip, or one pinned to a pane's edge (above the meeting's pane)? The note
   assumes workspace-wide: one bar, one place.
2. After the first dock, offered each time (the default here), or an opt-in to take it without
   asking?
3. Sharing another display: is pulling the bar onto ottospace's display what you want, or should
   the strip only hold it when you share ottospace's display?
4. Is Stop share on the status bar? If so, should that dock too (or the strip hold both)?
5. Any other window you'd dock (Zoom's reactions bar, a recording control, a timer)? That sets the
   height limit for offering "dock".
