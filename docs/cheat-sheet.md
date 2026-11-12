# ottospace cheat sheet

One page for daily use. ⌘/ shows the keys inside the app. Full detail: [README](../README.md).

## Start

```sh
ottospace            # opens on the Repos list
```

- First run: grant your terminal Accessibility (ottospace will not start without it) and Screen Recording (for app panes), in System Settings ▸ Privacy & Security.
- Click a repository or worktree: a terminal opens there, in the empty pane or else in a split to the right (⇧-click: below).
- The status bar (bottom left) shows who has the keyboard: `NORMAL`, `APP`, `PICK`, `LIST`, `TASK`, `VISUAL`.
- Stuck in an app? Click ottospace's rail or top bar. Or press the return key: F18, which most keyboards lack, so map Caps Lock to it (see README) or start with `OTTOSPACE_RETURN_KEY=f13` (f13 to f20). The status bar names the key in use.

## Panes and tabs

| I want to | Press |
|---|---|
| Split with a new terminal | ⌘D right · ⌘⇧D down |
| Choose what goes in a new pane (terminal or app) | ⌘E, type to filter, ↵ |
| Move between panes | ⌘⌥ arrows or ⌘⌥ H J K L · ⌘] ⌘[ cycle |
| Close the pane | ⌘W |
| New tab | ⌘T |
| Hide ottospace · quit | ⌘H · ⌘Q |
| Find an action by name | The menu bar: File, Edit, View, Pane, Tasks, Help |
| Switch tab | ⌘1 … ⌘9 · ⌘⇧] ⌘⇧[ · ⌃Tab |
| Close the tab and its panes | ⌘⇧W |
| Fill the screen | ⌘⌃F (again to restore) |

## The bar on the left (Apps, Tasks, Repos)

| I want to | Do |
|---|---|
| Open a section | Click its icon on the rail |
| Use it from the keyboard | ⌘⇧E, then `j` `k` move · ↵ open · ⇧↵ open below · Tab next section · Esc leave |
| Read the lists again | ↻ in the bar's header |

The fourth icon, Workbook, is empty for now.

## Apps

| I want to | Do |
|---|---|
| Put a running app's window in a pane | Apps, click the app (⇧-click: split down). Or ⌘E and pick it |
| Type in the app | Click its pane. The status bar turns `APP`; ottospace keys stop |
| Come back | Click ottospace itself, or the return key |
| Give the window back to macOS | Come back to ottospace first, then ⌘W on its pane: the window returns to where it was |

The badge on an app is how many windows it has open. A new window of a hosted app gets its own pane.

## Tasks

| I want to | Do |
|---|---|
| File a task | ⌘N, type a line, ↵ |
| Tag a project, name a repository, set priority | In that line: `#canvas` · `@flow-workbench` · `p1` … `p9`. Tab completes `#` and `@` |
| Go to the tab of the next task that needs me | ⌘⇧N. Only for a task that has a tab: one an agent filed with `tab`. Tasks from ⌘N have none yet, so for them nothing happens |
| See the inbox | Tasks on the rail: Needs you, Running, Captured, On hold, Done |
| Snooze 1 hour / hold / dismiss / undo | ⌘⇧E on Tasks, then `s` / `h` / `x` / `u` |

The status bar shows how many tasks need you. Tasks are kept in `~/.ottospace/tasks`, or the `tasks` setting.
`@name` must be a repository or worktree in the Repos list; any other `@word` stays in the title.

From a shell or an agent:

```sh
ottospace ctl '{"cmd":"task_new","line":"fix zoom flicker #canvas @flow-workbench p1"}'
ottospace ctl "{\"cmd\":\"task_add\",\"title\":\"review my PR\",\"state\":\"needs_you\",\"tab\":$OTTOSPACE_TAB}"   # a task tied to this tab
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

A shell started by ottospace has `$OTTOSPACE_PANE` and `$OTTOSPACE_TAB`.

## Settings

`~/.ottospace/settings.json`, read at start:

```json
{
  "terminal": { "font": "JetBrains Mono", "font_size": 14 },
  "repos": ["~/code", "~/notes"],
  "tasks": "~/notes/tasks"
}
```

A mistake in a key (one that does not exist, or a wrong value) shows in the status bar: that key
takes its default and the others still apply. If the file is not a JSON object at all, every
setting takes its default.

`OTTOSPACE_REPOS` (directories, `:` between them) wins over `repos`. `OTTOSPACE_HOME` names another folder than `~/.ottospace`.

## Not there yet

- Quitting loses tabs, splits and terminals (tasks and settings are kept).
- A task filed with ⌘N owns no tab and no worktree; filing one starts nothing. (An agent can attach a tab: `task_add` with `tab`.)
- No paste or arrow keys in the new-task box.
- Apps that are not running are not listed.
