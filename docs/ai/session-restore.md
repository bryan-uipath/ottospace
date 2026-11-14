# Session restore

Status: layer 1 (the layout file) is built: see "Layer 1, as built". Layers 2 to 4 are design.
Replaces backlog item 5.

Goal: quit ottospace (or restart the Mac) and get the same workspace back: tasks, their tabs, the
splits, a terminal in the same directory with its output, the same app windows in the same panes,
and each app on the thing it showed (the Chrome tabs, the Slack channel).

## What exists

- Tasks: one JSON file each in the tasks folder (`config.rs`, `save_task`).
- `recovery` (`ottospace-mac`): `~/Library/Application Support/ottospace/hosted.txt`, one
  `pid window-id x y w h` line per hosted window. Its only job is to give windows back after a
  crash. Its file and format stay as they are: restore must never depend on it, and it must work
  when restore fails. Only when it runs changes (see the lock, under Failure rules).
- Nothing else is kept. Tabs, splits, panes and terminal output die with the process.

## Layers

Each layer ships by itself and is useful without the next.

| Layer | Restores | Needs |
|---|---|---|
| 1. Layout | Tabs (with their task), split trees, which pane has focus, the active tab and section. Terminal panes come back as new shells in their start directory. App panes come back as placeholder panes that name the app | `session.json`, a placeholder `Kind` |
| 2. Content | Terminal scrollback and current directory. App panes re-attached to the same window if the app is still running | OSC 7, a scrollback file per pane, bundle id + window id + title |
| 3. Apps that are not running | The app is started and its window put in the pane | `open -g`, as the `apps` setting does |
| 4. App state | Chrome tabs and profile, Slack channel, Outlook view, VS Code folder | Capture and restore hooks, one per app, through the SDK |

## Layer 1, as built

The file is `layout.json`, not `session.json`, and a pane is kept as a workbook keeps one.

- **Where**: `<home>/layout.json` (`OTTOSPACE_HOME`; none in a test run that named no folder).
  0600, written whole and renamed into place, read only if it is a plain file of at most 4 MB
  (no link, no pipe). Unknown keys of the file are kept; `version` 1.
- **What**: one ordered list of tabs, each a workbook's `TabRecord` (title, `task`, split tree
  with ratios, focused pane, panes: terminal with `cwd` and the workbook's `history`, app by
  bundle id and name and never a title, empty), plus the workbook's tab it stands for and the
  marks that stood for it or its panes (by the mark's `id`), and `settings`: the record id of the
  pane Open Settings opened (kept without history; restored, the core sends `EditSettings` for
  that terminal and the shell runs its own fixed editor command; one at most, only on a terminal).
  A pane `kind` this build does not know reads as `empty` here (`marks.json` and workbooks still
  leave such a file alone). Then `selected` (the tab on show,
  by place), `section`, and `drawer` (`task`, `open`, `width`, `wide`). A session's
  conversation that is no task's is not kept. No scope on show: per-task tab sets are going away;
  restored on today's model, the tab on show brings its task's tabs with it.
- **The core owns it** (`ottospace-core/src/layout_file.rs`): `Model::layout_file(cwds,
  histories, lines)` makes it, `parse` checks it (caps of a workbook's tab, at most 256 tabs,
  ids that are ids, absolute one-line directories), `Action::RestoreLayout` builds it into a
  fresh model only, from the shell only (a client is refused). `Model::layout_moved` says when
  it changed; the shell writes it a second later, and at quit with each terminal's directory
  and text read again. Not an `Effect`: an effect on every change would sit in the result of
  every action.
- **Terminal text**: the workbook's rules exactly (`workbook_scrollback_lines`; `0` keeps no
  text and no command; cleaned, credential shapes hidden, 64 KB a terminal, 1 MB in all). The
  command is typed on the prompt, never run, and the divider names it. Never in the state.
