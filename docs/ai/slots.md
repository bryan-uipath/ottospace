# Slots: a pane that waits for a window

A slot is a pane that keeps its place in the layout and says which window belongs in it: "Zoom's
main meeting window", "the Figma window titled `*Roadmap*`". When no such window is open it is
empty and says what it waits for; when one opens it offers it (or, if the user chose so, takes it);
when the window closes it goes back to waiting. Slices 1 to 3 built (the selector, the model, on
screen); 4 to 6 not.

The owner, 2026-11-16: "i wonder if we'd want like a selector (like has certain ui) to
differentiate. its unclear how we'd differentiate the main meeting for zoom. but i guess we need
both the app and window title and identifier."

So one window selector, used by slots, by the `windows` rules (`ottospace-core::rules`) and by the
`meeting` windows of a `transcribe` rule ([transcripts.md](transcripts.md) §1a). Built on what was
measured in [zoom-ax.md](1-initial-brainstorm/zoom-ax.md): Zoom's main meeting window has
`AXIdentifier` `zm.meeting.window.main`; the share viewer has the same title and no identifier; the
`Zoom Workplace` home window stays open; toolbar buttons carry identifiers such as `leave`.

The approved mock (Slots, four states; not in the repository) is described in §2 and §3. Its app
is a made-up "Meetings" app; so are the examples here.

## Rules (hold from the slice that brings each part)

1. **Only the user makes, edits, fills or clears a slot.** A fill is the user's act now (`Bring it
   here`, ↵) or their choice made earlier (`Matching: Automatic`, which only they can set). Putting
   a window in a pane is the user's act too: that pane becomes a slot when its window closes (§2a).
   A socket client cannot make one, accept or decline an offer, change one, clear, close, move or
   swap its pane, and never puts a window into one (§6).
2. **Never pull a window from under the user.** Manual (the default) offers and waits. Automatic
   takes only on the tab on show, only a window that opened while the slot waited, and never the
   window the user is using outside ottospace: those are offered (§3).
3. **A window is in one pane at most**, slot or not. A window in a pane is never a candidate.
4. **Window rules come first.** A slot holds what a new window's judgement offers or splits
   (`rules::slot_holds`): a standard window, a dialog or a panel, as the picker lists them. Never a
   sheet, a tiny window, another kind, or one the rules ignore or float: a slot cannot host what
   hosting refuses. (The owner, 2026-11-24: a slot by the title `Meeting chat` never offered Zoom's
   popped-out chat, an `AXSystemDialog`, which the picker lists; a dialog was a candidate only for a
   slot that named its kind.)
5. **No new text in the state.** A slot's selector, its candidates' titles and identifiers are drawn
   in the pane and never sent on the socket: the core holds them in fields the state skips
   (`#[serde(skip)]`), and a property check holds it. A filled slot is an app pane and carries what
   an app pane carries today, no more (§6).
6. **The core decides.** Which window a slot selects, offers or takes, and whether a new window of a
   hosted app is a slot's or a new pane: all in `ottospace-core`, from window facts the shell passes
   in. The shell reads facts and hosts what the core names.

## 1. The selector

### Shape

```json
{ "app": "us.zoom.xos", "identifier": "zm.meeting.window.main" }
{ "app": "Meetings", "title": "*Incident*" }
{ "app": "us.zoom.xos", "title": "Zoom Meeting", "has": [{ "role": "AXButton", "identifier": "leave" }] }
```

| Field | Matches | In v1 |
|---|---|---|
| `app` | The app's name or bundle id, as `windows` rules today | Yes |
| `title` | The window's title | Yes |
| `identifier` | The window's `AXIdentifier` (one app gives its windows fixed names: Zoom's `zm.meeting.window.main`) | Yes |
| `subrole` | The window's `AXSubrole`: a slot kept from a dialog names its kind, and holds only that kind | Yes (exists) |
| `has` | The window holds elements: each entry (`role`, `identifier`, `description`, each a pattern) is met by some element, all fields of an entry by the same one. Every entry must be met | No: slice 6 |

