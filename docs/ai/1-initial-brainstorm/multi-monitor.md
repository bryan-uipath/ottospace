# Multi-monitor

Recommendation: one ottospace window per display, one shared model; a task spans displays and each of its tabs is assigned to a display. Reach it in steps.
Start with display-aware parking in the core, which fixes gaps that exist today with one window. Do not take ownership of all windows.

**Status: parked.** Bryan, 2026-11-12: multi-window is not important for now. He will use ottospace on the main
display and keep the second display for things ottospace does not control. One part still matters in that setup:
`parking_spot()` ignores where ottospace is, so parked windows can sit at a corner of the second display (see the
first PR below).

Refines #4 in `docs/ideas/1-initial-brainstorm.md` ("second- or third-screen layouts", "display affinity") and Bryan's
request of 2026-11-11. Not agreed. Nothing here was measured: no multi-display run was made for this note.

## What the code does today

| Fact | Where | Consequence with 2+ displays |
|---|---|---|
| One GPUI window. Closing it quits. `Workspace` holds the `Model`, the hosted windows and the 60 Hz tick | `main.rs` (`open_window`, `on_window_closed`, `Workspace::new`) | A second window needs the model and `hosted` lifted out of the view |
| `Model` has one `active` tab, one `layout`, one `focused`, one `live` | `model.rs` | "What each display shows" has no place in the model yet |
| A task points at one tab (`Task.tab`). Tabs are a flat list. "Task owns its tabs" is designed, not built | `tasks.rs`, `plan.md` | Display mapping should be designed together with task → tabs |
| `parking_spot()` takes the first display (system order) with no display reaching into the area right of and below its bottom-right corner. It ignores where ottospace is | `display.rs` | Windows can park on a different display from their pane |
| Every parked window goes to the same point | `Hosted::park` | They overlap each other. `hosting-windows.md` lists overlap as a throttle risk. Not measured with 2+ windows |
| A parked window is re-parked only when its pane size or crop changes. Only the live window gets the 500 ms drift check | `sync_layout` | Nothing re-parks them on a display change. They move only if their pane's size or crop changes, or when a pane stops being live |
| Stream scale is the ottospace window's `scale_factor()`. `Hosted::new` assumes 2.0 for the first stream config (replaced by the first `park`) and for the corner radius (never recomputed) | `park`, `sync_layout`, `Hosted::new` | Wrong when the parking display and the pane's display differ in scale. A scale-only change does not reconfigure the stream |
| Streams start at host time and stay active while the window is hosted, also for background tabs | `Hosted::new` | Showing two tabs at once adds no streams. Capture cost can still change with pane size and scale |
| `on_screen` adds the window's display origin to the pane bounds | `main.rs` | The one window should already work on a second display. Not verified |
| Return key: `Action::Leave`, then `cx.activate(true)`. One Carbon hot key, id 1 | `hotkey.rs`, `return_to_ottospace` | With two windows: which one gets the keys is undefined |
| Native fullscreen is disabled for ottospace's window | `window.rs` | Keep this for every ottospace window |
| Recovery file stores original frames in global coordinates | `recovery.rs` | An original frame can point at a display that is gone. macOS may clamp it. `set_position` can read back where the window landed, but recovery ignores that |

A free bottom-right corner always exists: the display with the lowest bottom edge (the rightmost one on a tie) has
nothing right of or below it. So "every display is in use" does not remove the parking spot. Parking needs a corner,
not a spare display. (Derived from the geometry in `parking_spot`, not measured.)

## Options

### A. A window per display, or one window that moves

