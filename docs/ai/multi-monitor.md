# Multi-monitor: a tab folded out to another display

The owner: "multi-monitor support - could we cover both windows and treat the secondary monitor as
additional panes (ie, window navigate arrows should go between the two monitors) - is that
possible?" Then: "wonder if we should just treat it as having another tab open in the other
monitor", "if we just treat each window as its own tab the collapse support is natural - the tab
just folds back in", and "we could have a key to fold a tab out".

Yes, it is possible, and this note decides how. It supersedes the parked survey
[`1-initial-brainstorm/multi-monitor.md`](1-initial-brainstorm/multi-monitor.md) (options A–D, its
experiments table is still the list of things to measure) for the model and the slices.

Not built. Nothing here was measured on two displays: the CI runners have one, and so had every run
behind this note. What GPUI 0.2.2 offers was read from its source in the cargo registry.

## The decision in one paragraph

ottospace's window stays the **main window**: Home, the top row, the left rail and the drawer are
there and only there. **⌘⇧O folds the tab on show out** to another display: a second ottospace
window opens there and shows that one tab, all panes. The tab never leaves its space or its place
in the tab list; the main window's tab row still lists it, marked as out. The focus arrows cross
from the main window's panes to that window's panes by where they are on the screens, and back.
⌘⇧O there folds it back in (the window closes; the tab is an ordinary tab again). Unplug the
display and the tab folds back in by itself; plug the same display back and it folds out there
again. One tab per secondary display, one window per display, behind the feature flag
`multi_monitor`, off by default.

## 1. Model

### Decided: each display's window shows one tab

The model already holds a list of tabs, each of one space, each with its own tree (`Tab::layout`,
`Tab::focused`; the active tab's live copies are `Model::layout` and `Model::focused`). A display
other than the main one shows **one of those tabs**. Nothing new is merged or split: a tab is
either on show in the main window, out on a display, or not on show.

Core (`ottospace-core`, a new `model/displays.rs`):

- `Display { uid: DisplayUid, frame: Rect, visible: Rect, scale: f32 }`, passed in by the shell
  (`Action::SetDisplays { displays, windows }`; the core refuses it from any origin but
  `System`). `windows` is each ottospace window's content frame and display. Frames are global,
  top-left origin, the AX space (`display::bounds`). The core does no I/O: the shell reads the list
  with `ottospace_mac::display` (GPUI's `cx.displays()` panics when `CGGetActiveDisplayList` fails,
  as it can mid-reconfiguration, and its `bounds()` has origin 0,0), and asks GPUI only for
  `uuid()`. It sends `SetDisplays` when the display list changes and when an ottospace window
  moves or resizes (`observe_window_bounds` per window): a drag of the main window onto the display
  of a tab out is seen without a reconfiguration.
- `Model::out: Vec<Out { tab: u32, display: DisplayUid }>`: the tabs folded out, at most one per
  display, never the main window's display.
- `Tab::display: Option<Affinity { uid, place, folded: u64 }>`: the display a tab belongs on (its
  **affinity**). Set by a fold out, cleared by the user's fold in, kept by an unplug, with `folded`
  a sequence number of that fold in (which tab returns first) and `place` the display's place in
  the arrangement (to tell identical monitors apart). That is how a tab comes back.
- `Model::main_tab`: the tab on show in the main window. `Model::active` keeps its meaning to all
  existing code, **the tab with the keyboard**: `main_tab` or one of `out`. The keyboard moving to
  another window is a tab switch the way `SelectTab` is one (stash the live tree, take the other),
  so every action that works on "the focused pane" works on display 2 with no change. The shell
  draws a window that does not have the keyboard from its tab's stashed tree.
- **`space()` splits in two.** Today it is `tabs[active].space` (`spaces.rs`), read in about 120
  places. It becomes two accessors, and every caller is sorted into one:
  - `space()`, **the keyboard's space**: `NewTab`, `CycleTab`, `visible_tab` (so ⌘1–9 and ⌘⇧⌥1–9
    count the tab row of the window being typed in), the chat's `joins`.
  - `shown_space()`, **the main window's**: the top row's selection, the drawer and the agent chip,
    `home()` (Home's screen only when the *main* tab is Home's), `HomeStep::on_show`, and
    `regroup`'s `last_spaces` and `Space::last` (written from `main_tab`, never from a tab out, so
    a secondary window never becomes "the workspace selected last" that `settle_home` fills).

  This split is the core slice's main risk; a property rule checks that the selection history
  never names a tab out.
