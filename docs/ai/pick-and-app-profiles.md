# Pick on screen, the extended accessibility mode, and app profiles

A design. 2026-11-18, the direction approved by the owner. Not built.

The user picks things in panes as a task's context. To find "the thing" under a click, ottospace
reads the app's accessibility tree with the app's fuller mode on, and falls back to the picture.
What differs between apps lives in an **app profile**: data, as a runner is its `RunnerSpec`.

**Supersedes**, once slice 1 lands: the lines in [capture.md](capture.md) ("does not switch an
app's accessibility mode on", "runs no OCR") and in [sneak-apps.md](sneak-apps.md) ("What is not
allowed", fuller accessibility modes). Until then those docs describe the build.

## The spike

2026-11-18, macOS, Apple M4 Pro. Throwaway tools in a temp folder, not in the repository:
`axprobe.swift` (AX hit tests, mode writes, Vision), `seg` (recursive XY-cut over the picture,
Rust), `run.py` (scores both). Overlays of every run:
https://gist.github.com/bryan-uipath/4ec1638133a070f05a36f4948696087d

For each point a box was expected (for pages, the DOM element under it, read over CDP). A pick is
right when it overlaps that box by 60% or more (IoU), or picks nothing on a blank point.

- **Tree** (B): the element at the point (`AXUIElementCopyElementAtPosition`), then the box rules
  below.
- **Picture** (A): the XY-cut box at the point, climbing while the parent's cut is 5 half-pixels
  or less; Vision's lines inside it give its text.

| Subject (made-up content) | Points | Tree | Picture | Either |
|---|---|---|---|---|
| Chrome, article | 18 | 15 | 16 | 17 |
| Chrome, chat of plain divs | 18 | 8 | 11 | 14 |
| Chrome, the same chat with `role=list`/`listitem` | 18 | 17 | 11 | 18 |
| Chrome, table | 16 | 14 | 3 | 14 |
| Finder, list view | 16 | 11 | 2 | 12 |
| TextEdit (paragraph rule) | 13 | 12 | 6 | 12 |
| VS Code, mode off | 17 | 0 | 7 | 7 |
| VS Code, mode on (`AXManualAccessibility`) | 17 | 12 | 7 | 13 |

Chrome rows are with the mode on. Its "off" runs scored the same (15, 8, 16, 15): the tree from an
earlier on stayed, so they are no clean off.

Times (release build of `seg`, whole window):

| What | Time |
|---|---|
| XY-cut | 1.0–22.0 ms |
| Vision fast | 40.4–81.7 ms (once 353.3 ms, VS Code off) |
| Vision accurate | 208.6–305.2 ms |
| AX hit test, first of a run | 14.5–34.1 ms |
| AX hit test, after | mostly 0.1–1 ms, at most 8.6 ms |
| Writing the mode attribute | 16.5–22.8 ms |
| The tree filling after the write | 1.96–2.23 s (Chrome 41 → 96 nodes, three runs; VS Code 13 → 260) |
| Extra CPU while it fills | Chrome 0.18–0.30 s, VS Code 0.24 s |

What the tree got wrong, for the rules and fixtures of slice 2:

- Plain-div chat: a line or the name, not the message (too small or too big); avatar, emoji and
  one message missed. With list roles, right.
- Images are skipped (`AXImage`): the article's figure missed.
- Finder's name cell gives its `AXTextField`, not the row; its sidebar's blank space gives a row;
  the column header and status text give the whole split group.
- VS Code with the mode on: Monaco's code lines and gutter missed; blank panel space gives a group.
- Twice the first hit after the mode went on missed (Chrome article `nav-gears`, table `input`),
  at least 2.5 s after the write.

Reported by the spike, not in its saved files, **to re-verify in slice 1**:

- `AXUIElementCopyElementAtPosition` returns the menu bar for a parked window. The spike's `find`
  (a walk down from the window by frame) works there.
- With Chrome's mode on, accessibility-API window moves animate for about 300 ms.

## 1. Pick on screen

- **Start.** Pick adds a **+ menu** to the new-task box; its item **Pick on screen** starts the
  mode. The box stays where it is, open in the drawer.
- **Shown.** A ring round ottospace's window, a pill "Pick mode · Esc to stop", a crosshair
  cursor. Live app panes are parked: a pick is drawn over the pane's picture, as a capture is, and
  no click reaches the app.
- **Hover** outlines what a click takes. **↑/↓** widen or narrow it through its ancestors (the
  tree's, or the XY-cut's in the fallback).
- **Click** takes it. **⇧-click** takes several. **Drag** draws a free region: the nodes under it
  (`capture::nodes_under`) or the rows under it (`lines_under`).
- **Esc or ↵** ends the mode.
- **Each pick is a chip** in the box. Into the brief go its text and a small picture, fenced as
  untrusted exactly as capture's blocks ([capture.md](capture.md), Trust), stored with the
  materials. Always kept: the picture, its frame in the pane, the element's role and path.
  Fallback picks are marked **approximate** on the chip and in the brief.
- **Terminals and the chat** read their own text: rows, messages. No tree, no OCR.
- **Who.** Only the user's own clicks pick. The core refuses every pick action from `Socket` and
  `System` (`Model::refusal`); the property test holds it. Pick text never goes in the state:
  chips there carry kind, frame and `approximate`, no text.

## 2. Finding the thing

**Primary: the tree, with the extended mode on.** A walk down from the window by frame, never
`AXUIElementCopyElementAtPosition`. The shell walks only the branches whose frame holds the point,
children last first (what is drawn on top wins), within the reader's limits (depth 40, 3000 nodes,
0.5 s a call), off the main thread, the latest hover wins. It hands the core the chain from the
window to the deepest node. The walk's own time was not saved by the spike: slice 2 measures it.

