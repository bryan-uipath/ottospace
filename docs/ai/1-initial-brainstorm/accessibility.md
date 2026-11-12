# Accessibility trees: reader, debug overlay, next steps

An experiment toward backlog item 2 and sneak mode (idea #8, "Control+G"). Built: a read-only
tree reader, an `ax_tree` socket request, and a debug overlay. Not built: sneak mode, AX actions.

## What the reader returns

`ottospace_mac::ax_tree::read(window, limits, fetch)` returns `ottospace_core::ax::Tree`: plain,
serde data with no macOS types.

- Per node: `role`, `subrole`, `title`, `description`, `value` (strings cut to 120 characters;
  numbers as text), `frame` (global screen points, top-left origin), `enabled`, `actions`
  (e.g. `AXPress`), `children`. A missing or unreadable attribute is left out.
- `count`: nodes read. `truncated`: `depth`, `nodes`, `timeout`, each true if that limit cut the walk.

Core helpers (`ottospace_core::ax`, unit tested, one recorded Calculator fixture):

- `flatten`: every node in reading order with its path (child indices from the root).
- `is_interactive`: has `AXPress`, `AXPick`, `AXConfirm` or `AXOpen`, or is a text field, text
  area, combo box, search field or slider; is not disabled; has a frame with area. `AXShowMenu`
  alone does not count: web views put it on nearly everything.
- `labels(n, alphabet)`: `n` prefix-free labels, shortest first (`a s d f g h j k l q w e r t y u i o p`,
  then two letters). A label is chosen as soon as it is typed.
- `Target { frame, source }`: something a label can go on, in ottospace window coordinates.
  `Source::App { pane, path, role }` is a hosted app's node; `Source::Own { id }` is one of
  ottospace's own controls. Labels do not care which.
- `app_targets(pane, root, window, content)`: a hosted window's interactive nodes, moved into the
  pane's content area. Nodes scrolled or clipped out of the pane are left out.
- `assign(targets, alphabet)`: one label namespace for the whole window, handed out in reading
  order (top to bottom, then left to right), so a control keeps its label while the targets above
  and left of it are unchanged. Tested with own controls and app nodes mixed.

## Limits

- **Depth** (default 40) and **node count** (default 3000), both per request.
- **Breadth first.** A node cap then drops the deepest nodes. Pre-order spent the whole budget on
  Activity Monitor's process table and never reached its toolbar.
- **Timeout.** Each element gets `AXUIElementSetMessagingTimeout(0.5s)`; the default is 6s. The
  first call that times out ends the walk (`truncated.timeout`), so a hung app costs one timeout,
  not one per node. Not covered: finding the window first (`AXWindows`, its frame) uses AX's
  default 6s, so a hung app can answer "the window is gone" after 6s.
- **Stale elements.** Elements that error as invalid (a row replaced during the walk) are dropped.
  Before this, about 1,170 of Activity Monitor's nodes came back with no role.
- **Off the UI thread.** `ax_tree` and the overlay read on a background thread. `AXUIElement` is not
  `Send`, so the thread finds the window again by pid and window id.

## Measured cost

Debug build, on a heavily loaded machine. Each app's main window, as it was on screen. Batched:
one `AXUIElementCopyMultipleAttributeValues` plus one `AXUIElementCopyActionNames` per node.
Single: one call per attribute (nine) plus the actions. Batched times are warm (second run).

| App | Kind | Nodes | Interactive | Batched | Single |
|---|---|---|---|---|---|
| Finder | native (AppKit) | 1038 | 133 | 556 ms (0.54/node) | 1483 ms (1.43/node) |
| Activity Monitor | native, live table | 3000 (cap) | 15 | 2151 ms (0.72/node) | 3208 ms, 2520 nodes (1.27/node) |
| Calculator | native (SwiftUI) | 36 | 24 | 8 ms | 14 ms |
| Google Chrome | browser | 63 | 40 | 10 ms | 16 ms |
| VS Code | Electron | 12 | 3 | 1 ms | 2 ms |
| Spotify | CEF | 17 | 3 | 2 ms | 5 ms |
| ottospace-canary | GPUI | 6 | 3 | 1 ms | 3 ms |

- Batched is 1.8-2.7x faster on large trees. On tiny trees both are noise.
- A cold first read can cost much more (Calculator: 61 ms cold, 8 ms warm).
- About 0.5-0.7 ms per node batched. A 3000-node read takes about 2 s: too slow to repeat on every
  key press, fine on demand.

## What apps expose

- **Native AppKit/SwiftUI**: rich and accurate. Buttons carry `AXPress` and a `description`;
  tables expose every row, not just the visible ones (Activity Monitor: about 1,250 rows).
- **Electron and CEF** (VS Code, Spotify): only the window chrome, the traffic lights and empty
  groups. The web content is not built until a client writes `AXManualAccessibility` (Electron) or
  `AXEnhancedUserInterface` (Chromium). Both are writes, so this read-only experiment did not.
- **Chrome**: its own UI (tabs, toolbar, about 40 targets) but not the page, for the same reason.
- **GPUI** (the canary, and ottospace itself): `AXWindow` > the three traffic-light `AXButton`s
  (close, minimise, zoom, each with `AXPress`), the title as `AXStaticText`, and one childless
  `AXGroup` for the whole GPUI view. Nothing inside the view is exposed.

Writing `AXEnhancedUserInterface` has a cost that other window managers report and that matters
here: Chromium then animates AX-driven moves and resizes. Measure that before turning it on for
hosted windows.

## Debug overlay

⌘⌃A, or `{"cmd":"ax_overlay"}`, toggles it on the focused app pane; `{"cmd":"ax_overlay","refresh":true}`
reads again. It outlines each target and shows its label as an amber pill (`theme::SNEAK`,
`#ffb900`, dark text) with the raised-panel shadow scaled down, so it reads over any content. The
role follows the label where the box is wide enough. A line gives node count, target count, time
and truncation. It reads on toggle and refresh only, never continuously.

- Drawn only while the pane is parked: the stream is ottospace's own pixels. A live window sits
  above ottospace and covers anything drawn there. Turning it on parks the live pane (core rule).
- Mapping: a parked window is parked its title bar (`crop`) taller than its pane, and the stream
  shows the part below the bar, filling the pane. So a frame maps by `y - crop`, then scales by
  pane size over shown size (1 unless the window refused the size). The window's frame is read
  just before the tree, in the same global coordinates. The overlay draws over the whole
  ottospace window, in window coordinates, as sneak mode will.
- **Title-bar controls are dropped.** The traffic lights (and anything else in a cropped bar) are
  not in the picture, so they get no label. They are not drawn over ottospace's pane header
  either: that header is ottospace's own UI, with its own close button, so a label there would
  point at nothing the user sees. Controls straddling the bar's edge are clipped to what shows.
- `cargo e2e-apps` checks the canary: `ax_tree` returns its three pressable traffic lights, all in
  the cropped bar, and the overlay reads the tree but labels nothing.

## Sneak over ottospace's own UI

Sneak labels ottospace's own controls too: rail icons, secondary-bar rows, tabs and their close
buttons, the new-tab and split buttons, pane headers and close buttons, picker rows, and each pane
body (to focus it). All of them share one namespace with the hosted app's nodes (`assign`).

- **Collecting them.** GPUI hands each element its bounds at prepaint; the shell already records
  pane bounds that way (a `canvas` prepaint closure writing into `cells`). A small registry filled
  during render, `id -> (bounds, Action)`, gives every own control as a `Target` with
  `Source::Own { id }`. Activating its label dispatches the same `Action` its click handler does.
  Activating a hosted-app label performs `AXPress`.
- **Cost.** One closure and one map entry per control per frame; tens of controls, so negligible.
  The real cost is discipline: each clickable element needs a registry line beside its handler,
  and a control without one is silently unreachable. A test can compare the registry with the
  e2e state (every tab, pane and rail section has an entry).
- **Risks.** The registry must match what was drawn: clear it at the start of each frame, and take
  targets from the last painted frame, not from the model. Overlays (picker, key reference) cover
  controls under them; while one is open only its own controls should be labelled. Scrolled lists
  (the Apps bar, picker rows) report bounds outside their visible area; clip each target to its
  scroll container, as `app_targets` clips to the pane. Elements in a background tab are not
  drawn, so they are naturally absent.
- **Where labels draw.** Own-UI labels are always drawable: they are ottospace's pixels. Hosted-app
  labels are drawable only over a parked pane. Proposed: entering sneak mode parks the live pane
  (as the debug overlay does) and labels it over its stream; leaving sneak mode by pressing a
  label in that app makes it live again only if the target needs typing.

## Built: sneak over ottospace's own UI

⌘⌃G (`{"cmd":"sneak"}`). Not ⌃G alone: that is BEL in a terminal, and readline, fzf, vim and
agent CLIs use it.

- Core: `sneak.rs` (`Sneak`: labels, `typed`, narrowing), `Mode::Sneak`, actions
  `Sneak { targets }` (the key: on or off), `SneakEnter` (the request: on, or stays on),
  `SneakDrawn` (each painted frame), `SneakType`, `SneakBackspace`, `LeaveSneak`, and
  `Effect::Activate { id }`. A letter no label goes on with cancels, as planned below; keys that
  are not letters do nothing.
- Labels follow what is drawn. The shell reports the controls of each painted frame
  (`SneakDrawn`); a label keeps its letters while its control is drawn and moves with it, goes
  when the control is not drawn, and a new control gets none. So a label can only activate a
  control of the last frame, and `Workspace::activate` refuses an id that frame did not draw.
- A pane's or tab's × (`close:`, `tab-close:`) always has two letters: one stray key closes nothing.
- What ends the mode: the user's own moves only. Every action comes with its `Origin` (`User`,
  `Socket`, `System`): a socket client's request and news (the clock, a list read again, a
  window an app opened, a pane closed because its shell or window is gone) never end it, or the
  next letter of a label would reach a terminal. Nor may they take the keyboard from under the
  labels (`Action::takes_keyboard`, `Model::refusal`): a socket request that would is answered
  "sneak mode is on". The mode's own actions work from any origin: a label typed with the `key`
  request is input to it; any other key or pointer request from the socket is refused. A ⌘ shortcut leaves first and then runs. The key that ended the mode types nothing
  while it is held, and the rest of a `key` request is dropped.