- **Every field given must match**, as in `windows` rules today. A selector with no field selects
  every window (a rule may want that; a slot always has `app`, the dialog writes it).
- **Text is the glob `windows` rules already use** (`rules::glob`): the whole text, `*` for any run,
  letters in either case. "Contains" is written `*x*`, "is" is written `x`, a pattern is anything
  else. One syntax for every field.
- **`Selector` is one struct in `ottospace-core::rules`**, with `selects(&Facts) -> Match` (`Yes`,
  `No`, or `NeedsTree` while `has` is unread; until `has` exists, slice 1 returns `bool`). `Facts` gains `identifier: Option<&str>`. Today only
  `ottospace-mac::ax_tree` reads `AXIdentifier`; slice 1 adds it to `ax::Window` and the facts the
  window list gives (`standard_of`).
- **Evaluated in the core, once.** The shell passes facts and the core runs `selects`, for slots,
  for `windows` rules and for `meeting`. Transcripts §1b has the shell send `ListenMeeting { on }`,
  an answer it worked out; that slice instead sends the facts (or the same `SlotWindows`-shaped
  look, §6) and the core decides. One place evaluates a selector.
- **Where it is written.** `windows` rules: the selector's fields beside `do`, as today. A
  `transcribe` rule's `meeting`: a list of selectors (its documented fields are already a subset).
  A slot: under `select` on its pane record in `layout.json` (§4). The same JSON in all three, so one
  can be copied to another.
- **No `flatten`.** serde refuses `deny_unknown_fields` with `#[serde(flatten)]`, and a misspelt key
  must stay an error (`rules.rs` tests it). `Rule` and the slot record deserialize into their own
  plain structs with every field and build the `Selector` from them.

Rejected:

- **Regular expressions.** A second syntax beside the glob the settings already use, with its own
  escaping for users to learn. Nothing measured needs more than a glob.
- **Title alone, or title and size.** Zoom's two `Zoom Meeting` windows share the title, and their
  sizes overlap with the chat panel open (zoom-ax.md: 911 to 1083 against 804 to 818 wide, the first
  reading 696 to 1399). The identifier names the main one exactly, as read in one Zoom version
  (7.2.x); whether it holds across versions is not measured. The size pick (§2) stays as the
  fallback for an app without identifiers.
- **`has` in v1.** Zoom, the one app measured, needs only `identifier`. `has` costs a tree read per
  window (below) and has no measured case yet. Designed now so the shape does not change; built
  when an app needs it.
- **A tie-break inside the selector.** "Which window" is a choice among the windows a selector
  selects, and only a slot makes it: a `windows` rule acts on every window it selects, and a
  meeting is on while any window matches. So `pick` is a field of the slot (§2), not of `Selector`.

### Matching order

The cheap facts first, the tree last. For each window of a slot's app, in this order, the first
"no" stops:

