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
