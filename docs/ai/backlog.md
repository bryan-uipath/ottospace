# Ideas

Backlog of features beyond the prototype. Not committed to; revisit after step 2.

North star: a super *agent* multiplexer. Agents run in terminal panes and work directly with the
app windows tiled beside them.

## 1. Agent skill: open panes with app windows

A skill (Claude Code / agent-facing) that drives ottospace: open a split or pane and host a given
app window in it, e.g. "open Safari on the right, 40%". Implies a control surface (CLI or local
socket) exposing layout commands: split, focus, close, move-window-into-pane.

## 2. Accessibility tree as a first-class surface

Each hosted window already has an AX handle (we use it for positioning). Expose its AX tree too:

- **Agents** — read structure/text/state of a pane's app (buttons, fields, selected text) and act on
  it via AX actions, without screenshots.
- **Keyboard navigation** — shortcuts that jump across a window's AX elements (hint labels like
  Vimium, or cycle through buttons/links/text fields).

- **Driving parked windows** — synthetic mouse/key events don't reach parked windows (Chrome ignores
  them), but AX actions (`AXPress`, set `AXValue`, `AXFocus`) don't depend on window position. Agents
  could act on a parked pane's app without swapping it live.

Open questions: AX tree size/latency on big apps (browsers, Electron); apps with poor AX support;
permission story is the same Accessibility grant we already need.

## 3. Typed splits with shortcuts

- **Terminal**: ⌘D split right, ⌘⇧D split down (iTerm convention).
- **Slack / browser**: same split gestures, but the new pane hosts an app window. Candidates:
  ⌘⌥D / ⌘⌥⇧D with a quick-pick of apps, or per-type chords (e.g. ⌘K S → Slack, ⌘K B → browser).
- Needs a real split tree (not today's flat row) and an embedded terminal (e.g. alacritty_terminal in GPUI).

## 4. Screen sharing (Zoom etc.)

- Sharing the **ottospace window** shows streamed panes (they're ottospace pixels) but not a pane in
  place mode (that's the real app's window stacked on top). Forward mode shares cleanly.
- Sharing the **whole screen** shows everything, including place-mode windows.
- Coexists with other capture sessions: SCK supports multiple clients. Parked windows show up in
  Zoom's window picker and can still be shared individually.
- Needs a real test.

## 5. Session restore

Designed in [session-restore.md](session-restore.md): layout first, then terminal content and
app windows, then per-app state. Today startup only gives leftover windows their original frames
back.

## 6. Routines and agents

ottobot's routines, an Agents section and a menu-bar item, in stages: see [routines.md](routines.md).

## 7. Fit a pane to its window by itself

An `auto_fit` setting, off by default. It applies `fit` (`Action::Fit`) at the moment an app is
hosted: for a window smaller than its pane, and for one larger where the tab has room. Never
later than hosting, so panes do not move under the user. With an undo.

## 8. Voice: speech to text and back

After the task and agent work. A hot key starts the microphone; speech becomes text where the
keyboard is (a terminal, the new-task box), transcribed on the machine (Whisper). The other way:
read an agent's reply or a task brief aloud.

- Push-to-talk on a global key, and a visible "listening" mark: never a microphone that is on
  without the user holding or toggling it.
- Local transcription first (whisper.cpp or similar, model fetched on request); say plainly if any
  mode sends audio off the machine.
- Needs the Microphone permission for whatever starts ottospace.
- Fits quick capture: hold the key, say the task, release, it is added.
- Open: which engine and model size; dictation into a hosted app's pane (keys go to that app);
  voices for reading aloud.

## 9. Text of a capture by OCR

A capture ([capture.md](capture.md)) keeps the picture and whatever text the app's accessibility
exposes. Electron and Chrome apps (Slack, VS Code) expose almost none. Decided for now (Bryan):
the annotated picture is the material, and agents read images themselves; ottospace does not
switch an app's accessibility mode on (that is a write into the app) and runs no OCR.

Later, optional: on-device OCR (macOS Vision) of a capture, so the text under a circle is
searchable and there for readers that cannot see images.

- On the machine only; say so, and never send a picture anywhere.
- The text is captured content like the rest: inside the brief's fenced blocks, marked untrusted.
- Keep the picture the first thing a reader is sent to.

## 10. Terminals and agents survive quitting, updating and reopening ottospace

Tasks come back as they were. Designed in [terminal-persistence.md](terminal-persistence.md): each
terminal, the drawer agent's included, runs in a small holder process outside the app, and the
app attaches again at start. Builds on [session-restore.md](session-restore.md) layer 1 (the
layout file names each terminal's session) and replaces nothing there: after a reboot, layer 2
(scrollback, directory) and agent resume still apply. Separate from workbooks
([workbook.md](workbook.md)), which are templates with no process in them.

## 11. User-configurable key bindings in settings.json

A `keys` map in `settings.json` (action name → chord, e.g. `"focus_left": "cmd-ctrl-h"`) that
replaces or adds to the default bindings. The menus and the ⌘/ list show the bindings in force;
a chord ottospace cannot parse, or one bound twice, is reported and the default kept.

## 12. Ask a hosted Electron app for its full accessibility tree

Measured on 2026-11-13 with no assistive client: Spotify exposes 17 elements and Slack 12, of
which 3 are pressable in each (the window's own buttons); Outlook exposes 372 or more. So sneak
labels, the inspector and captures get nothing from Electron and Chromium apps.

A screen reader gets the tree by setting an attribute on the app: `AXManualAccessibility`
(Electron: build the tree, change nothing else) or `AXEnhancedUserInterface` (what VoiceOver
sets; some apps then change window animation and resizing). ottospace sets neither.

Idea: while an app has a window in a pane, and only for apps the user opted in:

- On hosting: read the app's value, remember it, set `AXManualAccessibility`.
- On release of its last hosted window: put the remembered value back. The hosted-window record
  and the guardian do the same after a crash.
- Off by default. A list in `settings.json` and a toggle in the accessibility inspector; a mark
  on the pane header while it is on.
- Electron only at first. `AXEnhancedUserInterface` (Spotify and other Chromium apps) is left out.

To settle first: the setting is per app, not per window; whether Electron drops the tree again
when it is turned off is not verified; while it is on, the app's text (messages, names) is
readable by `ax_tree`, which a socket client may call, and by captures; the app pays CPU and
memory for it.