- Shell: `sneak.rs`. Each control has a `controls.mark(id)` line beside its click handler, which
  notes its bounds at prepaint, clipped to what shows. `Workspace::activate` runs what the click
  handler runs: the same function for a pane's body, a pane's × and a tab and its ×, the same
  `Action` for the rest. No mouse event is made.
- Starts from normal and list mode only. A live app has the keyboard, so no label is ever under
  a real window; the picker, the new-task box and the key reference cover the controls. Starting
  hides the debug overlay, whose chips use the same letters.
- Keyboard layouts: the letter is GPUI's `Keystroke::key`, which on a layout with no Latin
  letters is the letter the key has under ⌘, so the labels can be typed there.
- Trust. Sneak presses no control inside another app and sets no value; entering an app pane raises its window, as a click on the pane does (`Action::Enter`: `AXRaise` and
  app activation, less the click a mouse click replays into the app). It reads no AX tree.
- Not built: labels inside a hosted app (`Source::App` targets are dropped by `Sneak::new`), and
  pressing or setting anything in one. What follows is still the plan for those.

## Next step 1: sneak mode in hosted apps

1. **Core.** `Mode::Sneak { targets, typed }`. Actions `Sneak { targets }` (the shell passes the
   read-in targets), `SneakType { key }`, `SneakCancel`. Typing narrows by prefix; a full label
   emits `Effect::Press { pane, path }` for an app node, or the control's own `Action`; Esc or a non-label key cancels. Labels are prefix-free,
   so there is no timeout.