**Box rules** (`ottospace_core::pick`, pure), over the chain:

1. Skip text runs (`AXStaticText`, `AXInlineTextBox`) and the window, web area and scroll area.
   Skip a frame under 16×12 points.
2. A cell gives its row.
3. A list, table or outline item beats its parts (up to 4 levels up).
4. Controls stay as they are (button, link, field, checkbox, heading, ...).
5. A big text area (`AXTextArea`, value over 200 characters) gives the paragraph under the point:
   the shell asks `AXRangeForPosition`, the core cuts the paragraph from the value, the shell asks
   `AXBoundsForRange` for its box.
6. A container's own blank space (the deepest node is a container with children, not an item)
   gives nothing.

**Fallback (A)**: XY-cut on the pane's picture, then Vision in fast mode on the chosen box only.
Used while the tree fills, or where it has nothing: plain-div pages, Monaco, canvases (per
profile, §4). XY-cut is pure and goes in the core; Vision is in `ottospace-mac`.

## 3. The extended accessibility mode

- **On only on demand**, per app: while the user picks, while sneak mode is on, and while an agent
  with a grant ([agent-mode.md](agent-mode.md)) interacts with one of the app's panes. The core
  keeps the reasons per app; the mode goes off when none is left.
- **Only what ottospace turned on.** Read the attribute first. Already on (VoiceOver, another
  tool): ottospace never writes it, on or off.
- **The attribute** is the profile's: Electron `AXManualAccessibility`, Chromium
  `AXEnhancedUserInterface`. Chrome 157.0.8103.80 rejects the Electron one (`-25205`, attribute
  unsupported; its tree stayed at 41 nodes).
- **The return code cannot be trusted.** Chrome answers `-25208` (not implemented) to
  `AXEnhancedUserInterface` and fills its tree anyway. Success is checked by watching the tree
  fill (its node count grows, then holds), up to the profile's settle time (2.3 s from the spike's
  1.96–2.23 s). Picks before that use the fallback.
- **Moves.** A profile with `animates_moves` (Chrome) has the mode turned off before ottospace
  parks or moves its window, and on again after if a reason holds. To re-verify (the spike).
- **Off may not empty the tree** (Chrome kept it, above). Nothing may depend on either.
- **A crash** must not leave it on: the apps ottospace turned on are recorded beside `hosted.json`,
  and the next start (or `ottospace rescue`) turns off those still running. Proposed.

## 4. The pane-header icon

- While ottospace has the mode on for an app, a small accessibility icon shows in the header of
  every pane of that app (the mode is per app), beside the agent robot.
- **Tooltip**: why and until when. "Accessibility on for Chrome: you are picking, until Pick mode
  ends", "... sneak mode, until it ends", "... <task>'s agent, until 60 s after its last request"
  (proposed time).
- **Click** turns it off now, and the reasons with it (an agent's reads then fall back to what the
  plain tree gives). Only the user's click.