- **Restore** (`Workspace::new`, after tasks, workbooks and marks are read; the guardian's
  leftovers were given back before the app started): each tab is built as a workbook's tab
  (`rebuild_tab`), in order, each task's tabs together and tabs of no task last
  (`layout_file::grouped`, the one function to swap for the core's own once tabs are one bar).
  A tab whose task is gone is a tab of no task. A directory not found starts in the home
  folder, with one status-bar line. App panes ask for their window as a workbook's do, but an
  app that is not running is not started (`HostWindow { launch: false }`): the pane says so and
  offers `Open <App>` (`Action::UnfilledOpen`, the user's only). `restore_apps: true` starts it.
  The drawer shows its task's conversation with no agent: a click on the task goes on with it.
- **Failures**: does not read or does not pass → `layout.json.bad`, an empty start, a line in
  the status bar. A newer `version` → left as it is, not written this run. `restore_layout:
  false` → neither read nor written. File ▸ Start Fresh → the file is removed and not written
  again this run.
- **Not built from the design below**: the second-instance lock, the placeholder `Kind` (an
  empty pane that waits, `marks::Unfilled`, stands for it), window matching by id and title,
  OSC 7.

## Layer 1: `session.json` (the design)

`~/.ottospace/session.json` (under `OTTOSPACE_HOME`). `Session` is its own serde type, not the
model's state shape (`Node` and `Pane` do not deserialize, and `Kind::App` has no bundle):

```json
{
  "version": 1,
  "active": 0,
  "sidebar": "tasks",
  "current_task": 12,
  "tabs": [
    {
      "title": "flow-workbench",
      "task": 12,
      "focused": 2,
      "layout": { "split": { "axis": "right", "a": { "leaf": 1 }, "b": { "leaf": 2 } } },
      "panes": [
        { "id": 1, "key": "k3f9", "kind": { "type": "terminal" }, "cwd": "/Users/me/code/flow-workbench" },
        { "id": 2, "key": "k7c2", "kind": { "type": "app", "bundle": "com.google.Chrome", "name": "Google Chrome",
            "pid": 811, "window": 4211, "title": "localhost:3001" } }
      ]
    }
  ]
}
```

- **Names follow the model** where it has them: `sidebar` (null: the bar is closed), `axis`,
  `leaf`, `type`. Splits are equal halves today, so there is no ratio. PR #49 (`pane-keys`, open)
  adds split ratios; `Session` carries them once it merges.
- **The core owns the format.** `Model::session() -> Session` and `Action::Restore(Session)`,
  both pure, with a round-trip unit test. The shell reads and writes the file.
- **Placeholder.** A new `Kind` variant (bundle, name, reason) for an app pane with no window:
  not restored yet, starting, or not found. It does not exist today. It keeps its saved record,
  so a write after a failed restore loses nothing.
- **Directory.** `Pane.cwd` is set today only for a pane opened from a repository. Layer 1
  records every terminal's resolved start directory through an `Action`. A directory that is
  gone falls back to home, as now.
- **Written** after an action that changes it, at most once a second, to a temp file in the same
  folder that is then renamed, as `save_task` does. Also on quit. Best effort: without an fsync
  of the file and the folder, a power loss can lose more than the last second.
- **Mode 0600** from creation, temp file and renamed copies included: layer 1 already holds
  window titles and directories.
- **An app that quit is not a closed window.** Today a pane goes when its window goes
  (`drop_closed`). At logout apps quit before ottospace, and the next write would save a session
  with no app panes. So: a pane whose app quit becomes a placeholder; only a window closed while
  its app runs removes the pane. Writes stop once the system announces shutdown or logout.
- **Read** at start; `Action::Restore` replaces the default tab. A file that does not parse is
  renamed to `session.json.bad` and reported in the status bar; ottospace starts empty. A file
  with an unknown `version` (a newer build wrote it, then `osi` rolled back) is left untouched
  and nothing is written this run.
- **Ids.** Pane ids are renumbered on restore; `key` is the pane's stable name and does not
  change. A tab whose task no longer exists becomes a tab of no task.
- **Tests** never read it: an E2E run has no session unless it names its own `OTTOSPACE_HOME`.
  Layer 3 is off under `OTTOSPACE_E2E` without `OTTOSPACE_HOST_ONLY`: a test never starts a real
  app.
- **Off switch.** `"restore": false` in `settings.json`. `ottospace --fresh` starts empty once and
  keeps the old file as `session.json.fresh`.

## Layer 2: content

**Terminals.**
- Directory: the shell reports it with OSC 7; `Pane.cwd` follows it. Not handled today. Without
  OSC 7 (a shell that does not send it) the directory is the one the pane started in.
- Scrollback: we own the grid. On the same timer as `session.json`, changed panes write their
  lines to `~/.ottospace/scrollback/<key>.txt`, capped (10 000 lines). Named by the pane's `key`,
  not its id: ids are renumbered, and a stale `1.txt` would show in the wrong pane. Not written
  together with `session.json`: after a crash the text can be older or newer than the layout.
  On restore the text is drawn above a `[restored]` rule and a new shell starts below it. Text
  only: no colours at first.
- Not restored: the running process. A restored pane is a new shell. Agents that can resume
  (`claude --resume`) are a hook's job (layer 4), not the terminal's.
- Scrollback can hold secrets. The files are mode 0600 from creation, the folder 0700. A pane's
  file is deleted when the pane closes. `"terminal": {"keep_scrollback": false}` turns it off
  and deletes the folder.

**App panes.** A pane records bundle id, app name, pid, window id and title. On restore, in
order:
1. The app runs with the saved pid and bundle (the same run of the app) and has a window with
   that id: host it. Window ids start again after a reboot, so an id alone can name another
   window.
2. Else a window of that app with the same title: host it.
3. Else, if the app has exactly one window the rules allow: ask, then host it.
4. Else leave a placeholder that says "Chrome: window not found" with a key to pick one.

Each window is matched once: two panes titled "New Tab" never get the same window. A window
already in a pane is never a candidate. After a reboot step 2 or 3 does the work. Window rules
(`windows` setting) apply as they do for a new window.

Some apps bring their own windows back (Chrome, VS Code, macOS at login). After an app starts,
or while it is still starting, wait until its windows settle (no new window for a few seconds),
then run steps 1 to 3 again. Only after that may a layer 4 `restore` hook open a window.

## Layer 3: apps that are not running

As the `apps` setting does: `open -g <bundle path>`, and the window is hosted when it appears.
Today no pane exists while an app starts (the Apps row says "opening…"); here the placeholder
says `launching`. All apps of a session start at once; a pane gives up after 30 seconds
(`LAUNCH_WAIT`) and says so. Ask first when a restore would start more than five apps.

`session.json` is input, not trusted text. The bundle path comes from Launch Services, by the
saved bundle id; a path written in the file is never run.

## Layer 4: per-app state

A hook is a pair, run through the SDK (design note in PR #45, not merged; how scripts run:
`1-initial-brainstorm/scripting.md`):

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
| Outlook | The view: mail, calendar | Blocked on the `ax_press` decision |
| VS Code | Folder (window title, or `code --status`) | `code <folder>` |
| Terminal agent | Session id, from the pane's output | `claude --resume <id>` typed into the new shell, only if the id is a UUID |

- **Trust.** Capture reads another app's state: the same level as `ax_tree`, and Apple Events
  raise their own macOS prompt per app. Restore opens URLs and files, and for the agent hook types
  into a terminal. No hook presses a control in another app; that stays behind the decision in
  `1-initial-brainstorm/accessibility.md`. Outlook's restore needs a press, so it waits.
- **Input.** Captured state is checked before use. Terminal output is not ours (`cat`, `curl`, a
  remote host): the agent id must match the UUID pattern or the hook does nothing. Chrome
  restore opens `http` and `https` only. The Slack link is built from checked ids, never stored
  whole. `code <folder>` takes a folder that exists.
- **Privacy.** `session.json` then holds URLs and channel names. URLs are stored whole and can
  hold tokens. A window the rules ignore is never captured, nor is an Incognito window. A
  per-app `"capture": false` skips the hook and deletes the state it kept. `session.json.bad`
  and `session.json.fresh` are copies with the same content; `"restore": false` deletes them and
  `session.json`.

## Failure rules

- Restore hosts only windows of an app in the saved session, matched by steps 1 to 3, and never
  one already in a pane.
- Nothing blocks start: ottospace draws the layout at once and panes fill in as their content
  arrives.
- Every pane that could not be restored says why, in the pane.
- A second ottospace on the same `OTTOSPACE_HOME` does not restore and does not write the session;
  it says so in the status bar. The lock is held for the life of the process, as the socket's is
  (`control.rs`), so a crash frees it. It is taken before `recovery::restore_leftovers`: today a
  second instance gives back the first one's parked windows and clears its list (`hosted.txt`
  ignores `OTTOSPACE_HOME`). A non-owner neither recovers nor restores.

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
- Displays: which screen the ottospace window was on, and the per-tab `display` that
  `1-initial-brainstorm/multi-monitor.md` plans (parked).
- Windows used outside ottospace since a clean quit: re-attach always, only if the frame is
  unchanged, or ask when the session is old?
- Directory without OSC 7: macOS zsh sends it only to Terminal.app. Read the shell's directory by
  pid when the session is written? Accept OSC 7 only for this host (not from `ssh`)?
- Slack: the window title has the channel's name, not the team and channel ids the link needs.
  Where do they come from? Else restore the workspace only.
- Tab selection: save each task's last tab (`last_tab`), and repair `current_task`, `active` and
  focus together when a task is gone.
- Capture cost: on every session write is too often for Apple Events and accessibility reads.
  On quit, on leaving the pane, every N seconds?