1. `app` (from the running app, no AX call)
2. `rules::slot_holds` (rule 4; what a new window's judgement offers)
3. not in any pane (rule 3), and not passed over for this slot (§3)
4. `subrole`, `identifier`, `title` (one AX read of the window's attributes, which hosting already does)
5. `has` (a bounded tree read, below)

Then `pick` chooses among the windows left (§2).

### What `has` costs

A read of the window's tree with `ax_tree::read`, under proposed limits of depth 30, 2,000 elements
and 250 ms (`inspect::LIMITS` with a shorter time; the default `Limits` has no time bound), stopping
once every entry is met. Off the UI thread, as `ax_debug` reads are. Not measured with `ax_tree`:
zoom-ax.md's "well under a second, a few hundred elements" was System Events, and the depth of
Zoom's toolbar buttons is unknown. Slice 6 measures before it fixes the numbers.

- **When:** only for a window that passed steps 1 to 4, of a selector with `has`, while that
  selector is wanted: a slot empty on the tab on show, a `windows` rule when a new window appears, a
  `meeting` look (transcripts §1b's every 2 s while a call is on).
- **Cached in the core**, by (pid, window id, title): the answer stands until the title changes.
  A "no" is asked again after 30 s (a toolbar can appear after the window does). The core asks
  with an effect (`ProbeTree { window, has }`) and gets `TreeProbed { window, title, found }`
  (System); the shell reads, the core decides and remembers.
- **At most 4 tree reads per look** (up to 1 s of reading, off the UI thread), the rest next look.
  A window whose read timed out counts as "no" until its next ask.

## 2. Making a slot (mock state 1)

From a live app pane: the header's ⋯ ▸ **Keep this pane as a slot**, or the palette's `Keep Pane
as a Slot`. The pane becomes a filled slot with the window it holds. App pane headers have no ⋯
today; slice 3 adds one (icon, title, `fit`, ⋯, ×).

The dialog, filled in from the live window; the user narrows:

| Row | Filled in with | Shown |
|---|---|---|
| App | The window's app. Fixed | Always |
| Window id | The window's `AXIdentifier`, ticked | Only when the window has one. Ticked when no other window of the app has the same id |
| Window title contains | The window's title in full, for the user to cut down (empty when Window id is ticked) | Always |
| Which window, when several share that title | `The largest`, `The smallest`, `The one on display N`, `Any that is not already in a slot` | Only when more than one of the app's windows open now passes the rows above |
| Matching | `Manual` (default) / `Automatic` | Always |
| More options (folded): when the app is not running | `Wait` (default) / `Launch it` | Folded |
| More options (folded): when its window closes | `Keep the slot` (default) / `Close the pane` | Folded |

Cancel, Keep (↵). Keep sends `SlotMake` (User only).

**As built (slice 3, the owner's call 2026-11-18): a checklist, no free text.** Replaced by the
form below (board H4, direction (b), the owner's call 2026-11-23).

**The form (`ui::dialog`, H4).** Its rule in one sentence on top (`Dialog::headline`), then a form
whose every choice shows how many of the app's windows open now it takes (`Dialog::count`):

| Row | Choices |
|---|---|
| Window | Any window · the window id (when the window or the slot has one) · the title |
| Title | is · contains · starts with, and the words (the window's whole title to begin with). Dimmed while the rule is another; a click on a test or the words chooses it |
| Kind | for a dialog or panel only: said, not a choice (only that kind; another needs core work) |
| Several | the largest · the smallest · the frontmost · display N |
| When it opens | Offer it (O) · Take it (T), and a line saying what that does |

Under them the app's windows open now, those the rule takes ticked (`Dialog::listed`, with #377's
details); the action row says how many match and has Keep slot (Save when editing) with ↵. × and
Esc close it. The default with no window id is any window of the app, the largest (§2a's reason:
a meeting's title changes); with one, the id. A title rule is written as the glob the selector
reads (`Test::glob`: `Weekly`, `*Weekly*`, `Weekly*`) and read back as a test and words
(`Test::read`); a pattern written by hand stays as it is (`Test::Pattern`). The words are a text
field (`"slot"`) that has the keys while the rule is by title; O and T choose only while it does
not. The state has what is chosen (`by`, `test`, `pick`, ...), never a title, an id or the
words. Offer again, Give back the window and Clear slot are in ⋯'s menu, not the dialog.

`pick` in the record: `"largest"`, `"smallest"`, `{ "display": 2 }`, `"any"` (unset). Displays
are numbered by the core from their frames, passed in the look: 1 is the display with the menu
bar, the others left to right, then top to bottom. `"any"` takes the frontmost of the windows left.

Rejected:

- **The whole title kept by default.** A meeting's title changes with each meeting; kept whole, the
  slot would wait for that one meeting only. Prefilled so the user can see and cut it, and empty
  when an identifier does the work.
- **Making a slot from an empty pane, with an app picker.** No window to fill the dialog from, so
  every field typed by hand. Later, if asked.

## 2a. A pane the user put a window in (the owner, 2026-11-23)

The owner put Zoom's meeting windows in a tab's panes, without Keep; when the meeting ended every
pane closed and the tab went. Expected: the tab keeps its places, each waiting and saying for what,
so the next meeting's windows fill them. So, with `slots` on:

- **Its window closing makes it a slot.** An app pane whose window the user put there (the picker,
  ⌘E, Put it here, a restored layout) does not close when the window goes or its app quits: it
  becomes a slot (`WindowGone`, `Model::kept_place`), Away then Empty, or Absent, as any slot.
- **What it waits for:** the app, and the window's `AXIdentifier` as read when it was hosted
  (`Action::WindowGone::was`): Zoom's meeting window is `zm.meeting.window.main`, and so is the next
  meeting's, with another window id and title. A window with no identifier gives `identifier: ""`,
  which selects only windows with none (§1), so Zoom's home window does not take the meeting's
  place. Never the title: a meeting's changes (the dialog's own default for a window with no id,
`Title is “…”` whole, would wait for that one meeting). Of several, the largest. Matching Manual.
⋯ edits it.
- **Not the user's, so it closes as before:** a pane a socket client filled, a window the app
  opened itself (`Adopt`), a window no slot holds (rule 4: never a candidate, so it would wait for
  good), and every app pane with `slots` off. The user's own × or close key on the
  pane is the user saying no: the pane goes, as ever.
- **As easy to be rid of as to get:** × in its header, and ⋯'s Clear slot. Nothing moves or takes
  focus.
- **Drawn (the owner, 2026-11-23):** a dashed outline inset in the pane, the app's icon (dimmed),
  `Waiting for a Zoom window`, the rule in the dialog's words, `Offers it here when one opens` (or
  Takes); when every window it selects is in another pane, `Its window is already in another pane`
  (`Model::slot_note`), so it does not read as broken; no keys and no ⌘E hint (⌘E still works). Offering: a solid accent outline, the window's
  title and one button, Put it here; Not this one and `N more match` stay as quiet words under it.
  A small pane: the icon and one line (and Put it here while it offers). The icon is kept by bundle
  id (`slot_icon.rs`, `<home>/icons`, 64 at most, read again when the app's bundle is newer); the
  core holds only the bundle id, and the icon is never in the state.
- The model keeps the panes that are not the user's (`not_placed`, `#[serde(skip)]`), set after
  each step from who put a window in a pane. The property `check`: a standard window's going never
  closes the user's pane.

This changes §3's Rejected "Close the pane when the window closes, as app panes do" from slots only
to every pane the user filled. Pending the owner's look.

### ⋯ is a menu (the owner, 2026-11-23)

The owner found Keep only when told: ⋯ opened the dialog straight away, its name said nothing. ⋯
now opens the pane's menu (the row menu, `MenuOf::Pane`, the user's alone: ↓ ↑ ↵, Esc), in the
owner's words: on an app pane `Create slot…` (the dialog); on a slot `Edit slot…` (the dialog), `Offer
again` (while windows were passed over), `Give back the window` (while it holds one) and `Clear slot`. No other pane act is there: ×, `fit` and the robot
sit beside it. Pin is left out: the Apps section pins an installed app by its path, which a pane
does not know.

## 3. Lifecycle

| State | Pane shows (mock) | Leaves it |
|---|---|---|
| **Absent**: the app is not running | Header `Slot · Zoom · …`. "Zoom is not running", and with `Launch` "Opening Zoom…" | The app starts → Empty |
| **Empty**: running, nothing matches | "Waiting for a Meetings window titled “Incident…”" (or "Zoom's `zm.meeting.window.main` window"), `⌘E Put something else here` | A candidate → Offered (Manual), or Filled (Automatic, rule 2) |
| **Offered** | "“Incident review · 14:00” is open", `Bring it here ↵`, `Not this one`, `2 more match` | Bring it here → Filled. Not this one → the next candidate, or Empty. The window closes or fills another pane → the next, or Empty |
| **Filled** | An ordinary app pane, a small `slot` mark in its header | Its window closes or stops being listed → Away. The app quits → Absent. The user clears it → an app pane |
| **Away**: its window went while its app runs | As Empty, "Zoom's window is away" | The same app's matching window comes back → Filled, without an offer. Away ends with no return → Empty, or the pane closes (`Close the pane`) |

- **Manual is the default:** a slot offers, it never takes. A window is never moved into a pane
  while the user is in a call in it.
- **Automatic** takes at once, but only on the tab on show, and only a window first listed by a
  look after the slot became Empty in this run. A window already open at the first look (after a
  start, a restore, or the tab showing) is offered: the user may be in it. So is the focused window
  of the frontmost app when that app is not ottospace (the look carries the frontmost pid and
  window id).
- **Only the tab on show looks.** A slot on a hidden tab stays as it was until its tab shows.
  Filling a hidden pane parks the window: a window taken off the screen for a tab the user is not
  looking at is the pull rule 2 forbids.
- **`2 more match`** counts the other candidates; a click (`SlotNext`) offers the next in `pick`'s
  order.
- **`Not this one`** (`SlotDecline`) remembers that window, by (pid, window id), for that slot and
  this run only. Forgotten when the window closes, when the slot is edited, and by ⋯ ▸ `Look Again`.
  Not by title: a recurring meeting keeps its title, and one decline would hide it for good. Not
  kept across a restart: window ids start again, and a remembered id could hide another window.
- **⌘E Put something else here** opens the picker into that pane; choosing ends the slot (the
  user's act; the pane is then what was picked). Built in the core's `PickerChoose`: a client's pick
  opens beside the slot. A window of the slot's own app is not something else: picked, it fills the
  slot, which stays one (Zoom picked into a Zoom slot, then quit, leaves it Absent, not closed).

### Away: a window that goes and comes back

Zoom's main window is not listed for the whole of a share the user gives (zoom-ax.md), minutes,
not seconds. Whether Zoom closes it or only orders it out, and whether it comes back with the same
window id, is not measured: slice 4 measures it, and the design holds either way.

- **Into Away**: `WindowGone { pane, app_running, at }` (System). Today the shell's `drop_closed`
  sends `Close` once the frame is unreadable and the window server no longer has the window; it
  sends `WindowGone` instead, and the core empties a slot (any other pane closes, as now). A window
  that still exists but is no longer listed by AX (ordered out) is gone to the slot too: the look
  says so, and the shell stops its stream.
- **How long Away lasts**: while the app has a window that its `transcribe` rule's `meeting`
  selectors select (the built-in Zoom rule names `zoom share toolbar window`, there while sharing),
  plus 10 s; for an app with no `meeting`, 10 s. The core times it from `at` and `SlotTick { at }`,
  sent every 2 s while a slot is Away, as transcripts' `ListenTick`. The `meeting` selectors are
  read from the rules whether or not the `transcripts` flag is on: they describe the app, not a
  transcript.
- **A return** is a window of the same pid that the slot selects, seen while Away: it fills the slot
  without an offer, Manual too (open question 1). Any other window is a candidate as in Empty.
- **The window leaves without closing** (the user's × on the pane, ⋯ ▸ `Give Back the Window`, the
  tab closed, ottospace quit): it goes back to the frame it had before it was hosted, as every
  hosted window does (`give_back.rs`). `Give Back the Window` empties the slot and passes over that
  window, or an Automatic slot would take it straight back.
- **As built: a new window goes to a slot before it is offered.** Since pop-ups no longer split
  (hosting-windows.md), a new window of a hosted app floats and the shell sends
  `WindowOffered { pane, seen, why }` (System) with its facts, before anything is hosted. The core
  gives it to the first slot on show that waits and selects it: Away's return asks the shell to host
  it (`HostSlot`), any other is offered in the slot; else the app pane's chip offers it. The core
  says which (`Effect::Placed`), and the log's `place` line is written from that: `slot_return`,
  `slot_offer` (with the slot) or `offer`. A slot that waits with no other pane of its app learns of
  new windows from its looks (every 2 s).
- **A new window of a hosted app** was adopted as a pane (`adopt_new_windows`, every tick, for
  each app in `known`), which would beat the slot's 2 s look and make the window no candidate for
  good. So adoption asks the core first: `Adopt` carries the new window's facts, and the core either
  gives it to a slot on the tab on show (offer or return, as above) or opens it as a new pane
  as now. One action, no race. An Automatic slot takes it at its next look, which says whether the
  user is in it (`frontmost`); `Adopt` does not.
- **A transcript of the slot's app** follows transcripts §1b: for an app whose rule names `meeting`,
  the meeting decides, so Away does not stop it, and a return is the same source. For any other
  app, the window going stops it, as a pane closing does today; `Give Back the Window` is
  unhosting, and stops it.

Rejected:

- **Automatic as the default.** A slot made before a call would move the call's window the moment
  it opens. Taking is one setting away.
- **Close the pane when the window closes, as app panes do.** The point of a slot is the layout
  keeping its shape between windows.
- **A fixed 10 s grace for every app.** It would not cover a Zoom share, and `Close the pane` would
  close the slot mid-share.
- **Automatic that never takes a window on screen.** Every new meeting window is on screen when it
  opens, so it would never take anything. "Opened while the slot waited" is the line instead.

### Several slots, one window

A window is offered or taken by one slot at a time, and slots of one app do not starve each other
(the owner, 2026-11-24: slot A by the title both Zoom meeting windows have held the one window that
slot B, by id, could take, while the other window sat unhosted). Each look pairs the waiting slots
of an app on the tab on show at once (`slot::pair`, `Model::slot_pairs`):

1. Slots that name their window (by title or id, or a pick of the smallest or a display), Away ones
   first, then in reading order: as many served as can be. One with a second choice gives up its
   first to one with none (Kuhn's augmenting step); a free window in its own order comes before a
   moved one, and what it offers now stays first.
2. The rest, app-only slots (any window, or the largest), pair by size, as the owner put it:
   "biggest pane gets biggest window, next biggest pane gets next biggest window". Panes by area,
   windows by the area their app gave them (`Model::slot_natural`, read while the window is in no
   pane: hosted, it has its pane's size); ties front to back, then in reading order.

A window in a pane is never moved by this: offers pair again as windows come and go, filled slots
keep theirs. The user moves them: ⋯'s `Swap` (two slots of the app) or `Swap with “…”` (each other
slot, by what it holds; with the empty slot the window moves and the other waits) and `Re-sort Zoom
windows by size` (the windows those slots hold, paired again as above). Only the user's; the slots'
rules stay. Rejected: a key and a Rotate (two swaps cover three slots; fewer items).

A slot that waits with nothing to offer says why, per window of its app, never a title
(`Model::slot_misses`): `2 Zoom windows: 1 is in another pane; 1 has no window id`, and the log has
the same as codes (`slot_miss`: `rules`, `no_id`, `other_id`, `other_title`, `other_kind`,
`other_display`, `in_pane`, `passed_over`, `bigger_pane`, `other_slot`), one line each change.

## 4. Persistence

- **The record is an app pane's record with a `slot` key** (`RecordKind::App`, whose `extra` keeps
  keys a build does not know), in `layout.json`, workbooks and marks alike:

  ```json
  { "kind": { "type": "app", "bundle": "us.zoom.xos", "name": "zoom.us", "title": "",
      "slot": { "select": { "identifier": "zm.meeting.window.main" },
                "pick": "largest", "matching": "manual", "absent": "wait", "closed": "keep" } } }
  ```

  `select` holds no `app`: the record's `bundle` and `name` are its app. An older build reads a plain
  app pane that asks for any window of the app. A build before slice 2 drops `slot` on its next
  write (it wrote no `extra`); from slice 2 a build with the flag off, or one that cannot read the
  key, keeps it as read and writes it back, so turning the flag off and on loses nothing.
- **Rejected: a new `RecordKind::Slot`.** `layout.json` reads an unknown kind as `empty` and the next
  write saves it so, losing the slot; `marks.json` refuses the unknown variant and loses every mark
  for the run.
- **A title in `layout.json`.** The file keeps app panes "by bundle id and name and never a title"
  (`layout_file.rs` tests it). A slot's `select.title` is the exception: the user's own pattern,
  cut down from the title in the dialog (§2; the dialog fills it in only when no identifier does the
  work). Kept 0600, as the terminal text the file already holds; never in the state. Slice 2 adds the
  exception to that test.
- **On restore every slot starts Absent or Empty**, never filled by the old window id, and matches
  again: Manual offers; Automatic offers too, since every window is open at the first look (§3). The
  window went back where it came from at quit, so a restart that offered nothing would leave the
  user to find it.
- **Beside restored app panes.** `rebuild_tab` puts every restored app pane in `Unfilled` and sends
  `HostWindow` for each at once, and `pick_window` falls back to the app's main window: Zoom's
  meeting window. So a slot's record is not put in `Unfilled` (its pane is a slot, `Kind::Slot`,
  §6), and a plain restored app pane of an app that a slot on the same tab names waits for that
  app's first look; the core then marks the windows slots select (`marks::Window` gains `slot: bool`,
  set by the core) and `pick_window` passes over them.
- **"Running with no window on this Space"** (#187): an app runs but AX lists no window of it here
  (minimised, another Space, closed). A slot shows Empty, "Waiting for …": true, since a window on
  another Space is not listed and is not offered. Absent only when the app does not run (`ax::running`).

## 5. Launch or wait

`Wait` (default): Absent until the app starts. `Launch it`: when the slot's tab shows and the app
does not run, the core asks once per show (`LaunchApp`, `open -g` by bundle id through Launch
Services, as the `apps` setting does; never a path from the file). Its window then follows the
slot's matching. Never in a test run (`OTTOSPACE_E2E`) except under `OTTOSPACE_HOST_ONLY` (the
canary). `restore_apps` does not gate it: the user set `Launch` on this slot.

Rejected: launching every slot's app at start (a restore that starts apps for tabs not on show),
and Launch as the default (Zoom started because a tab was opened).

## 6. Who may, and the socket

A slot not filled is its own pane kind, `Kind::Slot`, not `Kind::Empty`: today `Open` (a client's
`open_app` or `picker`) fills the pane it opens beside when that pane is empty, and would fill a
slot. On a slot, `Open` splits instead. A filled slot is `Kind::App`, with the slot kept beside it
in the core.

| Action | Origin | |
|---|---|---|
| `SlotMake`, `SlotEdit`, `SlotClear`, `SlotAccept`, `SlotDecline`, `SlotNext`, `SlotLookAgain`, `SlotGiveBack` | User | Refused from Socket and System (`Model::refusal`) |
| `SlotWindows { app, windows, displays, frontmost, at }` (what a look found), `SlotTick`, `TreeProbed`, `WindowGone`, `Adopt` with facts | System | Refused from Socket |
| `Close`, `Swap`, `MovePane`, `DropPane` of a slot pane | User (and System's `Close` for `Close the pane`) | Refused from Socket: a client does not take a slot out of the user's layout, nor move it to a hidden tab |

A client can still close, move or swap ordinary panes, `open_app` into a new pane (that window is
then in a pane and no slot's candidate), and read the state.

**The state.** A slot not filled: `kind: { "type": "slot", "name": "zoom.us", "bundle": ...,
"status": "absent" | "empty" | "offered" | "away", "matches": 2, "matching": "manual" }`. Filled: the
app pane's kind as today, plus `"slot": { "matching": "manual" }`. Never the selector, a candidate's
title or identifier: the core keeps those `#[serde(skip)]`. `sdk/src/types.ts` and `parse.ts`
follow; `skill/guide.md` says a slot is the user's and a client cannot fill it.

**Looks.** The core says which apps to look at (`WatchSlots { apps }`, when the set changes): those
of slots Absent, Empty, Offered or Away on the tab on show. The shell reads their windows' facts
every 2 s and sends `SlotWindows` when they change. No look with no such slot.

**Test-only** (`control::test_refusal`, documented in `CLAUDE.md` and on `Request`, never in the
guide): `slot_windows` (a fixture of what a look finds: a test run hosts no window) and `slot_make`
(the dialog's Keep, as the user's act, as `repo_add` is). `"as":"user"` clicks drive the rest.

## 7. Tests

- **Core unit:** the selector (each field, the glob, a misspelt key refused, `NeedsTree`); matching
  order (rules first, a window in a pane never a candidate); each `pick` and display numbering;
  every transition of §3; Manual never fills from a look; Automatic offers the frontmost window and
  windows open at the first look, and takes only on the tab on show; decline remembered by id,
  forgotten on close, edit and Look Again, not by title; Away held by a `meeting` window, then 10 s,
  timed by `SlotTick`; the return; `Adopt` given to a slot, not a new pane; several slots and one
  window; restore starts empty and offers; a slot record is not `Unfilled`, and a plain restored pane
  of the same app waits for the first look and passes over a slot's window; an older build's record
  (no `slot` key) and a record with one round-trip; `Open` beside a slot splits; every refusal of §6.
- **Property** (`model/tests/prop.rs`): a maker for each action in `MAKERS`, from every origin. In
  `check`: a window is in at most one pane and offered by at most one slot; an offered window is in
  no pane; a declined window is not offered in that slot; no Socket action changes a slot or fills
  one; a filled slot's pane holds a window of its app; the serialized state never holds a title or
  identifier the fixture facts carried.
- **e2e** (`cargo e2e`, `OTTOSPACE_FEATURES=slots`): `slot_make` and `slot_windows` drive the three
  unfilled states and their drawing (pictures for the PR); `Bring it here` by `"as":"user"`;
  a client's `close`, `swap`, `move_pane` and `open_app` into a slot refused or split; the reply's
  JSON holds none of the fixture's titles.
- **e2e-apps** (`cargo e2e-apps`, in CI's `app-windows.yml` only, not locally: it takes the
  keyboard): the canary's `open_window` gains `title` and `identifier` (`NSWindow.identifier`, which
  AppKit gives as `AXIdentifier`; to confirm in slice 4). Two windows titled `Canary Meeting`, one
  with `canary.main`: an identifier slot is offered only that one; without an identifier, `largest`
  picks the larger; Not this one offers the other; a new window of the hosted canary that the slot
  selects goes to the slot, not a new pane; close the filled window, the pane stays (Away, then
  Empty); open it again, it is offered; × gives the window back to its first frame; a restart on a
  private `OTTOSPACE_HOME` brings the slot back empty and offers again.
- **Measured by hand in slice 4** (structure only, nothing pressed): whether Zoom closes its main
  window or orders it out while the user shares, and whether it comes back with the same window id;
  whether `zm.meeting.window.main` is there in the Zoom version of the day.

## 8. Slices

Each a stacked, draft PR, behind a `slots` feature flag (`features.rs`) from slice 2.

1. **The selector.** `Selector` and `Facts.identifier` in `ottospace-core::rules`; `windows` rules
   take `identifier`; `ax::Window` reads it for window facts. Transcripts slice 4 builds its
   `meeting` on this, evaluated in the core (whichever lands first adds `identifier`). Evidence:
   `cargo test` output.
2. **Slots in the core.** `Kind::Slot`, the record's `slot` key, the states with Away, the actions
   and refusals, decline, several slots, restore and `pick_window`'s pass-over, `Adopt` with facts;
   property makers. No UI. Evidence: `cargo test` output.
3. **Slots on screen.** ⋯ on app pane headers, the dialog, the unfilled panes, the `slot` mark,
   looks, `WindowGone`, `Adopt` asking the core, the state, SDK, test-only requests, e2e. Evidence:
   before and after pictures from `scripts/shot`.
4. **Real windows.** The canary's titles and identifiers; e2e-apps above; restart; the Zoom
   measurements of §7.
5. **Launch and Close the pane**, with Away's `meeting` hold, `SlotTick` and the return.
6. **`has`.** `ProbeTree`, the cache, the bounded read; `windows` and `meeting` accept it. Only when
   an app needs it.

## Owner's decisions (2026-11-17)

Every recommendation taken:

1. A window coming back while Away (Zoom after a share) goes back in without asking, Manual too.
2. No named selectors in v1: each place writes its own, the same JSON.
3. Displays by number (1 has the menu bar, then left to right).
4. Automatic never fills a slot on a tab that is not on show: it waits until the tab is.
5. The dialog shows the window id, as "the app's own name for this window".
6. Choosing something else with ⌘E in an empty slot ends the slot.
