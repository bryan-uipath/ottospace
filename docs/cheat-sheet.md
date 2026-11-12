# ottospace cheat sheet

One page for daily use. ⌘/ shows the keys inside the app. Full detail: [reference.md](reference.md). Install and update: [install.md](install.md).

## Start

```sh
ottospace            # opens on the Repos list
```

- First run: grant your terminal Accessibility (ottospace will not start without it) and Screen Recording (for app panes), in System Settings ▸ Privacy & Security.
- Click a repository or worktree: a terminal opens there, in the empty pane or else in a split to the right (⇧-click: below).
- The status bar (bottom left) shows who has the keyboard: `NORMAL`, `APP`, `PICK`, `LIST`, `TASK`, `VISUAL`, `SNEAK`, `QUIT?`.
- Stuck in an app? Click ottospace's rail or top bar. Or press the return key: F18, which most keyboards lack, so map Caps Lock to it (see [the return key](reference.md#the-return-key)) or start with `OTTOSPACE_RETURN_KEY=f13` (f13 to f20). The status bar names the key in use.

## Panes and tabs

| I want to | Press |
|---|---|
| Split with a new terminal | ⌘D right · ⌘⇧D down |
| Choose what goes in a new pane (terminal or app) | ⌘E, type to filter, ↵ |
| Move between panes | ⌘⌥ arrows or ⌘⌥ H J K L · ⌘] ⌘[ cycle |
| Resize the pane | ⌘⌃ arrows or ⌘⌃ H J K L: the split moves that way · ⌘⌃= evens all out |
| Give an app the room its window needs | ⌘⌃0, or `fit` in the pane header: shown when the app refused the pane's size and a split can move |
| Swap the pane with its neighbour | ⌘⇧⌥ arrows or ⌘⇧⌥ H J K L |
| Move the pane to another tab | ⌘⇧⌥1 … ⌘⇧⌥9. US keyboard layout only (US, ABC): on others the keys do nothing |
| Rearrange with the mouse | Drag a pane by its header: onto another pane's edge to split there, its centre to swap, a tab to move there. Esc cancels. Dragging an app you are in parks it: click it to go back in |
| Close the pane | ⌘W |
| New tab | ⌘T |
| Hide ottospace | ⌘H |
| Quit | ⌘Q, or the window's close button. While a terminal runs something (a program, or a background job) it asks first: ↵, Q or ⌘Q again quits; Esc, any other key or shortcut cancels and does nothing else. Shells at a prompt do not count. Dock ▸ Quit and logout do not ask |
| Find an action by name | The menu bar |
| Switch tab | ⌘1 … ⌘9 · ⌘⇧] ⌘⇧[ · ⌃Tab |
| Close the tab and its panes | ⌘⇧W |
| Fill the screen | ⌘⌃F (again to restore) |

## The bar on the left (Apps, Tasks, Repos)

| I want to | Do |
|---|---|
| Open a section | Click its icon on the rail |
| Use it from the keyboard | ⌘⇧E, then `j` `k` move · ↵ open · ⇧↵ open below · Tab next section · Esc leave |
| Read the lists again | ↻ in the bar's header |
| Click anything without the mouse | ⌘⌃G: every control gets a letter label (a × gets two). Type it · ⌫ takes a letter back · Esc or a wrong letter leaves |

The fourth icon, Workbook, is empty for now.

## Apps

| I want to | Do |
|---|---|
| Put a running app's window in a pane | Apps, click the app (⇧-click: split down). Or ⌘E and pick it |
| Start an app that is not running | Name it in the `apps` setting. It is listed under "Not running": click it |
| Type in the app | Click its pane. The status bar turns `APP`; ottospace keys stop |
| Come back | Click ottospace itself, or the return key |
| Give the window back to macOS | Come back to ottospace first, then ⌘W on its pane: the window returns to where it was |

The badge on an app is how many windows it has open. A new window of a hosted app gets its own pane.

## Tasks

| I want to | Do |
|---|---|
| Add a task | ⌘N, type a line, ↵. The + on the Tasks bar opens the same box |
| Add a task and start it (show its tabs) | ⌘N, type a line, ⌘↵ |
| Tag the task | In that line: `#project` · `>repo` · `@agent` · `/path` · `$skill` · `p1` … `p9`. Tab completes the tag being typed. [All tags](reference.md#tags) |
| Open a task: the tab bar shows only its tabs | Click it in Tasks. The first time, it gets a tab named after it. × beside its name on the tab bar goes back |
| Open the next task that needs me | ⌘⇧N |
| See what needs me at a glance | A tab with no panes is the home screen: the new-task line (click it, or ⌘N), up to three tasks under Needs you, and a node for each of the first 12 listed tasks. Left to right is the project; within a project the older task is farther; the colour is the state. No node sits under the text, so a small window shows fewer. Hover a node for its title; click a node or a row to open the task |
| See the inbox | Tasks on the rail: Needs you, Open (the other tasks with tabs open), Running, Captured, On hold, Done. Click a heading to fold it, or ⌘⇧E, move to it, ↵ or space |
| See agent sessions that are no task (a Claude Code or Codex started by hand) | Tasks on the rail, under Sessions: runner, repository, active or idle, what it does now. Hover a row: Adopt makes it a task, Hide removes it until the next start. [More](reference.md#sessions) |
| Snooze 1 hour / hold / dismiss / undo | ⌘⇧E on Tasks, then `s` / `h` / `x` / `u`. By mouse: the buttons on a row you hover; "Undo" then shows at the top for 6 seconds |

The status bar shows how many tasks need you. Tasks are kept in `~/.ottospace/tasks`, or the `tasks` setting.
A tag counts only if its name is known (`>repo` in the Repos list, `@claude` or `@codex`, a skill that is installed, a path that exists); any other word stays in the title.

From a shell or an agent:

```sh
ottospace ctl '{"cmd":"task_new","line":"fix zoom flicker #canvas >flow-workbench @claude p1"}'
ottospace ctl "{\"cmd\":\"task_add\",\"title\":\"review my PR\",\"state\":\"needs_you\",\"tab\":$OTTOSPACE_TAB}"   # a task tied to the tab this shell started in
ottospace ctl '{"cmd":"task_set_state","id":1,"state":"needs_you"}'   # running, needs_you, done, ...
ottospace ctl '{"cmd":"task_list"}'
```

## Terminal text

| I want to | Do |
|---|---|
| Scroll back | Wheel or trackpad · ⇧PageUp ⇧PageDown · ⇧Home ⇧End |
| Select | Drag · double-click a word · triple-click a line |
| Copy / paste | ⌘C / ⌘V |
| Select with the keyboard | ⌘⇧C, then `hjkl` move, `v` select, `y` copy, Esc leave |

Scrolling is the program's own inside full-screen programs (vim, less).

## Drive it from a shell

```sh
ottospace ctl '{"cmd":"state"}'                                   # everything, as JSON
ottospace ctl '{"cmd":"split","axis":"down"}'
ottospace ctl '{"cmd":"type","pane":3,"text":"cargo test; echo done-7f3a-$?\r"}'
ottospace ctl '{"cmd":"wait","pane":3,"pattern":"^done-7f3a-[0-9]+$","timeout_ms":600000}'
ottospace ctl '{"cmd":"capture","pane":3,"history":true}'
ottospace skill                                                    # the full guide for agents
```

A shell started by ottospace has `$OTTOSPACE_PANE` and `$OTTOSPACE_TAB`: the tab it started in.
If the pane was moved to another tab since, this gives the tab it is in now:

```sh
ottospace ctl '{"cmd":"state"}' | jq --argjson pane "$OTTOSPACE_PANE" '.state.tabs[.tab_panes | map(any(. == $pane)) | index(true)].id'
```

## Settings

`~/.ottospace/settings.json`, read at start:

```json
{
  "terminal": { "font": "JetBrains Mono", "font_size": 14 },
  "repos": ["~/code", "~/notes"],
  "tasks": "~/notes/tasks",
  "sessions": true,
  "session_dirs": { "claude": ["~/.claude"], "codex": "~/.codex" },
  "session_quiet_minutes": 3,
  "apps": ["Microsoft Outlook", "Slack"],
  "windows": [{ "app": "us.zoom.xos", "title": "Zoom Meeting", "do": "ignore" }]
}
```

A mistake in a key (one that does not exist, or a wrong value) shows in the status bar: that key
takes its default and the others still apply. If the file is not a JSON object at all, every
setting takes its default.

`sessions`: `false` reads no agent session files. `session_dirs` names where Claude Code and Codex keep them, if not `~/.claude` and `~/.codex`. `session_quiet_minutes`: a task adopted from a session moves to Needs you after the session has been quiet this long (default 3; 0 never).

`windows` rules name windows ottospace leaves alone, by `app`, `title` and `subrole`: [reference](reference.md#settings).

`OTTOSPACE_REPOS` (directories, `:` between them) wins over `repos`. `OTTOSPACE_HOME` names another folder than `~/.ottospace`.

## Not there yet

- Quitting loses tabs, splits and terminals (tasks and settings are kept).
- Filing a task starts nothing: no agent and no worktree. The task gets its first tab when you open it.
- No paste or arrow keys in the new-task box.
- An app that is not running is listed only if the `apps` setting names it.