| | A1. One window, sent to another display | A2. One window per display |
|---|---|---|
| User sees | The same window, on the other display | Each display has an ottospace window with its own tab bar and panes. One rail, on the primary window; the others have only a top bar |
| Keys (proposals) | ⌘⌃←/→: move the window to the display on that side | ⌘⌃←/→: focus the window on that display. ⌘⌃⇧←/→: send the focused tab there. ⌘⌥ arrows and the planned pane-move key (`app-windows.md`) continue across the display edge |
| Core | `Action::SetDisplays { displays }` (id, rect, scale, passed in by the shell). `Action::MoveWindow { dir }` → `Effect::MoveWindow(display)` | A `View` per display: `{ display, active tab, layout, focused }` replaces `Model::active / layout / focused`. `live` stays single: there is one keyboard. `Action::FocusDisplay`, `SendTab { display }`, `SendPane { display }` |
| Shell | Move the window, then re-park (parking follows the window) | Model and `hosted` move from the `Workspace` view to a shared entity. One view per window. The tick runs once, not per window. Quit when the last window closes. `on_screen` and `top_limit` take the pane's own window |
| Hosting | Parking must follow the window, or scale and Spaces differ between pane and window | Same, per window. A pane's window prefers the display that shows the pane; with no free corner there it falls back to another display, and the scale and Spaces risks return |
| Cost | Small | Large: the shell refactor is most of it. The core change is medium and unit-testable |

In `APP` mode ottospace keys do not fire (accepted constraint in `plan.md`). So "send this app to the other display"
needs the return key first, or a second global hot key. `RegisterEventHotKey` can register more than one; the handler
must then read the hot key's id (today it sets one flag for any key).

### B. An indicator on every display

| Variant | User sees | Notes |
|---|---|---|
| B1. Thin bar window on displays that have no ottospace window | Task name, tab names, "needs you" count. Click or key brings ottospace to that display | macOS has no public way to reserve a screen edge. Other apps' windows go under or over the bar. It must float above them, and it then covers their content |
| B2. Menu bar item | Otto icon with the "needs you" count. Menu: tasks and tabs; choosing one brings it forward | Costs no screen space. With "Displays have separate Spaces" on, each display has a menu bar. With it off, only the primary has one. GPUI support for a status item: not verified; may need `objc2` code in `ottospace-mac` |

