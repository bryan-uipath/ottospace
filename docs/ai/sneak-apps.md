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
  so those never change when the tree arrives (a property check). Every app label has two letters
  at least (see "Two letters in an app" below). With an app pane in focus the own
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

## Two letters in an app

2026-11-13. A review: "An app's destructive controls can get one-letter labels": one mistyped key
could press Delete or Send in another app. The user: "multi letter sneak labels are OK".

- **Rule.** Every label on an app's element has `sneak_apps::MIN_LETTERS` (2) letters or more,
  always; ottospace's own keep theirs (home row single letters first, a × always two). The first
  letter only narrows: the app labels it starts stay, the rest dim; ⌫ takes it back. Property
  checks: no app label is shorter, no label (own or app) starts another, and no single keystroke
  with nothing typed before it presses in an app.
- **The space.** `more_labels` walks the labels free of the own ones and of what was typed, and
  goes one level further down from any one-letter label. The letters no own label starts come
  first: 19 two-letter labels under each (`ta ts td ...`). E.g. 3 own labels leave 16 letters,
  304 labels, past the cap of 150. Then the two-letter labels under a letter own labels share (a
  × at `ra` leaves `rd`, `rf`, ...), kept for last so a client can still type `r` toward the ×
  while apps fit elsewhere. Then three letters, the shortest in reading order first. With an app
  pane in focus own labels leave at least one label free. One letter, `p` (18 own): 19
  two-letter labels, three letters for the rest (150: 11 of two, 139 of three). At worst one
  two-letter label, `pp` (36 own, no ×): three and four letters (150: 11 of three, 139 of four). The cap and
  its order are unchanged: the 150 nearest the pane's top left get labels.
- **Stable.** Labels are handed out on the first read and kept by element after (as above), so
  the split never moves while the mode is on. The epoch rule holds: a typed letter counts as
  taken, so no label drawn after it goes on with it.
- **Considered, rejected for now.** Leaving "destructive-looking" controls unlabelled: the tree
  has no reliable signal (titles are stripped, and a role does not say Delete from Save).

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

### Tried and dropped: posting to the parked window (#101)

The system's scroll event, copied, marked for the parked window
(`kCGMouseEventWindowUnderMousePointer…`) and posted to the app's process alone
(`CGEventPostToPid`). On GitHub's runner it was posted, and the canary (GPUI, parked off screen)
received nothing, in neither a native scroll view nor its GPUI view. Removed.

### The peek

2026-11-13. The user, after #101: "finder doesn't handle the mouse wheel either. i guess we can
raise the app when mousewheel is there". So the app goes live for the scroll.

- **What.** The user's wheel over a parked app pane's picture puts the real window over the pane
  exactly as entering does (`Hosted::place`, the same crop), and macOS gives the scrolls that
  follow to the window under the pointer. Not an entry: the mode stays as it was, and the status
  bar says `PEEK <App>`.
- **Activation.** Needed. On CI neither `AXRaise` alone nor ordering ottospace's window below the
  canary's (`orderWindow:relativeTo:`) put it above ottospace: the next scroll still came to
  ottospace's picture. With the app activated (as entering does) it is in front and the next
  scroll reaches it. So the keyboard goes to the app during a peek.
- **The first scroll.** Lost. A copy of it re-posted at the HID level, with the window fields
  cleared and after the app was in front, came back to ottospace's picture on CI. The re-post was
  removed; nothing is posted.
- **The end.** The core ends a peek (`Effect::Parked`, the same park path as leaving, which
  activates ottospace again when the app has the keyboard) when the pointer leaves the pane, 1.5 s
  pass with no scroll (`app_wheel::QUIET_MS`; the shell reads the system's last-scroll time), or
  what it was over changes (tab, tree, mode). A click or a key since the peek began (the system's
  last-event times) makes it an entry. The window stays in the hosted record throughout, so a
  crash during a peek is restored like any hosted window (`e2e-apps`: the guardian puts a peeked
  window back).
- **Who.** `Origin::User` only: `Model::refusal` refuses `AppWheel` from a client or the system
  (the status bar says why), and the property test checks that an `Effect::Peek` comes only from
  the user's `AppWheel`, that there is at most one peek, of a parked app pane on show, and that a
  peek that ends is parked, made an entry or its pane closed. The shell reads the system's input
  times and the pointer's place; it watches no events and posts none.
- **Not under** sneak mode, the palette, a box, the quit question, the accessibility overlay, or a
  drag (the left button down), nor while another app is in front (its end would give ottospace,
  not that app, the keyboard).
- **Off.** `"wheel_to_apps": false` in `settings.json`.
- **Verified on CI** (`e2e-apps`, a real HID scroll at the pointer): the peek starts, the window
  covers the pane under the pointer, the canary is in front, the next scroll reaches it, the mode
  stays normal, and a kill during a peek is undone by the guardian. Printed, not yet asserted: the
  park when the pointer leaves or the wheel is quiet, the click that enters, the restore at quit,
  and the client's refusal in the status bar. AppKit apps (Finder), Chromium and Electron are not
  verified.

## Open questions

- A per-app, user-initiated request for the full tree (`AXManualAccessibility` /
  `AXEnhancedUserInterface`) for Electron and Chromium apps, with its cost (Chromium animates
  AX-driven moves once it is on).
- `AXPress` reports success in some web views and does nothing. No fallback click is made.
