# Pick on screen, the extended accessibility mode, and app profiles

A design. 2026-11-18, the direction approved by the owner. Not built.

The user picks things in panes as a task's context. To find "the thing" under a click, ottospace
reads the app's accessibility tree with the app's fuller mode on, and falls back to the picture.
What differs between apps lives in an **app profile**: data, as a runner is its `RunnerSpec`.

**Supersedes**, once slice 1 lands: the "no accessibility mode" lines in
[capture.md](capture.md), [sneak-apps.md](sneak-apps.md) ("What is not allowed", Open questions),
[agent-mode.md](agent-mode.md) (Limits) and [sdk.md](sdk.md); capture.md's "runs no OCR" from
slice 4, for picks only. Until then those docs describe the build. [backlog.md](backlog.md)'s
idea of setting the mode while an app is hosted is replaced by §3.

## The spike

2026-11-18, macOS, Apple M4 Pro. Throwaway tools in a temp folder, not in the repository:
`axprobe.swift` (AX hit tests, mode writes, Vision), `seg` (recursive XY-cut over the picture,
Rust), `run.py` (scores both). Overlays of every run:
https://gist.github.com/bryan-uipath/4ec1638133a070f05a36f4948696087d

For each point a box was expected (for pages, the nearest message, paragraph, row, heading,
control, ... element over the point, read over CDP). A pick is right when it overlaps that box, or
that box trimmed to its ink, by 60% or more (IoU), or picks nothing on a blank point. "Either"
counts a point once either method is right: an upper bound, known only with the expected box.

- **Tree** (B): the element at the point (`AXUIElementCopyElementAtPosition`), then the box rules
  below. The walk by frame that §2 asks for was not scored: its accuracy is unmeasured too.
- **Picture** (A): XY-cut on a grey image at half resolution. The deepest box at the point, then
  up while the parent's cut is 5 pixels of that image (10 of the capture) or less, stopping at a
  box with its own fill (a bubble, a selected row). Nothing if the deepest box still has a wider
  cut inside, or the point is two levels deep or less. Vision's lines inside it give its text.

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

Chrome rows are with the mode on. Its "off" runs scored within one (15, 8, 16, 15), although the
script wrote off and loaded the page again first. Unexplained: the tree may stay after off, or the
hit test may wake it. In the cost run (`chrome_off1.json`) off dropped the tree from 96 to 41
nodes by the first sample, 0.1 s.

Times (release build of `seg`, whole window):

| What | Time |
|---|---|
| XY-cut | 1.0–22.0 ms |
| Vision fast | 40.4–81.7 ms (once 353.3 ms, VS Code off) |
| Vision accurate | 208.6–305.2 ms |
| AX hit test, first of a run | 14.5–34.1 ms |
| AX hit test, after | mostly 0.1–1 ms, at most 8.6 ms |
| Writing the mode attribute | 16.5–22.8 ms |
| The tree filling after the write | 1.96–2.23 s: the last sample still empty came at 1.96–2.07 s, the first full one at 2.14–2.23 s (Chrome 41 → 96 nodes, three runs; VS Code 13 → 260) |
| Extra CPU over the 3–4 s watch, the probe's polling included | Chrome on 0.18–0.30 s, Chrome off 0.22 s, VS Code on 0.24 s |

What the tree got wrong, for the rules and fixtures of slice 2:

- Plain-div chat: a line or the name, not the message (too small or too big); the time too big;
  avatar and one message missed; the emoji missed (off) or too small (on); blank space right of
  the messages missed. The tree's box is wrong, not empty. With list roles all right but the code
  block (too big) and that blank space (off).
- Images are skipped (`AXImage`): the article's figure missed; so is the quote's bar.
- Table: the header cell gives its whole row (too big): "a cell gives its row" must not take a
  header.
- Finder's name cell gives its `AXTextField`, not the row; its sidebar's blank space gives a row;
  the column header and status text give the whole split group; the path bar item missed.
- TextEdit: the title bar's name missed (cropped from a pane anyway).
- VS Code with the mode on: Monaco's code lines and gutter missed; the outline header too small;
  blank panel space gives a group.
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
  materials. Kept with it: the picture, its frame in the pane, the element's role and path.
  Fallback picks are marked **approximate** on the chip and in the brief.
- **Terminals and the chat** read their own text: rows, messages. No tree, no OCR.
- **Who.** Only the user's own clicks pick. The core refuses starting, hovering, taking and ending
  from `Socket` and `System` (`Model::refusal`); the property test holds it. The shell's answers
  (a chain read, a picture) come as `System` and count only for the current hover.
