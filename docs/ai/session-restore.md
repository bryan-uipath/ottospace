# Session restore

Status: design, not built. Replaces backlog item 5.

Goal: quit ottospace (or restart the Mac) and get the same workspace back: tasks, their tabs, the
splits, a terminal in the same directory with its output, the same app windows in the same panes,
and each app on the thing it showed (the Chrome tabs, the Slack channel).

## What exists

- Tasks: one JSON file each in the tasks folder (`config.rs`, `save_task`).
- `recovery` (`ottospace-mac`): `~/Library/Application Support/ottospace/hosted.txt`, one
  `pid window-id x y w h` line per hosted window. Its only job is to give windows back after a
  crash. It stays as it is: restore must never depend on it, and it must work when restore fails.
- Nothing else is kept. Tabs, splits, panes and terminal output die with the process.

## Layers

Each layer ships by itself and is useful without the next.

| Layer | Restores | Needs |
|---|---|---|
| 1. Layout | Tabs (with their task), split trees, which pane has focus, the active tab and section. Terminal panes come back as new shells in their recorded directory. App panes come back as empty panes that name the app | `session.json` |
| 2. Content | Terminal scrollback and current directory. App panes re-attached to the same window if the app is still running | OSC 7, a scrollback file per pane, bundle id + window id + title |
| 3. Apps that are not running | The app is started and its window put in the pane | `open -g`, already used by the `apps` setting |
| 4. App state | Chrome tabs and profile, Slack channel, Outlook view, VS Code folder | Capture and restore hooks, one per app, through the SDK |

## Layer 1: `session.json`

`~/.ottospace/session.json` (under `OTTOSPACE_HOME`). The shape follows the model:

```json
{
  "version": 1,
  "active": 0,
  "section": "tasks",
  "current_task": 12,
  "tabs": [
    {
      "title": "flow-workbench",
      "task": 12,
      "focused": 2,
      "layout": { "split": "right", "ratio": 0.6, "a": { "pane": 1 }, "b": { "pane": 2 } },
      "panes": [
        { "id": 1, "kind": "terminal", "cwd": "/Users/me/code/flow-workbench" },
        { "id": 2, "kind": "app", "bundle": "com.google.Chrome", "name": "Google Chrome", "title": "localhost:3001" }
      ]
    }
  ]
}
```

- **The core owns the format.** `Model::session() -> Session` and `Action::Restore(Session)`,
  both pure, with a round-trip unit test. The shell reads and writes the file.
- **Written** after an action that changes it, at most once a second, to a temp file that is then
  renamed. Also on quit. A crash loses at most the last second.
- **Read** at start, before the first tab is made. A file that does not parse is renamed to
  `session.json.bad` and reported in the status bar; ottospace starts empty.
- **Ids.** Pane ids are renumbered on restore. A tab whose task no longer exists becomes a tab of
  no task.
- **Tests** never read it: an E2E run has no session unless it names its own `OTTOSPACE_HOME`.
- **Off switch.** `"restore": false` in `settings.json`. `ottospace --fresh` starts empty once and
  keeps the old file as `session.json.prev`.

## Layer 2: content

**Terminals.**
- Directory: the shell reports it with OSC 7; `Pane.cwd` follows it. Without OSC 7 (a shell that
  does not send it) the directory is the one the pane started in.
- Scrollback: we own the grid. On the same timer as `session.json`, changed panes write their
  lines to `~/.ottospace/scrollback/<pane>.txt`, capped (10 000 lines). On restore the text is
  drawn above a `[restored]` rule and a new shell starts below it. Text only: no colours at
  first.
- Not restored: the running process. A restored pane is a new shell. Agents that can resume
  (`claude --resume`) are a hook's job (layer 4), not the terminal's.
- Scrollback can hold secrets. The files are mode 0600, the folder 0700, and
  `"terminal": {"keep_scrollback": false}` turns it off.

**App panes.** A pane records bundle id, app name, window id, title and the window's original
frame. On restore, in order:
1. The app runs and a window with that id exists: host it.
2. Else a window of that app with the same title: host it.
3. Else, if the app has exactly one window the rules allow: host it.
4. Else leave an empty pane that says "Chrome: window not found" with a key to pick one.

Window ids do not survive the app's restart, so after a reboot step 2 or 3 does the work. Window
rules (`windows` setting) apply as they do for a new window.

## Layer 3: apps that are not running

Same path as the `apps` setting: `open -g <bundle path>`, the pane shows `launching`, and the
window is hosted when it appears. All apps of a session start at once; a pane gives up after 30
seconds and says so. Ask first when a restore would start more than five apps.

## Layer 4: per-app state

A hook is a pair, run through the SDK (its design note is `docs/ai/sdk.md`, in review):

```ts
restore.register("com.google.Chrome", {
  capture: async (win) => ({ urls: await chromeTabs(win), profile: await chromeProfile(win) }),
  restore: async (state) => openChromeWindow(state.profile, state.urls),
});
```

- `capture` returns JSON, stored on the pane in `session.json` as `"state"`. It runs when the
  session is written, with a time limit; a hook that fails keeps the last good state.
- `restore` runs when no existing window matched, and returns the window to host.
- Shipped defaults, each small:

| App | Capture | Restore |
|---|---|---|
| Chrome | Tab URLs and active tab (Apple Events), profile | New window in that profile with those URLs |
| Slack | Channel or DM (accessibility: window title, or the deep link) | `slack://channel?team=…&id=…` |
| Outlook | The view: mail, calendar | Activate the view |
| VS Code | Folder (window title, or `code --status`) | `code <folder>` |
| Terminal agent | Session id, from the pane's output | `claude --resume <id>` typed into the new shell |

- **Trust.** Capture reads another app's state: the same level as `ax_tree`, and Apple Events
  raise their own macOS prompt per app. Restore opens URLs and files, and for the agent hook types
  into a terminal. No hook presses a control in another app; that stays behind the decision in
  `1-initial-brainstorm/accessibility.md`.
- **Privacy.** `session.json` then holds URLs and channel names. Mode 0600. A window the rules
  ignore is never captured. A per-app `"capture": false` skips the hook.

## Failure rules

- Restore never moves a window it did not host in the saved session.
- Nothing blocks start: ottospace draws the layout at once and panes fill in as their content
  arrives.
- Every pane that could not be restored says why, in the pane.
- A second ottospace on the same `OTTOSPACE_HOME` does not restore and does not write the session
  (a lock file); it says so in the status bar.

## Order of work

1. `Session` in the core, `session.json` write and read, terminals in their directory, app panes
   as named placeholders. Core tests, plus an E2E that quits and relaunches on a private home.
2. OSC 7 and scrollback files.
3. Re-attach app panes (steps 1 to 4 above), with `cargo e2e-apps` on the canary.
4. Start apps that are not running.
5. Hooks, after the SDK's first slice: Chrome, then Slack, then the rest.

## Open questions

- Scrollback with colours: store the grid's cells, or replay the raw bytes?
- One session, or one per task (so a task can be closed and reopened with its workspace)? The
  format allows both: a task's tabs are already grouped by `task`.
- Displays: which screen the ottospace window was on (see `1-initial-brainstorm/multi-monitor.md`,
  parked).
