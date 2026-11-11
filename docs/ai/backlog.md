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

Re-host the previous session's windows into panes on launch (reuse `recovery`'s hosted list; also
write it on clean quit). Today startup only restores leftover windows to their original frames.
