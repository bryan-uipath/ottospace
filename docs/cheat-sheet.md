# ottospace cheat sheet

One page for daily use. ⌘/ shows the keys inside the app (J / K, space, Home / End scroll them). Full detail: [reference.md](reference.md). Install and update: [install.md](install.md).

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
| Choose what goes in a new pane (terminal or app) | ⌘E, type to filter, ↵. Typing also offers installed apps that are not running |
| Edit the picker's filter or the new-task line | As any Mac text field: ⌥←/→ words, ⌘←/→ line, ⇧ selects, ⌘A, ⌘C/⌘X/⌘V, ⌘Z/⇧⌘Z, ⌥⌫ a word, ⌘⌫ the line; click, drag, double click |
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
| Start an app that is not running | Apps, click it under "Not running" (pinned apps, then the Dock's) or under "All apps". Or ⌘E, type its name, ↵ |
| Keep an app at the top of that list | Hover its row, Pin. Or ⌘⇧E on Apps, then `p` (again unpins). It is written to `apps` in the settings |
| Stop a Dock app being suggested | Hover its row, Hide. Or ⌘⇧E, then `x`. Undo on the line at the top (six seconds), or `u` |
| Find an app among all installed | Apps, "All apps (N)", then click the filter or `/` in list mode, and type |
| Type in the app | Click its pane. The status bar turns `APP`; ottospace keys stop |
| Come back | Click ottospace itself, or the return key |
| Give the window back to macOS | Come back to ottospace first, then ⌘W on its pane: the window returns to where it was |

The badge on an app is how many windows it has open. A new window of a hosted app gets its own pane.

## Marks

A mark is a named way back to a pane or a whole tab. It is kept between runs.

| I want to | Do |
|---|---|
| Mark the focused pane / the tab on show | ⌘B / ⌘⇧B. Again takes the mark off. A marked pane or tab shows ◆ |
| Go to a mark | ⌘' then its letter · ⌘⌥1 … ⌘⌥9 for the first nine · click it in Marks on the rail |
| Give a mark a letter, rename it, remove it | In Marks: click its letter box, ✎ or ×. Or ⌘⇧E, then `m`, `r`, `x` |
| Put back a mark I took off | Undo on the line at the top of Marks (six seconds), or ⌘⇧E on Marks, then `u`. ⌘B again on the same pane or tab puts back the same mark |
| Reorder marks | ⌘⇧E on Marks, then ⇧J / ⇧K |

Open now (a dot in the list): its tab comes forward and its pane takes focus. Else it is opened
again: a terminal in the directory it was in when marked, an app's window by its title, a tab with
its splits. A mark whose directory is gone or whose app is not installed says so; a tab opens with
what can be opened, and its other panes are empty and say why. Marks are kept in
`~/.ottospace/marks.json`.

## Tasks

| I want to | Do |
|---|---|
| Add a task | ⌘N, type a line, ↵. The + on the Tasks bar opens the same box. On the home screen click its line: it is the box, and stays for the next task |
| Add a task and start it (show its tabs) | ⌘N, type a line, ⌘↵ |
| Point at something for a task ("this message", "this error") | ⌃⌥⌘Space, from any app: the pane in use freezes as a picture and a draft opens on the right. Drag to circle, ⇧-drag a box, ⌥-drag a line, click to type a note. ↵, type what you want, ↵ (⌘↵ to start it). Esc leaves the picture; Esc in the title cancels the draft. The task keeps the pictures and the text under what you circled as its brief: [Capture](reference.md#capture) |
| Capture a second pane into the same draft | While drawing, click the other pane. Or press ⌃⌥⌘Space again |
| Give an agent a task's brief | In a terminal of the task: `ottospace ctl "{\"cmd\":\"task_brief\",\"id\":$OTTOSPACE_TASK}"` |
| Tag the task | In that line: `#project` · `>repo` · `@agent` · `/path` · `$skill` · `^reference` · `p1` … `p9`. A dropdown finishes the tag at the cursor: ↑↓ move, ↵ or Tab takes a row, Esc closes it. [All tags](reference.md#tags) |
| Add files to the task | Drag them from Finder onto the box, ⌘O, or copy in Finder and ⌘V: each becomes a `/path` |
| Point the task at a tab, a pane, a mark or an app | `^name` in the line, or ⌥-click the tab or pane while the box is open |
| Open a task: the tab bar shows only its tabs | Click it in Tasks. The first time, it gets a tab named after it. × beside its name on the tab bar goes back |
| Open the next task that needs me | ⌘⇧N |
| See what needs me at a glance | A tab with no panes is the home screen: the new-task line (click it, or ⌘N), up to three tasks under Needs you, and a node for each of the first 12 listed tasks. Left to right is the project; within a project the older task is farther; the colour is the state. No node sits under the text, so a small window shows fewer. Hover a node for its title; click a node or a row to open the task |
| See the inbox | Tasks on the rail: Needs you, Open (the other tasks with tabs open), Running, Captured, On hold, Done. Click a heading to fold it, or ⌘⇧E, move to it, ↵ or space |
| See agent sessions that are no task (a Claude Code or Codex started by hand) | Tasks on the rail, under Sessions: runner, repository, active or idle, what it does now. Hover a row: Adopt makes it a task, Hide removes it until the next start. [More](reference.md#sessions) |
| Snooze 1 hour / hold / dismiss / undo | ⌘⇧E on Tasks, then `s` / `h` / `x` / `u`. By mouse: the buttons on a row you hover; "Undo" then shows at the top for 6 seconds |

The status bar shows how many tasks need you. Tasks are kept in `~/.ottospace/tasks`, or the `tasks` setting.
A tag counts only if its name is known (`>repo` in the Repos list, `@claude` or `@codex`, a runner with a model as `@claude-opus-5-5` or `@codex-gpt-6-astra`, a skill that is installed, a path that exists); any other word stays in the title.

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
  "agent_models": { "claude": ["claude-opus-5-5"], "codex": ["gpt-6-astra"] },
  "capture_key": "ctrl-alt-cmd-space",
  "capture": { "keep_images": true, "ax": true },
  "apps": ["Microsoft Outlook", "Slack"],
  "apps_hidden": ["Music"],
  "suggest_dock_apps": true,
  "windows": [{ "app": "us.zoom.xos", "title": "Zoom Meeting", "do": "ignore" }]
}
```

A mistake in a key (one that does not exist, or a wrong value) shows in the status bar: that key
takes its default and the others still apply. If the file is not a JSON object at all, every
setting takes its default.

`sessions`: `false` reads no agent session files. `session_dirs` names where Claude Code and Codex keep them, if not `~/.claude` and `~/.codex`. `session_quiet_minutes`: a task adopted from a session moves to Needs you after the session has been quiet this long (default 3; 0 never). `agent_models`: the models `@` offers after each runner, as its CLI names them, in place of the built-in list.

`apps`: the pinned apps, by name or bundle path. `apps_hidden`: Dock apps not to suggest. Pin, Unpin and Hide in Apps write these two and leave the rest of the file as it is. `suggest_dock_apps`: `false` lists no Dock apps.

`capture_key`: the key that captures the pane in use (`"off"` for none). `capture`: `keep_images` `false` writes no picture; `ax` `false` reads no accessibility tree for a capture.

`windows` rules name windows ottospace leaves alone, by `app`, `title` and `subrole`: [reference](reference.md#settings).

`OTTOSPACE_REPOS` (directories, `:` between them) wins over `repos`. `OTTOSPACE_HOME` names another folder than `~/.ottospace`.

## Not there yet

- Quitting loses tabs, splits and terminals (tasks and settings are kept).
- Filing a task starts nothing: no agent and no worktree. The task gets its first tab when you open it.
- A running app cannot be pinned from the Apps section: pin it while it is closed, or name it in `apps`.