- **Capture's settings hold**: `keep_images: false` keeps no picture, `ax: false` reads no tree
  (picks are then the picture's), a window the rules ignore is never picked. A picture-only pick
  has no role or path. Pick text never goes in the state:
  chips there carry kind, frame and `approximate`, no text.

## 2. Finding the thing

**Primary: the tree, with the extended mode on.** A walk down from the window by frame, never
`AXUIElementCopyElementAtPosition`. The shell walks only the branches whose frame holds the point,
within the reader's limits (depth 40, 3000 nodes, 0.5 s a call), off the main thread, the latest
hover wins. The spike's `find` keeps the deepest path, then the smallest frame; overlapping
siblings (what is drawn on top) are not decided by it. It hands the core the chain from the window
to the deepest node. The walk was not scored and its time not saved: slice 2 measures both.

**Box rules** (`ottospace_core::pick`, pure), over the chain:

1. Skip text runs (`AXStaticText`, `AXInlineTextBox`), `AXImage`, `AXUnknown`, and the window,
   web area and scroll area. Skip a frame under 16×12 points. (Skipping images loses figures:
   slice 2 decides.)
2. A cell gives its row; a header cell does not.
3. A list, table or outline item beats its parts (up to 4 levels up).
4. Controls stay as they are (button, link, field, checkbox, heading, ...).
5. A big text area (`AXTextArea`, value over 200 characters) gives the paragraph under the point:
   the shell asks `AXRangeForPosition`, the core cuts the paragraph from the value, the shell asks
   `AXBoundsForRange` for its box.
6. A container's own blank space (the deepest node is a container with children, not an item)
   gives nothing.

**Fallback (A)**: XY-cut on the pane's picture, then Vision in fast mode on the chosen box only.
Used while the tree fills, or where it has nothing: Monaco, canvases (per profile, §5). On
plain-div pages the tree gives a box of the wrong size, not none: ↑/↓ fixes it by hand; when to
prefer the picture's box there is open. XY-cut is pure and goes in the core; Vision is in `ottospace-mac`.

## 3. The extended accessibility mode

- **On only on demand**, per app: while the user picks, while sneak mode is on, and while an agent
  with a grant ([agent-mode.md](agent-mode.md)) interacts with one of the app's panes. The core
  keeps the reasons per app; the mode goes off when none is left.
- **Only what ottospace turned on.** Already on (VoiceOver, another tool): ottospace never writes
  it, on or off, not even for a move. The read cannot say so alone: VS Code read `0` back right
  after a write that worked (`vscode_on1.json`), Chrome read nothing. Proposed: "already on" is the
  attribute reading on, or the tree already full before any write; unknown counts as already on.
  Each write is tied to that app's process: a relaunched app is a new one.
- **The attribute** is the profile's: Electron `AXManualAccessibility`, Chromium
  `AXEnhancedUserInterface`. Chrome 157.0.8103.80 rejects the Electron one (`-25205`, attribute
  unsupported; its tree stayed at 41 nodes).
- **The return code cannot be trusted.** Chrome answers `-25208` (not implemented) to
  `AXEnhancedUserInterface` and fills its tree anyway. Success is checked by watching the tree
  fill (its node count grows, then holds), up to the profile's settle time (proposed 2.3 s, past
  the spike's 1.96–2.23 s). Picks before that use the fallback. A full count is not the end:
  twice the first hit missed later than that (§ The spike).
- **Moves.** A profile with `animates_moves` (Chrome) has the mode turned off before ottospace
  parks or moves its window, and on again after if a reason holds; only when ottospace turned it
  on. To re-verify (the spike).
- **Off may not empty the tree** (unexplained, above). Nothing may depend on either.
- **A crash** must not leave it on: what ottospace turned on (app, pid) goes in the hosted-window
  record, and the guardian and the next start turn off those still running with that pid.
  Proposed.
- **Who adds a reason.** The user's own Pick and sneak entry (`Origin::User`); a socket client's
  `sneak` adds none. An agent's request only on a pane granted to its task; the reason goes when
  the grant does (level changed, pane moved, task ended), or after it is idle (proposed 60 s).
  Tree reads and fill news come back as `System` actions, taken only for the mode and pane still
  current.

## 4. The pane-header icon

- While ottospace has the mode on for an app, a small accessibility icon shows in the header of
  every pane of that app (the mode is per app), beside the agent robot.
- **Tooltip**: why and until when. "Accessibility on for Chrome: you are picking, until Pick mode
  ends", "... sneak mode, until it ends", "... <task>'s agent, until 60 s after its last request"
  (proposed time).
- **Click** turns it off now, and the reasons with it (an agent's reads then fall back to what the
  plain tree gives). Only the user's click. It stays off for that app until the user's own next
  Pick or sneak entry; an agent's request does not turn it back on (proposed).
- **Already on** (not ottospace's): the icon shows "already on", the click does nothing.
- **State**: `ax_mode` per app: the app, `by` (`ottospace` or `already`), its reasons. No text of
  the app.

## 5. App profiles

A runner is its spec; an app is its profile. The core asks a profile by capability, never matches
an app's name.

```json
{
  "match": { "app": "com.google.Chrome" },
  "mac": {
    "mode": { "attribute": "AXEnhancedUserInterface", "settle_ms": 2300,
              "animates_moves": true },
    "roles": { "message": [{ "role": "AXGroup", "parent": "AXList" }],
               "row": [{ "role": "AXRow" }], "paragraph": [{ "role": "AXTextArea" }] },
    "segment": "tree"
  }
}
```

- **Match**: `app` (name or bundle id, as the window selector's), or `family`, detected from the bundle: Electron (`Electron
  Framework.framework`), Chromium (a Chromium framework), Catalyst, Java, else AppKit. The platform
  crate reads the bundle's facts; the core decides the family.
- **Mode**: the attribute, settle time, quirks (`animates_moves`; `keeps_tree_after_off` once
  it is reproduced). None for AppKit.
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
   - Core: matching (app, family, user before built-in), reasons and off-when-empty, never a write
     when already on or unknown (a move included), off before a move with `animates_moves`, a
     socket `sneak` adds no reason; a property check that the mode is on only while a reason holds
     and that ottospace writes off only what it wrote on.
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
   - Core: an agent's request on a granted pane adds the reason; idle, a changed grant or an
     ended task removes it; no grant, no reason; after the user's click, no request turns it on.
   - e2e: agent-mode's fixtures, the icon's tooltip names the task.
   - e2e-apps: the canary's mode on while a granted read runs.

## Open

- The walk's time per hover, and whether a cached tree beats it.
- Settle detection when the tree is already full (Chrome keeping it).
- Teams' family (its web view) and Zoom's need of a mode.
- Windows: whether UI Automation needs a mode at all.