- **Which tab has the keyboard follows the key window.** Each window's key and pointer handlers
  first send `KeyWindow { tab }` (the user's) when the window they fire in does not hold `active`,
  so the user always types into the window they look at. A client's `focus` of a pane in another
  window's tab sets that tab's own focused pane but does not move `active`: a client never moves
  the keyboard between windows.

Rules (property-tested after every step, from every origin):

1. A tab is on show in at most one window. `main_tab` is in no `out`; `out` names distinct tabs
   and distinct displays, all present in the last `SetDisplays`, none the main window's.
2. `active` is `main_tab` or a tab of `out`.
3. Home's tab is never out (it holds no pane; `settle_home` stays as is).
4. A tab out is still in `tabs`, in its space, at its place: fold in moves nothing.
5. With `multi_monitor` off, `out` is empty.
6. `last_spaces` and every `Space::last` name no tab out.
7. A tab closed (by the user, by `settle_home`'s stray-tab rule, a task dismissed) leaves `out`:
   its window closes.

### Rejected

- **A per-display part of one layout tree** (a top-level `Node` per display, a tab that spans
  displays). It was the first lean. It makes every tab carry a display dimension, splits and drops
  must know which part they are in, and an unplug has to merge one tree into another: where, at
  what ratio, and how to split it apart again on replug. With a tab per display the unplug is "the
  tab is no longer out". The owner's read holds: simpler and it fits the existing model.
- **A workspace per monitor.** Spaces are the top row's unit; a second set of spaces means a
  second top row, second Home, second "selected last" history, and tasks that cannot use both
  displays (agent terminal on one, browser on the other is the common case). A tab per display
  gives that without a new kind of thing.
- **One giant window spanning displays.** With "Displays have separate Spaces" on (the default)
  a window is drawn only on the display holding most of it
  ([`NSScreen.screensHaveSeparateSpaces`](https://developer.apple.com/documentation/appkit/nsscreen/screenshaveseparatespaces)
  tells which). Off, it spans, but displays of different scale or arrangement (offset, gaps,
  different heights) leave dead areas, the window crosses the bezel, and fill (§4) cannot cover
  two displays. No.

### What each window shows

| | Main window | A secondary window |
|---|---|---|
| Top row | as today: Home, the spaces, the tab row of the selected space. A tab that is out carries a small display mark; a click on it moves the keyboard to its window | **the tab row of its tab's space only**: the space's glyph, its tabs, its tab highlighted, others that are out marked. No Home, no spaces, no + for spaces. A ⤓ button at the row's end folds it in |
| Left rail, sections, list mode | yes | no |
| Drawer, agent chip | yes, for the selected space (main's) | no |
| Notice bar, palette, ⌘/ | yes | in the window with the keyboard (one at a time) |
| An empty tab | the key caps | the key caps |

The **main window** is the window ottospace started with. The user picks its display by putting it
there (drag, or ⌃⌘F there). No separate "main display" setting: nothing else to keep in step.

### Which tab where, as the user switches

- **⌘⌃1–9, a space click, ⌘⌃] [**: change the main window only. A tab out stays where it is when
  the main window changes workspace (it is pinned to its display). If the keyboard was on a
  secondary window it moves to the main window: you just picked something shown there. The main
  window shows the space's last tab (`Space::last`, never a tab out, rule 6), else its first tab
  not out. **Every tab of that space is out**: the selection does not change and the keyboard goes
  to the window showing the space's last tab.
- **⌘1–9, ⌘} ⌘{, a tab click** in a window's tab row: that window shows that tab. If the tab is on
  show in another window, the keyboard goes there instead (rule 1): nothing is moved by a tab
  switch. In a secondary window, the tab it showed folds in (its affinity cleared) and the new one
  is out on that display.
- **⌘T** in a secondary window: the new tab opens in that window (the old one folds in, as above).
  **⌘D, ⌘E, ⌘W** act on the focused pane, wherever it is. Closing a secondary window's last pane
  leaves its tab empty there, as in the main window.
- **⌘⇧W** in a secondary window closes the tab, and the window with it; the keyboard goes to the
  main window, whose tab does not change (`CloseTab`'s "its neighbour takes its place" applies to
  the main window's tab only).
- **The red button** of a secondary window folds it in (it closes no tab). The main window's stays
  as today (quit, asking). Today `on_window_closed` quits on *any* window closing and
  `on_window_should_close` is the main window's only (`main.rs`): slice 2 limits the quit to the
  main window and gives each secondary window its own should-close handler.
- A **task gone**, a **workspace deleted**: its tabs go where they go today; a tab that was out
  stays out on its display, unless it is closed (rule 7).

Rejected: **a secondary window follows the workspace** (each space remembers a tab per display, and
switching workspace changes every display). The second display then flashes on every switch, an
empty space leaves it blank, and "pinned to the display" is what the owner described ("another
tab open in the other monitor"). Can be added later as a per-tab choice; open question 2.

## 2. Navigation

### Fold out, fold in: ⌘⇧O

The keymap (`main.rs::keymap`) has nothing on ⌘⇧O or ⌘⇧⌥O, and `key_down` takes only digits
with ⌘⇧⌥ (⌘⇧D, suggested first, is Split Down).
macOS reserves no ⌘⇧O system-wide; in Finder it is Go ▸ Documents, but that is Finder's own menu and
only while Finder is in front. Menu: View ▸ Fold Tab Out / Fold Tab In; the palette: same names,
and "Move Tab to Display…" which lists the displays by where they are ("Left of this one",
"Above"), drawn on screen only. A tab's right-click menu gets the same items.

| Where the keyboard is | One display | Two | Three or more |
|---|---|---|---|
| Main window | a notice: "No other display" (`Notice`, nothing else) | out to the other | out to the next display, in arrangement order (left to right, then top to bottom), starting after the main window's, that shows no ottospace tab. If every one shows one, the next one after the main window's, whose tab folds in (a swap) |
| A secondary window | | folds in | folds in |

- Home on show (the main window shows Home's tab): ⌘⇧O does nothing (a notice: "Home stays here").
- Folded out, the keyboard follows the tab to its window. The main window shows the tab its space
  would show if this one closed (the rule of `CloseTab`); when it was its space's only tab, the
  space selected before it (`last_spaces`), Home if none. Folded in, the tab is not put on show;
  the keyboard goes back to the main window.
- **Only the user** folds out or in: `fold::USERS_ALONE`, the core refuses `TabOut`/`TabIn` from
  `Socket` and `System` (`Model::refusal`), as `FillDisplay` is refused today. The one exception is
  `System` folding in for a display gone and out for a display back (§5): that is a separate
  action, `SetDisplays`, which only the shell's display watcher sends.

### The focus arrows cross displays by geometry

`Node::neighbor` finds the pane touching the focused one in a unit square. That stays. When it
finds none, today the zones take over (`model/zones.rs`: left of the panes the section list, then
the rail; right the drawer; the user's alone). Today they always take ← and → at a pane edge (→
even with no drawer to enter), the rail's ← and the drawer's → do nothing, they read
`self.layout`, and `back_to_pane` returns to `active`'s pane. So the zones change too:

- They run only while the keyboard is on the main tab, and `back_to_pane` returns to the main
  tab's pane.
- → at the main panes' edge with no drawer open, the drawer's →, the rail's ←, and ← at the
  main panes' edge with the rail and list closed, go to the next step, instead of stopping.
- In a secondary window there are no zones: its edges go straight to the next step.

The next step, by global geometry, decided in the core:

1. Each window's panes get global rects: the window's content frame (from `SetDisplays`) times
   the tab's unit rects. Chrome insets are ignored: this chooses a neighbour, it does not draw.
2. The display on the `dir` side of the focused pane's display: the one whose facing edge touches
   it with an overlapping span across (macOS arranges displays edge to edge). None: stop, as now.
3. In that display's window, the pane touching the window's facing edge whose span overlaps the
   focused pane's span most; none overlapping (different heights), the nearest by centre.
4. The main window's zones are on the way, in order on the screen: from display 2 on the right,
   ← reaches the drawer (if open) before the main window's panes; from the main window, ← from
   the leftmost pane goes list, rail, then display 2 if it is on the left.

A pure function, `layout::across(from: Rect, dir, candidates: &[(PaneId, Rect)])`, with unit tests
for side by side, stacked, offset, different heights, negative origins and three displays.

Origins: a client's `FocusDir` stops at its window's edge, as it stops at the panes' edge today,
and a client's `focus` never moves the keyboard between windows (§1). The shell makes a window key
only for the user's moves.

### Moving panes across

- **Swap** (⌘⇧⌥ arrows): at the edge it swaps with the pane across, by the same rule.
  `swap_panes` already swaps leaves in every tab's tree. The focus stays on the pane, now on the
  other display.
- **Move to a tab** (⌘⇧⌥1–9): unchanged; moving into the tab that is out moves the pane to that
  display.
- **⌘⇧⌥O, Move Pane to Other Display**: the focused pane goes into the tab out on the next display
  (beside the pane nearest the facing edge); with none out there, a new tab of its space with that
  pane alone, folded out there. The user's alone, like ⌘⇧O. A later slice.
- Dragging a pane header from one window to another: GPUI drags do not cross windows. Not in
  these slices.

### Key window, and an app the user types in

One app, one key window. The focus arrows are ottospace's keys, so they only fire while ottospace
is in front; moving the keyboard to another ottospace window is `makeKeyWindow` on it, which takes
nothing from another app. An app pane stays live until the user leaves it, as today: the return
key (read by `hotkey::take_press` on the tick) does `Leave` and makes key **the window holding the pane that was live**,
not the main one. `observe_window_activation` runs per window; the 400 ms click guard is shared.
`live` stays one pane across all windows: entering a pane on display 2 parks the live one on the
main window (already the rule).

## 3. Hosted windows on display 2

- **Live**: placed over its pane in its own window. `on_screen` already adds the window's display
  origin to window-local bounds (GPUI's window bounds are relative to its display); it must take
  the pane's own `Window`, not the main one. `top_limit` likewise (each display has its own menu
  bar with separate Spaces on). The 500 ms drift check runs per window.
- **Parked**: **on the free corner of the display the pane is on**, falling back to the global
  spot (`display::parking_spot`). Reasons: the same Space (with separate Spaces on, a window
  parked on display A's Space disappears from a Space change there, not B's), the same scale for
  the stream, and a short move on enter. The decision (which corner) moves into the core
  (`parking(displays, display) -> Point`), unit-tested, with `Effect::Repark(pane)` when the answer
  changes (a plug, an unplug, a fold). Measure first: the brainstorm's experiments 1, 2, 4, 10.
  Until measured, the slices below keep the one global spot.
- **ScreenCaptureKit**: a window filter names no display, so the stream does not care where the
  window is parked. The stream's scale is the pane's window's `scale_factor()` (today the one
  window's): a fold to a display of another scale reconfigures the stream. `Hosted::new`'s assumed
  2.0 for the corner radius is recomputed then.
- **Coordinates**: AX is global, origin the primary display's top left, y down; AppKit is global,
  origin its bottom left, y up. `window::flipped` and `recovery::screens` already flip about the
  first `NSScreen`'s height, which is right for every display, including those left of or above
  the primary (negative x or y). The brainstorm's experiment 8 checks a live window on such a
  display before slice 4 ships.

## 4. Fill the whole display, on each display

- **Fill is per window.** ⌃⌘F, the green button and the top row's expand button act on the window
  they are in: its frame becomes its display's, as `fill::frame` makes it today (below a camera
  housing's strip). `layout.json`'s `fill` becomes
  per window (`fill` for the main one, `Out::fill` for the others).
- **The bars are app-wide.** `NSApplication.presentationOptions` belongs to the app, not a window:
  `autoHideMenuBar` ("The menu bar is normally hidden, but automatically appears when moused
  near.") and `autoHideDock` ([Apple](https://developer.apple.com/documentation/appkit/nsapplication/presentationoptions-swift.struct)).
  They are set while **any** ottospace window fills, and count only while ottospace is in front, as
  today. The docs do not say whether `autoHideMenuBar` hides the menu bar of every display or only
  the active one's: by hand, check 3 below.
- **"Displays have separate Spaces" on** (the default): every display has a menu bar. A window that
  fills its display under a menu bar that does not hide keeps the bar over its top row (as an app
  pane does today on the main display). If check 3 shows the bar stays on a display, a filling
  secondary window takes that display's `visibleFrame` top instead: no part under a bar that never
  goes.
- **Off**: one menu bar, on the primary display; secondary displays have none, so a secondary
  window's fill is its whole display, bars or not. The Dock is on one display either way (it
  follows the pointer to the bottom edge of another with separate Spaces on).
- Native full screen stays off for every ottospace window (`zoom_instead_of_fullscreen`): it would
  give the window a Space of its own, where hosted windows cannot follow.
- `fill_holds` runs per window: a secondary window dragged off its display ends its fill.

## 5. Plugging and unplugging

### Display identity

`CGDirectDisplayID` is not stable: it changes across reboots and reconnects. GPUI 0.2.2's
`PlatformDisplay::uuid()` (`platform/mac/display.rs`) wraps
[`CGDisplayCreateUUIDFromDisplayID`](https://developer.apple.com/documentation/colorsync/1500384-cgdisplaycreateuuidfromdisplayid),
a UUID for the display that, unlike the id, is meant to stay the same for the same monitor
across reboots and reconnects (built from its vendor, model and serial: not verified here).
`DisplayUid` is that UUID. Two identical monitors with no serial can share or swap one; then the
core tells them apart by the `place` kept in the affinity (the one on the left keeps the first). A
`uuid()` error: the display gets no tab out (⌘⇧O's notice says so).

### What happens

| Event | Tabs out | Main window |
|---|---|---|
| A display with a tab out goes | folds in: `out` loses it, `Tab::display` kept. The window closes. The keyboard, if it was there, goes to the main window. The main window does not switch to it | stays; macOS moves it if its display went |
| The main window's display goes | macOS moves the main window to another display. A tab out on that display folds in (rule: never out on the main window's display) | as macOS puts it; fill ends (`fill_holds`), as today |
| A display comes back | if it is not the main window's display and shows no tab: of the tabs whose affinity is it, not on show in the main window and without the keyboard, the one folded in last folds out there. The others keep their affinity (⌘⇧O sends one). Affinity to the main window's display waits until the main window leaves it | unchanged |
| Lid closed (clamshell) | the built-in display leaves the list: as "a display goes" | as above |
| Lid opened | as "a display comes back" | |
| Resolution or scale change, rearrangement | no fold. The window keeps its display: the shell sizes it to the new frame (and keeps fill), re-reports `SetDisplays`, the core re-picks parking and reconfigures streams | the same |
| An ottospace window moved to another display (the user's drag, or macOS during a reconfiguration: the shell cannot tell them apart, so both are `System`, by the same rule) | a secondary window: if that display has no ottospace window, its tab's affinity becomes that display; else it folds in. The main window onto a secondary's display: that one folds in, affinity kept | as moved |
| Sleep and wake | displays may leave and come back: fold in, fold back out, by the rules above. No flapping: the shell sends `SetDisplays` only once the list has held for 1 s | |

Folding in has nothing to place: the tab is already in its space's row, at its place (rule 4). The
order is the row's order.

### The freeze in #207 must not repeat

#207 (open, not merged; these slices stack on it): after an unplug the window stopped drawing, and
the quit question with it. Its likeliest cause, not proven: GPUI's per-window `CVDisplayLink` was
built for a display that was gone, and never fired. Its fix (a CG
reconfiguration callback that only sets a flag, kicks from the 16 ms tick, `windowDidChangeScreen:`
and `setNeedsDisplay`) is for one window. With several:

- Every ottospace window has its own link: the stall watchdog kicks **every** open window, and
  counts stalls per window (`counts.redraws` per window in the state).
- **Closing is at once, opening waits.** A display with a tab out that leaves the list (read on
  the tick after the callback's flag) has its window closed then, from a task of its own (not
  inside a GPUI update, as fill sets frames today): a window that macOS would move mid-change, with
  a link for a display that is gone, is not left drawing. A window is **opened** only from the
  settled list (1 s with no callback), so no link is built for a display still in flux. The main
  window is never closed; it is the one kicked.
- A `find_display` that fails, or a display whose link cannot be built, leaves the tab folded in:
  never a window that does not draw.
- Quit must be reachable from the main window at all times: the quit question is drawn there only.

### `layout.json`

```json
"out": [{ "tab": 3, "display": "6F1D…-UUID", "fill": false }],
"tabs": [{ "…": "…", "display": "6F1D…-UUID" }]
```

- `selected` stays the tab on show in the **main** window (`main_tab`), not `active`: quit with
  the keyboard on display 2 and the next start does not show that tab twice.
- `out`: tabs out at quit, by place in `tabs`. Restored once the main window exists and the first
  settled display list is in (`RestoreLayout`, as for `fill`): displays present get their window;
  the others' tabs stay folded with their affinity.
- `tabs[].display`: the affinity, kept even while folded, so a display plugged in a week later
  still gets its tab.
- With `multi_monitor` off both are read and kept unchanged (`Extra`) but not acted on.
- The display's frame is not kept: the window takes the display's visible frame (or fill).

## 6. Opt-in

`Feature::MultiMonitor`, `multi_monitor`, Experimental (off unless `settings.json` turns it on).
Off: no ⌘⇧O, no menu or palette item, no tab mark, no second window, `out` stays empty, the shell
still reports displays (parking needs them) but nothing folds. On with one display: ⌘⇧O shows
"No other display"; nothing else is new. `Action::feature` names it for `TabOut`, `TabIn`,
`PaneOut`; `SetDisplays` has no feature (parking uses it).

## 7. Socket and state

State (every reply):

```json
"displays": [{ "id": 0, "frame": [0,0,1728,1117], "visible": [0,38,1728,1079], "scale": 2.0, "main": true }],
"windows": [{ "display": 0, "tab": 1, "main": true, "key": true }, { "display": 1, "tab": 4, "main": false, "key": false }]
```

- `id` is the display's place in the list, for this run only. No UUIDs (a hardware fingerprint),
  no names (`localizedName` names the user's hardware).
- `tabs[].out`: the display id it is on, or null.
- Pane `bounds` stay window-local, and each pane names its window (`window` index).

Requests:

- **No agent-facing request folds or moves a tab to a display.** Moving the user's layout between
  screens is the user's decision, like fill: an agent that wants a tab on the other display asks
  the user. The core refuses `TabOut`/`TabIn` from `Socket`; a client's `key` of ⌘⇧O is the
  client's and is refused.
- Agents may read `displays` and `windows`, `focus` any pane (within its tab; it never moves the
  keyboard between windows, §1), and
  `move_pane` into a tab that is out, which they can already do to any tab.
- **Test-only** (`control::test_refusal`, documented in `CLAUDE.md` and on `Request`, never in
  `skill/guide.md`): `displays_set` (`"displays": [{frame, visible, scale, uid}]`) replaces the
  shell's display list with made-up ones and sends `SetDisplays` as the watcher would, including
  unplug and replug by leaving one out and putting it back; and ⌘⇧O through `key` with
  `"as":"user"`. `display_change` (#207) stays for the kicks. Under `OTTOSPACE_E2E` a made-up
  display has no `DisplayId`: the shell opens its window with none (GPUI then puts it on the
  primary display), at the test's sliver, and reports the made-up display's visible frame as the
  window's frame to the core. The `find_display` rule (§5) applies outside a test run only.

## 8. Tests

- **Core unit**: `layout::across` for every arrangement above; fold out, fold in, a swap with three
  displays; unplug folds and replug returns (affinity kept, cleared by the user's fold in, not
  returned while on show in main); the selected space follows `main_tab` while the keyboard is on
  display 2; ⌘1–9 in a secondary window; refusals by origin.
- **Property** (`model/tests/prop.rs`): makers for `TabOut`, `TabIn`, `SetDisplays` (random lists
  of 1–3 made-up displays, displays dropped and returned); `check` gains rules 1–7 of §1. A new
  property: after any sequence, `SetDisplays` with the full list again, and every present display
  other than the main window's that has at least one tab with its affinity (not on show in main,
  without the keyboard) shows one of them. Also: the selection history never names a tab out
  (rule 6), and keys in a window always go to its tab (`KeyWindow`).
- **e2e** (`cargo e2e`, `MULTI_MONITOR` feature set): `displays_set` with two made-up displays
  side by side, of different heights. The second window, like the first, sits at a free corner as
  a sliver (offset so both show and draw); the core's geometry is the made-up frames', so the
  span-overlap rule and the different-heights fallback are both tested. Its pane bounds are read
  from the state as for the first. Tests: ⌘⇧O opens the second window with the tab and the main row marks
  it; ⌘⌃→ from the rightmost pane focuses display 2's nearest pane and `key` is the second window;
  ⌘⌃← comes back; leaving the second display out of `displays_set` folds it in and closes the
  window; putting it back folds it out again; `display_change` kicks both windows and both draw on
  (#207); a client's ⌘⇧O is refused; restart with `OTTOSPACE_HOME` restores `out`.
- **e2e-apps** (CI's `app-windows.yml`, not locally: it takes the keyboard): two made-up displays
  side by side on the runner's one real display, so the second ottospace window is really on
  screen. The canary in a pane of the tab out: parks, goes live over the right window, the return
  key makes key the second window, unplug (made-up) parks it back with no window left in view.
- **What CI cannot test**: real display ids and UUIDs; a real unplug and the GPUI display link
  (the #207 failure itself); mixed scale; separate Spaces and per-display menu bars; fill on a
  second display; the lid; sleep and wake. The made-up list proves the model and the shell's
  reaction, not macOS's.

### By hand (the owner, two displays, before the flag leaves Experimental)

1. ⌘⇧O a tab with a terminal and an app pane. ⌘⌃→ ⌘⌃← across; type in each; enter the app pane on
   display 2 and use the return key: the right window is key.
2. ⌃⌘F on each window, separately and both. With separate Spaces on: does `autoHideMenuBar` hide
   the second display's bar, and does clicking an app on one display bring the bars back on both?
   Repeat with it off (needs a log out).
3. Unplug the display with the tab out: the tab is in the main row, the window drew all along, ⌘Q
   works (#207). Plug it back: the tab returns. Same with the lid closed and opened.
4. Change the second display's resolution and scale: nothing folds, the app pane's picture is
   sharp after the change.
5. A display arranged left of and above the primary (negative origin): a live app pane lands over
   its pane (the brainstorm's experiment 8).
6. Two identical monitors: swap their cables; do tabs return to the right one?
7. Sleep with a tab out, wake: it is where it was.

## 9. Slices

Each a stacked PR, shippable alone, behind `multi_monitor` from slice 3 on. They stack on #207.

1. **Displays in the core, no change on screen.** `Display`, `DisplayUid` from GPUI's `uuid()`,
   `SetDisplays` from #207's callback (gone at once, new after 1 s settled) and window bounds,
   `displays` in the state and the SDK, `displays_set` (test-only). Parking still the global spot,
   now chosen in the core. No flag needed: nothing new is visible.
2. **The core of tabs out, one window still.** `out`, `main_tab`, `KeyWindow`, the `space()` /
   `shown_space()` split, `TabOut`/`TabIn` with refusals, rules 1–7 in the property test. The
   shell draws nothing new (no window opens); every existing test passes.
3. **⌘⇧O folds a tab out and in** (the first user-visible slice). `Feature::MultiMonitor`. The
   shell becomes one view per window: `Workspace` stops being the one window's root, actions and
   `key_down` take the window they fire in, a reply waits for the frame of **every** window that
   draws its request (`drawn_seq`), effects name their window. Quit only on the main window's
   close; each secondary window's red button folds in. Secondary tab row and ⤓; the notice with
   one display; the main row's mark; `windows` in the state; per-window stall kicks (no second
   window ships without them); a display gone folds in at once (the table's first row; the rest
   is slice 4). App panes in a tab out go live over their own window: `on_screen`, `top_limit`
   and the drift check take the pane's window. ⌘⇧O in the keymap, `key_use` bindings, ⌘/ rows,
   the cheat sheet and `docs/reference.md`. Focus moves between windows only by a click and by
   ⌘1–9 here.
4. **Replug folds out; `layout.json`.** The rest of the table in §5, `out` and `tabs[].display` in
   the file, `selected` as `main_tab`, restore after the first settled list.
5. **Focus arrows and swaps across displays.** `layout::across`, the zones' changes (§2), client
   stop at the window's edge, the return key to the right window.
6. **Hosted apps, the rest**: stream scale per window, e2e-apps on CI. Then parking on the pane's
   display, once the brainstorm's experiments 1, 2, 4 and 10 are run by hand.
7. **Fill per window**, after check 2 by hand.
8. **⌘⇧⌥O, a pane to the other display**, and "Move Tab to Display…" for three or more.

## Open questions for the owner

Each has a default the slices assume.

1. **Key**: ⌘⇧O for fold out and in? Default: yes; ⌘⇧⌥O for a pane.
2. **Workspace switch**: does the tab out stay when the main window switches workspace? Default:
   it stays (pinned to its display). Following the workspace could be a per-tab choice later.
3. **The drawer**: main window only, for the main window's space, even while you type on display
   2? Default: yes; ⌘L takes the keyboard to the main window's drawer.
4. **Crossing the rail**: should the focus arrows skip the section list and the rail when crossing
   from the main window to a display on its left? Default: no, they go in screen order (list,
   rail, display 2); the rail is one press if closed (it is then one cell).
5. **Several tabs per secondary display** (its own tab row to switch between them, not only the
   space's)? Default: no, one tab out per display; ⌘1–9 there swaps which.
6. **Replug while the tab is on show in the main window**: leave it there, or send it out anyway?
   Default: leave it; ⌘⇧O sends it.
7. **Separate Spaces**: do you run with it on? Default: designed for on (macOS's default), checked
   by hand for off.
8. **Main window by display**: is "the main window is the one you started with, wherever you put
   it" enough, or do you want a setting naming the main display? Default: no setting.
