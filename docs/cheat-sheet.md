# ottospace cheat sheet

One page for daily use. ⌘/ shows the keys inside the app (J / K, space, Home / End scroll them). Full detail: [reference.md](reference.md). Install and update: [install.md](install.md).

## Start

```sh
ottospace            # opens on the Repos list
```

- First run: grant your terminal Accessibility (ottospace will not start without it) and Screen Recording (for app panes), in System Settings ▸ Privacy & Security.
- Repos lists each repository's name over its checkouts: the branch, then `Default` (its own directory) or a worktree's directory. Click one: a terminal opens there, in the empty pane or else in a split to the right (⇧-click: below).
- The status bar (bottom left) shows who has the keyboard: `NORMAL`, `APP`, `PICK`, `COMMAND`, `INSPECT`, `LIST`, `TASK`, `VISUAL`, `SNEAK`, `QUIT?`.
- Stuck in an app? Click ottospace's rail or top bar. Or press the return key: F18, which most keyboards lack, so map Caps Lock to it (see [the return key](reference.md#the-return-key)) or start with `OTTOSPACE_RETURN_KEY=f13` (f13 to f20). The status bar names the key in use.

## Panes and tabs

| I want to | Press |
|---|---|
| Split with a new terminal | ⌘D right · ⌘⇧D down |
| Choose what goes in a new pane (terminal or app) | ⌘E (right) or ⌘⇧E (below), type to filter, ↵. Typing also offers installed apps that are not running |
| Edit the picker's filter or the new-task line | As any Mac text field: ⌥←/→ words, ⌘←/→ line, ⇧ selects, ⌘A, ⌘C/⌘X/⌘V, ⌘Z/⇧⌘Z, ⌥⌫ a word, ⌘⌫ the line; click, drag, double click |
| Move between panes | ⌘⌃ arrows or ⌘⌃ H J K L · ⌘] ⌘[ cycle |
| Move to the rails and back | The keys of the row above, on past the panes: left to the section list, then the left rail; right to the conversation, then the right rail. On a rail ↑↓ move, ↵ opens, Esc goes back |
| Resize the pane | ⌘⌥ arrows or ⌘⌥ H J K L: the split moves that way · ⌘⌃= evens all out |
| Give an app the room its window needs | ⌘⌃0, or `fit` in the pane header: shown when the app refused the pane's size and a split can move |
| Swap the pane with its neighbour | ⌘⇧⌥ arrows or ⌘⇧⌥ H J K L |
| Move the pane to another tab | ⌘⇧⌥1 … ⌘⇧⌥9. US keyboard layout only (US, ABC): on others the keys do nothing |
| Rearrange with the mouse | Drag a pane by its header: onto another pane's edge to split there, its centre to swap, a tab to move there. Esc cancels. Dragging an app you are in parks it: click it to go back in |
| Close the pane | ⌘W |
| New tab | ⌘T |
| Hide ottospace | ⌘H |
| Quit | ⌘Q, or the window's close button. While a terminal runs something (a program, or a background job) it asks first: ↵, Q or ⌘Q again quits; Esc, any other key or shortcut cancels and does nothing else. Shells at a prompt do not count. Dock ▸ Quit and logout do not ask |
| Start empty next time | File ▸ Start Fresh. Otherwise the tabs, splits, the tab on show and the drawer come back after quitting (`restore_layout`) |
| Find an action by name | ⌘P: type part of its name, ↵ runs it. Or the menu bar |
| See what an app exposes to accessibility | Focus its pane, ⌘P, "inspector": its tree in a panel on the right, counted by role. Click a row to outline it |
| Switch tab | ⌘1 … ⌘9 · ⌘⇧] ⌘⇧[ · ⌃Tab |
| Close the tab and its panes | ⌘⇧W |
| Save the tab as a workbook | ⌘⇧S |
| Fill the screen | ⌘⌃F (again to restore) |

## The bar on the left (Tasks, Apps, Workbook, Repos, Marks)

| I want to | Do |
|---|---|
| Open a section | Click its icon on the rail |
| Use it from the keyboard | ⌘⌃E (the open section, or Apps), then `j` `k` move · ↵ open · ⇧↵ open below · Tab next section · Esc leave |
| Read the lists again | Close and open the section, or ⌘⌃E: Apps and Repos are read when they open |
| Add something | + in the bar's header: a new task in Tasks, the tab on show in Workbook, a repository in Repos (pick a folder that is one or holds some; it goes into `repos` in the settings) |
| Stop listing a repository | Point at its name in Repos: Remove. Only for one added as itself |
| Close the bar | × in its header |
| Click anything without the mouse | ⌘; or ⌘⌃G: every control gets a letter label (a × gets two), and so do the buttons and text fields of the focused app pane, in orange, two letters or more. Type it · ⌫ takes a letter back · Esc or a wrong letter leaves. An orange label presses that button in the app, or puts you in its text field |

## Apps

| I want to | Do |
|---|---|
| Put a running app's window in a pane | Apps, click the app (⇧-click: split down). Or ⌘E and pick it |
| Start an app that is not running | Apps, click it under "Not running" (pinned apps, then the Dock's) or under "All apps". Or ⌘E, type its name, ↵ |
| Keep an app at the top of that list | Hover its row, Pin. Or ⌘⌃E on Apps, then `p` (again unpins). It is written to `apps` in the settings |
| Stop a Dock app being suggested | Hover its row, Hide. Or ⌘⌃E, then `x`. Undo on the line at the top (six seconds), or `u` |
| Find an app among all installed | Apps, "All apps (N)", then click the filter or `/` in list mode, and type |
| Type in the app | Click its pane. The status bar turns `APP`; ottospace keys stop |
| Come back | Click ottospace itself, or the return key |
| Give the window back to macOS | Come back to ottospace first, then ⌘W on its pane: the window returns to where it was |
| Find a window lost off screen | `ottospace rescue` in a terminal, or Developer ▸ Bring All Windows Back |

The badge on an app is how many windows it has open. A new window of a hosted app gets its own pane.

## Workbooks

A workbook is a saved, named set of tabs, for a context that comes back. It is kept between runs.

| I want to | Do |
|---|---|
| Save the tab on show | ⌘⇧S, or + in the Workbook section's header. Type its name, ↵ |
| Add the tab on show to one | `Add tab` on its row, or ⌘⌃E, then `a` on it |
| Save every tab on the bar as one | File ▸ Save All Tabs as Workbook |
| Update a tab of it after changing it | ⌘⇧S again on that tab. Undo on the line at the top (six seconds) |
| Open it | Workbook, click it. Or ⌘⌃E, ↵. Its tabs open in order; one already open is kept |
| See, open, rename or remove one of its tabs | ▸ on its row (⌘⌃E: space), then the tab's row: click, Rename, Remove |
| Rename or delete it | Hover its row: Rename, Delete. Or ⌘⌃E, then `r`, `x`; `u` puts a deleted one back |

It keeps the splits, each app, and each terminal's directory, last 500 lines and the command
running in it. Opened, a terminal shows those lines above its prompt with the command typed on
it, not run: ↵ runs it. Not the page or document an app showed. A directory that is gone: the terminal starts in your home
folder. Kept in `~/.ottospace/workbooks`.

## Marks

A mark is a named way back to a pane or a whole tab. It is kept between runs.

| I want to | Do |
|---|---|
| Mark the focused pane / the tab on show | ⌘B / ⌘⇧B. Again takes the mark off. A marked pane or tab shows ◆ |
| Go to a mark | ⌘' then its letter · ⌘⌥1 … ⌘⌥9 for the first nine · click it in Marks on the rail |
| Give a mark a letter, rename it, remove it | In Marks: click its letter box, ✎ or ×. Or ⌘⌃E, then `m`, `r`, `x` |
| Put back a mark I took off | Undo on the line at the top of Marks (six seconds), or ⌘⌃E on Marks, then `u`. ⌘B again on the same pane or tab puts back the same mark |
| Reorder marks | ⌘⌃E on Marks, then ⇧J / ⇧K |

Open now (a dot in the list): its tab comes forward and its pane takes focus. Else it is opened
again: a terminal in the directory it was in when marked, an app's window by its title, a tab with
its splits. A mark whose directory is gone or whose app is not installed says so; a tab opens with
what can be opened, and its other panes are empty and say why. Marks are kept in
`~/.ottospace/marks.json`.

## Tasks

| I want to | Do |
|---|---|
| Start a task | ⌘N, type a line, ↵: its agent starts in the drawer on the right, beside your workspace. The + on the Tasks bar or the right rail opens the same box; its picks set where and who |
| Add a task for later | ⌘N, type a line, ⇧↵ |
| Talk to a task's agent | Click the task (or ↵ on it): its conversation opens in the drawer, started or gone on with. ⌘L moves the keyboard in and out; ⌘⇧L closes the drawer to the right rail; Stop ends the agent; drag its left edge to resize |
| Switch between running agents | Click a badge on the right rail, or ⌘⌃1 … ⌘⌃9. Hover a badge for what it is doing |
| See a task's tabs | Go to its tabs (⌘↵) in the drawer's header; with none, Claim this tab or New tab |
| Point at something for a task ("this message", "this error") | ⌃⌥⌘Space, from any app: the pane in use freezes as a picture and a draft opens on the right. Drag to circle, ⇧-drag a box, ⌥-drag a line, click to type a note. ↵, type what you want, ↵ starts it (⇧↵ for later). Esc leaves the picture; Esc in the title cancels the draft. The task keeps the pictures and the text under what you circled as its brief: [Capture](reference.md#capture) |
| Capture a second pane into the same draft | While drawing, click the other pane. Or press ⌃⌥⌘Space again |
| Give an agent a task's brief | In a terminal of the task: `ottospace ctl "{\"cmd\":\"task_brief\",\"id\":$OTTOSPACE_TASK}"` |
| Tag the task | In that line: `#project` · `>repo` · `@agent` · `/path` · `$skill` · `^reference` · `p1` … `p9`. A dropdown finishes the tag at the cursor: ↑↓ move, ↵ or Tab takes a row, Esc closes it. [All tags](reference.md#tags) |
| Add files to the task | Drag them from Finder onto the box, ⌘O, or copy in Finder and ⌘V: each becomes a `/path` |
| Point the task at a tab, a pane, a mark or an app | `^name` in the line, or ⌥-click the tab or pane while the box is open |
| A task's tabs | Side by side behind its badge on the one bar. Starting a task takes the tab on show (if it is no task's); ⌘T joins the task on show; drag a tab onto another group; palette: New Tab outside the Task, Remove Tab from Task; Claim tab on a row |
| The next task that needs me | ⌘⇧N: its conversation; the tab stays |
| See what needs me at a glance | A tab with no panes is the home screen: what goes there (Open an app ⌘E, Terminal ⌘D), up to three tasks under Needs you, and a node for each of the first 12 listed tasks. Left to right is the project; within a project the older task is farther; the colour is the state. No node sits under the text, so a small window shows fewer. Hover a node for its title; click a node or a row to open its conversation |
| See the inbox | Tasks on the rail: Needs you, Running, Later, Done (folded). Click a heading to fold it, or ⌘⌃E, move to it, ↵ or space |
| See tasks by project | Project in the Tasks header (State goes back): a group a `#project` with a bar of its states, then "no project". Today's work is under the list |
| See a task's time and tokens | Hover its row, or its badge on the right rail: started, agent time, workspace, tokens by model |
| See agent sessions that are no task (a Claude Code or Codex started by hand) | Tasks on the rail, under Other sessions (folded at first: click its heading; Today \| 7 days \| 30 days \| All on it picks how far back): runner, repository, the session's title, active or idle, what it does now, the model. A click opens its conversation in the drawer, gone on with unless it is at work in another terminal. Hover a row: Open, Adopt makes it a task (its conversation too), Hide removes it until the next start. [More](reference.md#sessions) |
| Snooze 1 hour / hold / dismiss / undo | ⌘⌃E on Tasks, then `s` / `h` / `x` / `u`. By mouse: the buttons on a row you hover; "Undo" then shows at the top for 6 seconds |

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

`calendar_ics`: your calendar's feed (an `https://` link or a file) for the [meeting timeline](reference.md#meeting-timeline): a hairline of the next `calendar_window_hours` (8) across the top, the next meeting beside the tab bar's buttons, a pulse from 10 minutes before, and at the start, if you are in no call, a banner with Join and Dismiss and a notification. `calendar_notify: false` turns alerts off; `calendar_notify_for: "links"` alerts only meetings with a meeting link.

`windows` rules name windows ottospace leaves alone, by `app`, `title` and `subrole`: [reference](reference.md#settings).

`OTTOSPACE_REPOS` (directories, `:` between them) wins over `repos`. `OTTOSPACE_HOME` names another folder than `~/.ottospace`.

## Not there yet

- Quitting ends what runs in terminals: tabs, splits and folders come back, with their last lines and the command offered on the prompt, not the programs. An app that is not running is not started: its pane offers `Open <App>` (`restore_apps: true` starts it).
- Filing a task starts nothing: no agent and no worktree. The task gets its first tab when you open it.
- A running app cannot be pinned from the Apps section: pin it while it is closed, or name it in `apps`.