2. **Shell.** ⌃G: park the live pane if any, read the focused app pane's targets off the UI
   thread, join them with ottospace's own (above), then enter the mode. Draw the labels as the
   overlay does now, with typed letters dimmed.
3. **Press.** Find the element again by path, check that its role and frame still match (the tree
   may have changed since the read), then `AXPress`. AX actions do not depend on window position,
   so the press works on a parked window, where synthetic clicks do not reach (see
   [hosting-windows.md](../hosting-windows.md)). Text fields get `AXFocused`, then the pane goes
   live, so typing lands.
4. **Known failure.** Earlier work on an AX CLI found that `AXPress` reports success in web views
   but does nothing. Keep a fallback: go live and click the target's centre.

## Next step 2: agent requests

- `ax_find {pane, role?, text?}`: matching nodes with paths, so an agent need not read a whole
  tree. Text matches title, description and value.
- `ax_press {pane, path, expect_role?}`: `AXPress` on one node, refused if the node at `path`
  is gone or its role differs.
- `ax_set_value {pane, path, value}`: set `AXValue` on a settable text field.
- **Trust.** Today a socket client can only drive ottospace and type into terminal panes. These
  requests would let it press buttons and fill fields in other apps: "Send" in Mail, "Delete" in
  Finder, an "Allow" in a permission dialog. Same-user processes can already do this with their own
  Accessibility grant, but ottospace would lend its grant to any client of its socket.
  Proposal: only hosted panes, never other windows; off by default (`OTTOSPACE_AX_ACTIONS=1`
  or per-app allow list); log every action; refuse when the target app is frontmost and the user
  is typing.

## Not done

- Live coverage of real controls inside a window. The canary exposes none. A native `NSButton`
  and `NSTextField` in it would need three more objc2-app-kit features and a direct
  `objc2-foundation` dependency for `NSString`, and it is unproven that GPUI's view exposes
  subviews to AX.
- Electron and Chrome with their AX mode on.
- Release-build timings.