- **Already on** (not ottospace's): the icon shows "already on", the click does nothing.
- **State**: `ax_mode` per app: the app, `by` (`ottospace` or `already`), its reasons. No text of
  the app.

## 5. App profiles

A runner is its spec; an app is its profile. The core asks a profile by capability, never matches
an app's name.

```json
{
  "match": { "bundle": "com.google.Chrome" },
  "mac": {
    "mode": { "attribute": "AXEnhancedUserInterface", "settle_ms": 2300,
              "animates_moves": true, "keeps_tree_after_off": true },
    "roles": { "message": [{ "role": "AXGroup", "parent": "AXList" }],
               "row": [{ "role": "AXRow" }], "paragraph": [{ "role": "AXTextArea" }] },
    "segment": "tree"
  }
}
```

- **Match**: by bundle id, or by family, detected from the bundle: Electron (`Electron
  Framework.framework`), Chromium (a Chromium framework), Catalyst, Java, else AppKit. The platform
  crate reads the bundle's facts; the core decides the family.
- **Mode**: the attribute, settle time, quirks (`animates_moves`, `keeps_tree_after_off`). None
  for AppKit.
- **Roles to meanings**: which nodes are a message, a row, a paragraph, for the box rules and the
  brief.
- **Segmentation hints**: `tree`, `picture`, or regions that trust the picture (VS Code's editor).
- **Per platform**: `mac` (AX) now; `windows` (UI Automation) later, read by a sibling crate to
  `ottospace-mac`. The core's types do not name either API.
- **Built in**: the families, and Slack, Chrome, VS Code, Zoom, Teams. Measured: Chrome, VS Code
  (Electron). Slack takes the Electron family's; Zoom ([zoom-ax.md](1-initial-brainstorm/zoom-ax.md))
  and Teams get no mode write until measured.
- **User overrides** in `settings.json` (`app_profiles`), the user's before the built-in ones, as
  `windows` rules. `match` uses the window selector's glob (`rules::glob`).
- **Layering**: types, matching and the box rules in `ottospace-core`, pure, unit tested; reads,
  writes and Vision in the platform crate.

## 6. Slices

Each ships alone, behind a flag.

1. **Profiles, matching, the mode toggle and the header icon; sneak mode uses it.** Sneak mode turns
   the focused app's mode on and labels the fuller tree. Re-verify the parked hit test and Chrome's
   move animation.
   - Core: matching (bundle, family, user before built-in), reasons and off-when-empty, never off
     what was already on, off before a move with `animates_moves`; a property check that the mode
     is on only while a reason holds and only an off ottospace's own.
   - e2e: an `app_fixture` app with a test-only `ax_mode_fixture` (already on or not, a settle);
     the icon, its tooltip, the click, "already on", `ax_mode` in the state.
   - e2e-apps: the canary answers `AXManualAccessibility` and reports writes on its socket: on in
     sneak mode, off after, none when already on.
2. **The tree-walk picker core and the box rules.**
   - Core: `pick` on recorded fixture trees (the spike's subjects, recorded with the reader,
     made-up content, paths scrubbed), every row of the table above as a case, the misses listed
     as known; ↑/↓; the paragraph cut.
   - e2e: none yet (no UI).
   - e2e-apps: the walk on the canary, parked.
3. **Pick mode UI and chips.**
   - Core: the mode, refusals from `Socket` and `System`, chips, the brief's fenced blocks.
   - e2e: a test-only `pick_app_tree` (as `sneak_app_tree`), clicks with `"as":"user"`: hover,
     ↑/↓, ⇧-click, drag, Esc, ↵; a terminal pick; a client's click refused.
   - e2e-apps: a pick on the canary, with its picture.
4. **The A fallback.**
   - Core: XY-cut on recorded pictures (the spike's), the climb rule, `approximate`.
   - e2e: a fixture picture through the pick request.
   - e2e-apps: a pick during settle is approximate, then the tree's.
5. **Agents use the extended mode while they interact.**
   - Core: an agent's request on a granted pane adds the reason, 60 s idle removes it; no grant,
     no reason.
   - e2e: agent-mode's fixtures, the icon's tooltip names the task.
   - e2e-apps: the canary's mode on while a granted read runs.

## Open

- The walk's time per hover, and whether a cached tree beats it.
- Settle detection when the tree is already full (Chrome keeping it).
- Teams' family (its web view) and Zoom's need of a mode.
- Windows: whether UI Automation needs a mode at all.