- Core: none beyond what the bar reads. It is a second view of `tabs`, `tasks.needs_you()` and, later, the views. One action: `Summon { display }` (A1's move).
- Hosting: nothing changes. The bar hosts no windows.
- B alone adds no surfaces on the other display. It is a way in, and a status display. It is useful with A1 (one window, summoned to where you look) and unneeded with A2 (each display has a top bar already).
- Cost: B2 small, B1 small to medium (a non-activating floating window on each display, on every Space).

### C. Own every window on every display

What macOS allows without SIP changes, and what the tools do. From memory of their docs; not re-checked for this note.
The yabai and Amethyst rows depend on the tool's and macOS's version: treat their "cannot" lists as unverified.

| | Uses | Can do | Cannot do |
|---|---|---|---|
| Public API (AX, CGWindowList, NSWorkspace) | Accessibility grant | List, move, resize, raise, minimise other apps' windows. Watch windows open and close | Reparent a window. Move a window to another Space. Set z-order below or between other apps' windows. Stop an app from placing its own window. Hide a window without minimising or hiding the app |
| yabai | AX. Optional scripting addition injected into Dock.app, which needs SIP partly disabled | Tiling by AX works with SIP on | Without the addition: create, remove or reorder Spaces, move windows between Spaces, instant Space switch, opacity, layers, sticky windows |
| Amethyst | AX, plus private read-only calls for Spaces | Tiles each display's current Space. SIP stays on | Does not own Spaces. Moving a window to another Space is a synthetic drag and has been unreliable |
| AeroSpace | AX, plus `_AXUIElementGetWindow` (the same private call ottospace uses) | Its own workspaces: windows of hidden workspaces sit at a screen corner, mostly off-screen. One workspace per monitor. SIP stays on | Does not use macOS Spaces. Its guide asks for a monitor arrangement that leaves corners free, and discusses "Displays have separate Spaces" |

ottospace already is the AeroSpace model plus capture. Option C would mean: adopt every standard window at launch,
one ottospace window per display, everything parked or live.

- User sees: no free-floating windows. Every app is a pane somewhere.
- Core: A2's model, plus rules for where a new window goes (the window rules of `scripting.md`).
- What breaks: one stream per window (about 95 MiB each at 4K, `hosting-windows.md`); per-app throttling is measured for two apps only; dialogs, sheets, popovers and non-standard windows escape panes; windows with a minimum size do not fit; a crash strands every window, not a few (the recovery file helps); z-order still cannot be fixed without SkyLight.
- A tiling manager keeps every visible window real. ottospace keeps one real and streams the rest, because its own drawing (headers, borders, the share curtain, sneak hints) must sit above pane content. Making all visible panes live would drop that, and clicking ottospace would bury them.
- Cost: A2 first, then large and open-ended. The per-app failures are the cost.

### D. Tabs with a display affinity (A2, with the mapping decided)

This is A2 plus an answer to "how do tasks map to displays". It matches #4a.

- A tab has an optional `display` (a stable display UUID, not the session's display id).
- A task spans displays. Switching task changes what every display shows: each window shows the task's tabs assigned to it.
- A tab with no affinity opens in the focused window.
- A pinned tab (for example Slack, or a DRI dashboard on the side monitor) stays on its display when the task changes. Optional; see open questions.
- Display removed: its tabs become tabs of a remaining window and keep their affinity. Display back: they return. This is a pure function of the display list, so it lives in the core with unit tests.

Rejected mapping: one task per display. "Pop next" then has to pick a display, and a task cannot use two displays, which is the common case (agent terminal on one, browser on the other).

## Hosting: what gets hard

| Topic | With 2+ displays | State |
|---|---|---|
| Where parked windows go | On the display that shows their pane: same scale, same Space, short move on enter. Use that display's free corner. If it has none, the nearest display that has one | Bottom-left corners (window's top-right pixel on screen) would double the spots. Whether macOS allows that position: not verified |
| Overlap at the spot | All parked windows share one point today | Throttling with several windows at one spot: not verified. If it happens, spread them over corners or offset them |
| Stream from another display | An SCK window filter does not name a display, so frames should keep coming | Not verified |
| Display sleep | If every display sleeps, nobody sees the panes. One display off, or the lid closed, may change the display list or only the power state (not verified) | Stream behaviour across sleep and wake: not verified |
| Mixed scale | A window parked on a 1x display probably renders at 1x. The stream is configured at the ottospace window's scale. A 1x source in a 2x pane is blurred | Not verified. Parking on the pane's display avoids it. `SetDisplays` must carry scale, and a scale change must reconfigure the stream |
| Enter across displays | The window moves from the parking display to the pane's display. The app may re-render for the new scale | Delay and flicker: not verified |
| "Displays have separate Spaces" on (default) | A window belongs to one display's current Space. If the user changes Space on the parking display, parked windows stay behind | Whether streams continue from a hidden Space, and where an AX move across displays lands: not verified. Already a risk with one display |
| Same setting off | One Space covers all displays. Simpler for us. One menu bar. Native fullscreen on one display may blank the others (from memory; not verified) | AeroSpace's guide prefers it off (from memory) |
| Fullscreen apps | A native-fullscreen app takes its display's Space. The ottospace window and windows parked there are on another Space | Not verified. A hosted window that goes fullscreen while live is not handled today either |
| Display unplugged | macOS moves the windows that were on it. Parked windows may land in full view. The live window is fixed by the drift check once its pane has new bounds | Where macOS puts sliver-parked windows, and whether streams survive: not verified |
| Display connected | The free corner may change. Parked windows stay at the old point, which may now be inside the new display | Follows from the code. Needs the re-park step below |

Display changes, proposed handling: the shell watches the display list (reconfiguration callback, or the 500 ms tick)
and sends `SetDisplays { displays, home }`, where `home` is the display ottospace's window is on. It sends it when the
list changes and when the window's display or scale changes: dragging ottospace to another display must re-park too.
The core recomputes each window's display, each tab's display and each parking spot, and emits a new
`Effect::Repark(pane)` for every pane that is not live. Not `Effect::Parked`: that means "this pane stopped being
live", and the shell's handler for it clears the click guard and can take the keyboard back. `Repark` only moves the
window. The shell also extends the drift check to parked windows.

## Return key and focus

- `live` stays one pane across all displays. Entering a pane on display 2 leaves a live pane on display 1 (already the rule in `Action::Enter`).
- Return key: `Leave`, then make key the window that held the live pane. If none was live, the window used last. The core keeps "focused view"; the shell raises that window only.
- Whether `cx.activate(true)` raises every ottospace window, and whether that matters: not verified.
- Clicking any ottospace window while a pane is live already leaves it (`observe_window_activation`). With several windows, each one's observer must consult the one shared 400 ms guard.
- Keyboard focus moving to another display does not move the pointer. Whether it should: open question.

## Tests

| Loop | Covers | How |
|---|---|---|
| `cargo test` (core) | Parking choice for display arrangements (side by side, stacked, diagonal, negative origins, mixed scale). Tab and view reassignment on connect and disconnect. Focus and send across displays. Return-key target | `SetDisplays` with made-up rectangles. No system call |
| `cargo e2e` (hosting off) | A second window opens, shows its tabs, takes focus keys; state reports views and parking spots | A test-only request sets a fake display list (`OTTOSPACE_E2E` only). The "displays" are halves of the real screen. Only ottospace's own windows move |
| `cargo e2e-apps` (canary only) | Re-park after a display change; enter and leave across two "displays"; scale carried into the stream config | The same fake list. Only the canary's windows move (`OTTOSPACE_HOST_ONLY`) |
| Manual, two real displays | Scale, Spaces, sleep, unplug, fullscreen | The experiments below. No automated run can cover these without a second display |

A `CGVirtualDisplay` could give tests a real second display. It is private API and it rearranges the user's displays,
which can move the user's windows. Not for local runs.

## Recommendation

Build D in stages: A1 and B2 are stops on the way. A1 is useful alone; B2 is optional, and once each display has a window (A2) only its menu-bar count still adds something. Skip C.

- Bryan's note #4 asks for layouts across displays with affinity. Only A2/D gives a second surface; A1 and B only move or point at the one surface.
- Terminals are drawn by GPUI, so a terminal on display 2 needs an ottospace window there. No AX-only design can do that.
- C adds no surface that D lacks. It adds every app's hosting failure at once, and the measured base is two apps.
- The parking and display-change work is needed for all options, and it fixes gaps that exist today.

### First PR

Parking decided in the core, and re-park on display change. One window still.

- Core: `Action::SetDisplays { displays, home }`; `parking(displays, home) -> Point` moves from `display.rs` (it is a decision) and prefers the home display's free corner; `Effect::Repark` for each parked pane when the answer changes. Unit tests for the arrangements above.
- Shell: read the display list (id, UUID, rect, scale) and the window's own display on the 500 ms tick and send the action when either differs; drift check for parked windows; reconfigure the stream when scale changes.
- State snapshot: `displays`; `parking` stays. E2E: fake display list request, canary re-parks when the list changes.
- User-visible aim: parked windows are put back after a display is plugged, unplugged or rearranged. What macOS does to them on unplug is not verified (experiment 5), so this is the aim, not a promise.
- Run experiments 1, 3 and 10 before or with this PR: they can change the rule it builds.

### Then, in order

1. Run the remaining experiments in this order: 2, 4, 5, 6 and 9 next (parking rule and display changes); 8 before step 2; 11 before steps 3 and 6; 7 and 12 before step 6.
2. A1: ⌘⌃←/→ sends the window to the next display. Parking follows. (Step 7 gives these keys a new meaning, focus the window on that display; decide then whether to keep one meaning.)
3. B2: menu bar item with the "needs you" count and a task list. Independent; can move earlier or be dropped.
4. Task owns its tabs (already planned). Do it before views, so views are designed against the final hierarchy.
5. Core views: `View` per display in the model, still one window drawn. A refactor: many existing tests read `layout`, `focused` and `active` and are updated with it.
6. Shell: shared model entity, a window per view, one tick. Second window opens by a key or a setting, not automatically.
7. Focus and send across displays; return key picks the right window.
8. Tab affinity by display UUID, fallback on disconnect, return on reconnect. Kept with session restore (phase D of the plan).
9. Optional: pinned tabs; a second global hot key to send the live pane's tab to the other display.

## Experiments (need a real multi-display setup)

The `probe` tool is in git history (`d483a37:crates/probe`); it prints frames per second for on-screen, parked, restored.
It takes an app name and seconds only: experiments 1, 3 and 12 need it to take a parking position and several windows first.

| # | Run | Result that changes the design |
|---|---|---|
| 1 | `probe` with the window parked at a corner of display B while a video plays; ottospace on display A | Frame rate drops: park only on the pane's own display |
| 2 | Park on a 1x display, pane on a 2x display. Read the captured buffer's pixel size and look at text | 1x content: parking display must have scale ≥ the pane's, or always be the pane's display |
| 3 | Park three animated windows at one spot. Frame rate of each | Any drops: one window per spot. Count the usable corners; consider the virtual-display fallback |
| 4 | "Separate Spaces" on. Park on display B, switch B to another Space. Then enter the pane on display A | Frames stop: park on the pane's display and handle Space change. Enter lands on the wrong Space: needs a fix before A2 |
| 5 | Unplug the display that holds parked windows. Read each window's frame and stream status. Plug it back | Windows land in view (expected): the re-park step is required. Streams die: restart them on display change |
| 6 | Turn one external display off, close the lid, and sleep and wake, each with parked windows | Frames do not resume: restart streams on wake |
| 7 | Native-fullscreen an app on the parking display, and on the ottospace display. Repeat with "separate Spaces" off | Streams stop or the window cannot go live: show the pane as unavailable, or move parking away from that display |
| 8 | Move the current window to a second display placed left of or above the main one (negative origin). Enter a pane | Live window misplaced: fix `on_screen` first |
| 9 | Enter a pane when parking and pane are on displays with different scale. Time until the window looks right | Slow or flickers: parking on the pane's display becomes a hard rule |
| 10 | Set a window so only its top-right pixel is on a display's bottom-left corner. Read the frame back | Clamped: only bottom-right corners are usable |
| 11 | GPUI 0.2: open a second window on a chosen display; check `window.display()` after a move; press the return key with two windows; try a non-activating floating window; try a menu bar status item | Any failure moves work into `ottospace-mac` (AppKit calls) and raises the cost of A2 or B1 |
| 12 | Same as 1 with "separate Spaces" off | Behaves better: recommend that setting in the README, as AeroSpace does |

## Open questions

1. Task switch on two displays: should every display change, or does the second display keep its content (pinned tabs)?
2. What is on the second display most of the day: terminals, or apps such as browser and Slack? If only apps, a smaller design may do.
3. Should the second window appear on its own when a display is connected, or only when asked?
4. The rail: on one window, or on each?
5. Do you use "Displays have separate Spaces", and do you use more than one Space per display?
6. Do you run fullscreen apps on a display next to ottospace?
7. When focus moves to another display by key, should the pointer follow?
8. Is the menu bar item (B2) wanted, or is the status bar's "needs you" count enough?
9. Keys: are ⌘⌃←/→ and ⌘⌃⇧←/→ free in your setup?
