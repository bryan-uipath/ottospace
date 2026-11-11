# Plan

Refines `docs/ideas/1-initial-brainstorm.md` (items cited as `#n`). Older AI backlog: `../backlog.md`.

## Model

```
Task              (a full workspace; #10)
└─ Tab
   └─ Split tree  (binary: axis + ratio)
      └─ Pane     (Terminal | App window | Brief | later: Files, Markdown, Diff)
```

Sidebar (#7) selects the task or workbook. Status bar (#9) shows mode and focus-block state.
The task brief is a pane like any other: split a terminal, explorer or app beside it.

## Phases

| Phase | Items | Gives |
|---|---|---|
| A. Shell | #0, #7c, #8 (splits), #9 | Split tree, terminal panes, keys, status bar |
| B. Control | backlog 1, #5 | Local socket + CLI. Agents and tests drive layout. Window rules |
| C. Navigation | #2, #7a-b, #8 (sneak) | App switcher, sidebar categories, AX hints |
| D. Persistence | #6, #4 | Session restore, per-app adapters, display affinity |
| E. Tasks | #10, #3 | Task = cwd + conversations + priority + resources. "Pop next". Quick capture. Lifecycle and cleanup |
| F. Focus | #1 | Focus blocks, usage metrics, breakthrough agent |

Order rationale: every later phase needs A. B makes the rest scriptable and testable. C is the
cheapest daily win. E is the largest value but needs A, B and D.

## Phase A: steps

1. **Split tree.** Done.
2. **Core + control socket + e2e.** Done. Pulled forward from phase B. `ottospace-core` holds
   state and rules; `ottospace ctl` and `cargo e2e` drive the running app.
3. **Keys + status bar.** Done: ⌘D, ⌘⇧D, ⌘W, ⌘⌥arrows; `NORMAL` / `APP` pill.
   Open: draggable dividers, global key to leave `APP` mode.
4. **Terminal pane.** Done. PTY + `alacritty_terminal` grid, painted in GPUI. Key encoding and
   colours in the core. E2E types a command and reads the grid back.
   Open: scrollback, selection and copy, mouse reporting, cwd tracking, wide-glyph alignment.
5. **Tabs, then tasks.** Model first (core), then shell.

## Decisions

- **Terminal backend:** `alacritty_terminal` 0.26 with our own renderer (the Zed model).
  Rejected: `gpui-terminal` (one release, 2025-12-24, wraps `alacritty_terminal` 0.25) and
  `gpui-ghostty` (one release, 2026-08-04, 58 downloads); hosting Ghostty/iTerm as an
  app pane (works today, but no control of cwd, scrollback or persistence, which #6 and #10 need).
- **Window rules before a scripting language (#5).** Start with declarative rules:
  match (app, title, role) → placement. Add scripting only when rules prove too weak.

## Constraints found

- **Keys while an app pane is live.** The real app has key focus, so ottospace shortcuts do not
  fire. Accepted (2026-11-11). One global event-tap key returns focus to ottospace, and the
  status bar shows `APP` mode while an app has the keys. The status bar is done; the global
  key is the open item of step 3.
- **Notifications (#1).** ottospace cannot suppress system banners or Dock badges itself. It can:
  not show Slack during a block (parked windows are invisible), cover the Dock, and toggle a macOS
  Focus through Shortcuts. Banner control comes from the macOS Focus, not from us.
- **Metrics (#1a).** Cheap: we already know which pane is live. Log (app, start, end) to a file the
  weekly-review skill reads.
- **Persistence (#6).** Per-app adapters, each small: Chrome URLs (AppleScript), Slack channel
  (deep link), VS Code folder (`code` CLI), terminal cwd (OSC 7) + scrollback (we own the grid).

## Answers (2026-11-11)

- A task owns a full workspace: tabs → splits.
- Zen mode hides held Slack completely: no pane, no count.
- The brief is a pane kind, not a full-screen view.
- Return key from `APP` mode: Caps Lock, remapped to F18 with `hidutil`. Make the key a setting,
  since the remap is a personal choice. Default: F18.

- Quick capture (⌘⌘) and task lifecycle: see `quick-capture.md`.
- Zoom meeting layout and participants → spinner: see `zoom-meeting.md`.
- Hiding panes from screen sharing: see `screen-share-privacy.md`.
- Scripting in TypeScript (replaces "rules before a scripting language"): see `scripting.md`.
- Look: apollo-ui future dark, one accent per key mode. Otto mark: `background/otto-icon.svg` (app icon) and
  `background/otto-pane.svg` (empty pane). Smooth shaded, mono, cyan eyes, ninja band.

## Open questions

1. #10: where do tasks live? Proposal: files in `~/notes`, so skills can read them.
2. #3: which ottobot parts move here: the scheduler, or only the report views?

## Mockup

Design canvas, 5 boards: https://claude.ai/artifact/GUTdW4ZrsRs1fjvPb4Atyh
(workspace, in-app mode, app switcher, focus block, next task). Not yet agreed.
