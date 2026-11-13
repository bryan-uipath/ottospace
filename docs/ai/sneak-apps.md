# Sneak labels inside hosted apps

## The decision

2026-11-13. The user: "yep! i want to be able to have sneak labels be pressable in apps".

Until then sneak mode (⌘⌃G) labelled only controls ottospace draws, and pressed nothing in another
app ([accessibility.md](1-initial-brainstorm/accessibility.md), "Built: sneak over ottospace's own
UI"). The user's own brainstorm asked for labels inside apps; this records that they now want it,
and what it allows.

## What is allowed

- **Which pane.** The focused app pane only, while it is parked (sneak mode never runs while an
  app is live). Labels stay few and one window is read. To label another pane, focus it (its own
  label, or a click) and press ⌘⌃G again. Widening later: read every visible app pane in
  `read_sneak_app` and let `Sneak::read_app` keep one `AppLabels` per pane; the label rules do not
  change.
- **Which elements** (`ottospace_core::sneak_apps::targets`): those the app's accessibility tree
  says support `AXPress`, or are text fields (`AXTextField`, `AXTextArea`, `AXSearchField`,
  `AXComboBox`); not disabled; at least 4 points each way where the pane shows them (a cropped
  title bar shows none); the innermost of nested ones (a button in a pressable row, not the row);
  at most 150, nearest the pane's top left.
- **What a label does.** A button: `AXUIElementPerformAction(kAXPressAction)` on that element. A
  text field: `AXFocused` = true, and the pane goes live so the user types there. Nothing else:
  no value set, no menu opened by ottospace, no scrolling, no synthetic click, no posted event.
- **Stale elements.** At press time the element is found again by its path from the window, and
  `sneak_apps::check` compares it with what was labelled: the same role, not disabled, still
  pressable, and within 2 points of the same place in its window. Otherwise nothing is pressed and
  the status bar says the control moved. The element checked is the element pressed (the same
  `AXUIElement`), so only the app's own change in that instant lies between them.
- **Who.** `Origin::User` only. `Model::refusal` refuses a `SneakType` from any other origin whose
  letters would choose an app's label or start one (else a client could type `g` and the user's
  next `a` would press `ga`). The property test checks that no `Effect::PressApp` comes from
  anything but the user's own `SneakType`, and that no other origin types toward an app's label.
  A text field's label makes the pane live only once the field took focus.
- **Labels.** One namespace with ottospace's own: the app's take letters the own labels leave free,
  so those never change when the tree arrives (a property check). With an app pane in focus the own
  labels leave at least one label free; if none is, the app gets no labels.
- **Labels drawn after a letter.** The app's tree arrives while the user may already have typed a
  first letter. Rule: what is typed counts as taken, so no app label drawn then goes on with it;
  every letter of an app label is typed after the label was drawn (a property check: a press is
  only of an element labelled when nothing was last typed). If no label is free outside what is
  typed, the app gets none until the next read. A later read keeps a label on its element only
  while the element keeps its path, role and place (within 2 points); one that moved loses its
  label. A label is never handed from one element to another while the mode is on. Tinted APP mode's orange. Read off the main thread
  with the `ax_tree` reader's limits (depth 40, 3000 nodes, 0.5 s per call, the first timeout ends
  the walk), once on entering and then each second while the mode is on; a label follows its
  element (same path and role) and goes when the element does. A slow app: ottospace's own labels
  show at once, the app's when read.
- **Privacy.** The tree is stripped of every text (title, description, value) on the read thread,
  before it reaches the model. The state has each label's frame, path and role, and counts
  (`sneak.app`), no text. Nothing is logged.

## What is not allowed

- **Agents.** The control socket gains nothing. A client cannot choose an app's label (refused,
  with the reason), and no request presses, focuses or sets anything in another app. Its letters
  could otherwise press "Send" in Mail or "Allow" in a permission dialog with ottospace's
  Accessibility grant; the user's typed letter is the trust decision, and it is theirs alone. The
  agent requests in accessibility.md ("Next step 2") stay a separate decision.
- **Fuller accessibility modes.** ottospace does not write `AXManualAccessibility` (Electron) or
  `AXEnhancedUserInterface` (Chromium). Such apps expose little until a client does, so they get
  few labels; the status bar's count (`N controls in <App>`) makes that visible.

## Tests

- Core: `sneak_apps` unit tests (nested, disabled, off the pane, tiny, the cap, labels beside own
  ones, later reads, stale-element refusal, origin refusal); the property test's maker for
  `SneakApp` and its checks (app labels only on the pane read, one label per element, presses only
  from the user's letter of a labelled element).
- `cargo e2e`: a fixture tree through the test-only `sneak_app_tree`; a client's letter refused,
  the user's asks for the press of the right path.
- `cargo e2e-apps`: the canary's checkbox pressed through its label (the canary reports it), its
  disabled checkbox unlabelled.

## The wheel in a parked app pane

2026-11-13. The user: "can we forward mousewheel events to the app without needing to foreground
it?". The second kind of the user's own input that reaches another app without entering its
pane, after the sneak label.

- **What.** The user's wheel or trackpad over a parked app pane's picture scrolls that app's
  window. The window is not raised or activated, and the real pointer does not move. Nothing else
  is passed on: a click, a key or a drag still enters the pane first.
- **How.** The system's own scroll event is copied (its phases and momentum with it), added up
  per frame (same pane, window and phase), capped at 600 points each way by the core
  (`app_wheel::cap`), placed at the point of the parked window under the pointer (the picture
  shows the window unscaled from its top left, below the cropped title bar), marked for that
  window (`kCGMouseEventWindowUnderMousePointer…`), and posted to the app's process alone with
  `CGEventPostToPid`, never to the system's event stream. At post time the pane must still
  host that window of that process, parked. A test's injected scroll, which the system never
  queued, is made from its deltas instead.
- **Who.** `Origin::User` only: `Model::refusal` refuses `AppWheel` from a client or the system,
  and the property test checks that no `Effect::AppWheel` comes from anything else. No socket
  request scrolls an app; the test-only `mouse` `wheel` without `"as":"user"` does not.
- **Off.** `"wheel_to_apps": false` in `settings.json`.
- **Which apps.** AppKit apps take scrolls posted to their process while in the back. Chromium
  and Electron apps may ignore a scroll for a window that is not key, or that is parked off
  screen; not verified here. The accessibility route (a scroll area's scroll bar `AXValue`) would
  set a value, which the user has not approved, so it is not used.

## Open questions

- A per-app, user-initiated request for the full tree (`AXManualAccessibility` /
  `AXEnhancedUserInterface`) for Electron and Chromium apps, with its cost (Chromium animates
  AX-driven moves once it is on).
- `AXPress` reports success in some web views and does nothing. No fallback click is made.
- A one-letter label in an app can press, e.g., "Delete". ottospace's own × always gets two
  letters; an app's destructive controls cannot be told apart from its tree.
