# ottospace cheat sheet

One page for daily use. ⌘/ shows the keys inside the app, the ones you have not learned first; those you know fold into `Known` (↵ opens it). Full detail: [reference.md](reference.md). Install and update: [install.md](install.md).

## Start

```sh
ottospace            # opens on the Repos list
```

- First run: grant your terminal Accessibility (ottospace will not start without it) and Screen Recording (for app panes), in System Settings ▸ Privacy & Security.
- Repos lists each repository's name over its checkouts, one line each: the branch, a dot on the repository's own folder. The chevron (or a click on the name) folds a repository; a count shows while folded. Point at a name: + makes a worktree (an existing branch, or a new one from HEAD), the trash stops listing it. Click a row: the workspace of that checkout (its default worktree, else one with a task there), else a new workspace for it, named for its branch, with a terminal there. A workspace's new terminals and tasks start in its default worktree; tasks of one workspace in different worktrees name theirs on their rows. A task whose agent makes a worktree and works there follows it (its workspace too, when it is its one task). A row a task works in carries the task's badge (Terminal, on hover, opens a terminal anyway).
- The status bar (bottom left) shows who has the keyboard when it is not the focused pane's: `APP`, `PICK`, `COMMAND`, `INSPECT`, `LIST`, `TASK`, `VISUAL`, `SNEAK`, `QUIT?`. Nothing there: normal. Its right end shows `● 2` when two tasks need you; the version is under About ottospace (⌘P, or the app menu) and the hover of the bar's empty middle.
- A coloured bar at the window's foot is a notice: red an error, amber a warning, blue information. × or Esc dismisses it; a settings problem offers Open settings.
- Stuck in an app? Click ottospace's rail or top bar. Or press the return key: F18, which most keyboards lack, so map Caps Lock to it (see [the return key](reference.md#the-return-key)) or start with `OTTOSPACE_RETURN_KEY=f13` (f13 to f20). The status bar names the key in use.

## Panes and tabs

| I want to | Press |
|---|---|
| Split with a new terminal | ⌘D right · ⌘⇧D down |
| Choose what goes in a new pane (terminal or app) | ⌘E (right) or ⌘⇧E (below), type to filter, ↵. Typing also offers installed apps that are not running |
| Edit the picker's filter or the new-task line | As any Mac text field: ⌥←/→ words, ⌘←/→ line, ⇧ selects, ⌘A, ⌘C/⌘X/⌘V, ⌘Z/⇧⌘Z, ⌥⌫ a word, ⌘⌫ the line; click, drag, double click |
| Move between panes | ⌘⌃ arrows or ⌘⌃ H J K L · ⌘] ⌘[ cycle |
| Move to the rails and back | The keys of the row above, on past the panes: left to the section list, then the rail; right to the conversation. On the rail ↑↓ move, ↵ opens, Esc goes back |
| Resize the pane | ⌘⌥ arrows or ⌘⌥ H J K L: the split moves that way · ⌘⌃= evens all out |
| Give an app the room its window needs | ⌘⌃0, or `fit` in the pane header: shown when the app refused the pane's size and a split can move |
| Swap the pane with its neighbour | ⌘⇧⌥ arrows or ⌘⇧⌥ H J K L |
| Move the pane to another tab | ⌘⇧⌥1 … ⌘⇧⌥9. US keyboard layout only (US, ABC): on others the keys do nothing |
| Read a pane's header | A terminal's directory, and its program while one runs (`app-api · cargo test`); an app's icon and window title |
| Rearrange with the mouse | Drag a pane by its header: onto another pane's edge to split there, its centre to swap, a tab to move there. Esc cancels. Dragging an app you are in parks it: click it to go back in |
| Close the pane | ⌘W |
| New tab | ⌘T |
| Hide ottospace | ⌘H |
| Quit | ⌘Q, or the window's close button. While a terminal runs something (a program, or a background job) it asks first: ↵, Q or ⌘Q again quits; Esc, any other key or shortcut cancels and does nothing else. Shells at a prompt do not count. Dock ▸ Quit and logout do not ask |
| Start empty next time | File ▸ Start Fresh. Otherwise the tabs, splits, the tab on show and the drawer come back after quitting (`restore_layout`) |
| Find an action by name | ⌘P: type part of its name, ↵ runs it. Or the menu bar |
| Edit the settings | ⌘, (Open Settings): `settings.json` in your editor, in a tab of its own |
| See what an app exposes to accessibility | Focus its pane, ⌘P, "inspector": its tree in a panel on the right, counted by role. Click a row to outline it |
| Switch tab (of the task on show) | ⌘1 … ⌘9 · ⌘⇧] ⌘⇧[ · ⌃Tab |
| Switch task or workspace (top row) | ⌘⌃1 … ⌘⌃9 · ⌘⌃] ⌘⌃[ |
| Go to Home (the house, first on the top row) | ⌘⌃1, or Go to Home in the palette |
| Switch conversation in a workspace of several tasks | The strip over the drawer (a tab a task; + adds one) · ⌘⌥] ⌘⌥[ |
| New workspace | + on the top row, or in the Workspaces section's header |
| Hide the task or workspace on show | ⌘⌃W, or its × on the top row. Its row in Tasks or Workspaces opens it again |
| Close the tab and its panes | ⌘⇧W |
| Save the tab into its workspace, else as a new one | ⌘⇧S |
| Fill the screen | ⌘⌃F (again to restore) |

## The bar on the left (Tasks, Calendar, Workspaces, Repos; Apps and Marks behind flags)

| I want to | Do |
|---|---|
| Open a section | Click its icon on the rail |
| Use it from the keyboard | ⌘⌃E (the open section, else Apps, or with it off the first), then `j` `k` move · ↵ open · ⇧↵ open below · Tab next section · Esc leave |
| Read the lists again | Close and open the section, or ⌘⌃E: Apps and Repos are read when they open |
| Add something | + in the bar's header: a new task in Tasks, a new empty workspace in Workspaces, a repository in Repos (pick a folder that is one or holds some; it goes into `repos` in the settings) |
| Stop listing a repository | Point at its name in Repos: the trash. Only for one added as itself; nothing on disk is touched. While a task works there it asks first (↵ unlists, Esc keeps it) |
| New worktree | Point at a repository's name in Repos: +, type a branch, ↵. It goes in `~/.ottospace/worktrees/<repo>/<branch>` (`worktrees_dir`) and opens as a terminal tab |
| Fold a repository | Its chevron, or ← / → in list mode |
| A worktree's pull request | Its `#number` at the row's end: muted draft, ✓ ready, amber waiting, red failing. Hover for checks, review and merge; click opens it (via `gh`, `repos_prs`) |
| Close the bar | × in its header |
| Click anything without the mouse | ⌘; or ⌘⌃G: every control gets a letter label (a × gets two), beside a small control and outlining it, and so do the buttons and text fields of the focused app pane, in orange, two letters or more. Type it · ⌫ takes a letter back · Esc or a wrong letter leaves. An orange label presses that button in the app, or puts you in its text field |

## Apps

The Apps section is behind the `apps_section` flag, off by default: turn it on in ⌘P ▸ Features….
Without it, ⌘E puts an app in a pane.

| I want to | Do |
|---|---|
| Put a running app's window in a pane | Apps, click the app (⇧-click: split down). Or ⌘E and pick it |
| Start an app that is not running | Apps, click it under "Not running" (pinned apps, then the Dock's) or under "All apps". Or ⌘E, type its name, ↵ |
| Keep an app at the top of that list | Hover its row, Pin. Or ⌘⌃E on Apps, then `p` (again unpins). It is written to `apps` in the settings |
| Stop a Dock app being suggested | Hover its row, Hide. Or ⌘⌃E, then `x`. Undo on the line at the top (six seconds), or `u` |
| Find an app among all installed | Apps, "All apps (N)", then click the filter or `/` in list mode, and type |
| Type in the app | Click its pane. The status bar turns `APP`; ottospace keys stop |
| Scroll the app | Wheel or trackpad over its pane: the window comes up (`PEEK`) and goes back when the pointer leaves or you stop. The first scroll is lost |
| Come back | Click ottospace itself, or the return key |
| Give the window back to macOS | Come back to ottospace first, then ⌘W on its pane: the window returns to where it was |
| Find a window lost off screen | `ottospace rescue` in a terminal, or Developer ▸ Bring All Windows Back |

The badge on an app is how many windows it has open. A new window of a hosted app gets its own pane.

## Workspaces

A workspace is a named set of tabs, for a context that comes back. It is kept between runs, and
listed in the Workspaces section whether it is open or not. The first item of the top row is
Home, a house: it holds no pane. What you open there (a pane, ⌘T, a Repos row) gets a new
workspace; tabs of nothing (a dismissed task's, a client's) go to the workspace selected last.

| I want to | Do |
|---|---|
| Make a new, empty one | + on the top row, or in the Workspaces section's header: "Workspace 2", one empty tab, selected. It never takes the tab on show |
| Take it off the top row | Its ×, or ⌘⌃W. Nothing in it stops; it stays listed, and its row opens it again |
| Save the tab on show | ⌘⇧S. In a workspace: into that one. Elsewhere: as a new one; type its name, ↵ |
| Add the tab on show to one | ⌘⌃E, then `a` on its row. Or File ▸ Add Tab to Workspace… |
| Save every tab on the bar as one | File ▸ Save All Tabs as Workspace |
| Update a tab of it after changing it | ⌘⇧S again on that tab. Undo on the line at the top (six seconds) |
| Open it | Workspaces, click it. Or ⌘⌃E, ↵. Its saved tabs open in order; one already open is kept. None saved: one empty tab |
| See, open, rename or remove one of its tabs | ▸ on its row (⌘⌃E: space), then the tab's row: click opens it; its … has Rename, Remove |
| Rename or delete it | Point at its row: … opens a menu with Rename, Delete. Or ⌘⌃E, then `r`, `x`; `u` puts a deleted one back |

A saved tab keeps the splits, each app, and each terminal's directory, last 500 lines and the command
running in it. Opened, a terminal shows those lines above its prompt with the command typed on
it, not run: ↵ runs it. Not the page or document an app showed. A directory that is gone: the terminal starts in your home
folder. Kept in `~/.ottospace/workbooks`.

## Marks

A mark is a named way back to a pane or a whole tab. It is kept between runs. Marks are behind
the `marks` flag, off by default: turn it on in ⌘P ▸ Features….

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

Which keys you use is counted in `~/.ottospace/keys.json` (your own presses only). Help ▸ Reset
Key Counts forgets it.

## Tasks

| I want to | Do |
|---|---|
| Start a task | ⌘N, type a line, ↵: its agent starts in the drawer on the right, beside your workspace. The + on the Tasks bar or the top row opens the same box; its picks set where and who |
| Add a task for later | ⌘N, type a line, ⇧↵ |
| Use the bar while writing a task | Click as usual: a rail icon or a fold leaves the box as it is; a click that goes elsewhere (a tab, a pane, a repository) puts the line aside until the next ⌘N |
| Write a message in your editor | ⌃G in the composer or the new-task box: `$VISUAL`/`$EDITOR` opens in the drawer on the text. Save and quit puts it back (not sent; ↵ sends); quit with an error keeps it as it was. A permission card meanwhile shows under the editor: click it to answer, ⌃G back |
| Talk to a task's agent | Click the task (or ↵ on it): its conversation shows in the drawer. Not running: Start agent, or Resume session once it ended (a click never starts one). ⌘L moves the keyboard in and out; ⌘⇧L closes the chat, or opens it again; Stop ends the agent (natively: Stop the Agent in the palette); drag its left edge to resize |
| Get a closed conversation back | ⌘⇧L, or click the agent chip at the right end of the tab row (the top row, with one tab): the badge and what the agent does (`Working · cargo test`, `Needs you`), amber while it needs you. A click on the selected space on the top row opens it too |
| See what an agent is doing | The dial under a native conversation's header: its calls by station (Shell, Files, Network…, each named), lit and pulsing while one runs (larger for more at once), the centre turning while the model thinks, a red dot for a failure since your last message, the ring for the context used. Hover for counts and tokens. A click or ⌘⇧A collapses it to a marker beside the badge |
| Chat with Claude Code in the drawer | ↵ send, ⇧↵ new line, Esc leaves; Esc Esc, or Stop at the composer's right while a turn runs, stops the turn. Once the agent ended, ↵ or Resume session (at the composer's right) goes on with its session; an ended agent says why, a stop and its resume are one thin divider, and Show details opens what its program printed. Hover a message: Copy, Copy as Markdown; a code block has its own Copy. An edit shows its diff under its call. A link opens on your click only. A permission card: `a` allow (this one request), `d` deny, `m` deny with a message, `s` show all (needed before `a` on a long one); ↵ does nothing there. `Chat \| Terminal` in the header switches to the CLI's TUI and back (`conversation_view` sets the default). The model and the folder the agent runs in are at the drawer's foot (with a mode that acts unasked: `haiku-5.5 · auto · ~/code/app`; hover it for the whole path); hover the title for mode and time |
| Switch between running agents | Click its task on the top row, or ⌘⌃1 … ⌘⌃9 |
| See a task's tabs | Click its row, or ⌘⇧N for the next that needs you: its space |
| Point at something for a task ("this message", "this error") | ⌃⌥⌘Space, from any app: the pane in use freezes as a picture and a draft opens on the right. Drag to circle, ⇧-drag a box, ⌥-drag a line, click to type a note. ↵, type what you want, ↵ starts it (⇧↵ for later). Esc leaves the picture; Esc in the title cancels the draft. The task keeps the pictures and the text under what you circled as its brief: [Capture](reference.md#capture) |
| Capture a second pane into the same draft | While drawing, click the other pane. Or press ⌃⌥⌘Space again |
| Give an agent a task's brief | In a terminal of the task: `ottospace ctl "{\"cmd\":\"task_brief\",\"id\":$OTTOSPACE_TASK}"` |
| Tag the task | In that line: `#project` · `>repo` · `@agent` · `/path` · `$skill` · `^reference` · `p1` … `p9`. A dropdown finishes the tag at the cursor: ↑↓ move, ↵ or Tab takes the highlighted row (↵ never starts a half-typed tag), Esc closes it. A tag's cap under the line types its mark. [All tags](reference.md#tags) |
| Add files to the task | Drag them from Finder onto the box, ⌘O, or copy in Finder and ⌘V: each becomes a `/path` |
| Run the agent in a folder | Type `~/`, `/` or (with a `>repo`) `./`, and take a folder from the dropdown: "runs in ~/notes/ ×" shows it. The path stays in the title |
| Point the task at a tab, a pane, a mark or an app | `^name` in the line, or ⌥-click the tab or pane while the box is open |
| A task's tabs | Its space, behind its badge on the bar. Starting a task (↵) in a workspace (one you made, or a task's) puts it there, tabs and all untouched: a workspace may hold several, sharing its folder. From Home it opens its own, with one empty tab; ⌘T joins the space on show; drag a tab onto another space; palette: Move Tab to… |
| The next task that needs me | ⌘⇧N: its space and chat |
| See what needs me at a glance | Home (⌘⌃1) is the home screen: a list of what goes there (Open an app ⌘E, Terminal ⌘D: click a row or press its key; a task's empty tab has the same list), up to three tasks under Needs you, and a node for each of the first 12 listed tasks. Left to right is the project; within a project the older task is farther; the colour is the state. No node sits under the text, so a small window shows fewer. Hover a node for its title; click a node or a row to open its conversation |
| See the inbox | Tasks on the rail: Needs you, Running, Later, Done (folded). Click a heading to fold it, or ⌘⌃E, move to it, ↵ or space |
| See tasks by project | Project in the Tasks header (State goes back): a group a `#project` with a bar of its states, then "no project". Today's work is under the list |
| See a task's time and tokens | Hover its row: started, agent time, workspace, tokens by model |
| See agent sessions that are no task (a Claude Code or Codex started by hand) | Tasks on the rail, under Elsewhere (folded at first: click its heading; the last 7 days, `sessions_range` for another): one line a row, its title (else repository) and runner; hover for what it does now, the model, when last active. A click shows its conversation in the drawer: Resume session goes on with it, unless it is at work in another terminal (then watched: its calls and edits, a message at a time). Hover a row: Track makes it a task (its conversation too), Hide removes it until the next start. [More](reference.md#sessions) |
| Snooze 1 hour / hold / dismiss / undo | ⌘⌃E on Tasks, then `s` / `h` / `x` / `u`. By mouse: Snooze and Done on a row you hover (hold is `h` only); "Undo" then shows at the top for 6 seconds |

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
| Select | Drag · double-click a word · triple-click a line · ⇧-click extends · Esc clears |
| Copy / paste | ⌘C / ⌘V: the pane pulses and says "Copied 3 lines". Text with a line break asks first: ↵ pastes |
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

`~/.ottospace/settings.json`, read at start. ⌘, (Open Settings) edits it in `$VISUAL`/`$EDITOR`; on
save the notice bar says what is wrong (`settings.json:3:1 trailing comma`) or which keys need a
restart, and the rest applies at once:

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
  "windows": [{ "app": "us.zoom.xos", "title": "Zoom Meeting", "do": "ignore" }],
  "features": { "marks": false, "apps_section": false }
}
```

A mistake in a key (one that does not exist, or a wrong value) shows in the notice bar: that key
takes its default and the others still apply. If the file is not a JSON object at all, every
setting takes its default.

`sessions`: `false` reads no agent session files. `session_dirs` names where Claude Code and Codex keep them, if not `~/.claude` and `~/.codex`. `session_quiet_minutes`: a task adopted from a session moves to Needs you after the session has been quiet this long (default 3; 0 never). `agent_models`: the models `@` offers after each runner, as its CLI names them, in place of the built-in list.

`apps`: the pinned apps, by name or bundle path. `apps_hidden`: Dock apps not to suggest. Pin, Unpin and Hide in Apps write these two and leave the rest of the file as it is. `suggest_dock_apps`: `false` lists no Dock apps.

`features`: [feature flags](reference.md#features), each `true` or `false`; ⌘P ▸ Features… writes them. A flag is temporary and about readiness; a preference is a normal setting.

`capture_key`: the key that captures the pane in use (`"off"` for none). `capture`: `keep_images` `false` writes no picture; `ax` `false` reads no accessibility tree for a capture.

The Calendar section (second on the rail, only while `calendar_ics` is set) lists a day's meetings, one line each; `‹ ›` in its header (← → in list mode) go a day back and on, 7 each way, `Today` (`t`) comes back; ↵ or `Join` opens a meeting's link. Its rail cell has an amber dot near a meeting, a filled badge while the banner is up.

`calendar_ics`: your calendar's feed (an `https://` link or a file) for the [meeting timeline](reference.md#meeting-timeline): a band of the workday (`calendar_workday`, `"09:00-17:00"`) above the status bar, hover a block for its details (now's tick: the next meeting and when the feed was read), a pulse from 10 minutes before, and at the start, if you are in no call (or still in an earlier one), a banner with Join and Dismiss and a notification. `calendar_notify: false` turns alerts off; `calendar_notify_for: "links"` alerts only meetings with a meeting link; `calendar_email` (your address in the feed) leaves out meetings you declined; `calendar_hide_free: true` leaves out free entries (a feed without attendees); `calendar_colors` colours by title (`[{"match": "1:1", "color": "teal"}]`). On the band calls sit in lanes over faint blocks of time held; a block that starts with `Focus` or `Deep Work` and has no call link is green.

`windows` rules name windows ottospace leaves alone, by `app`, `title` and `subrole`: [reference](reference.md#settings).

`OTTOSPACE_REPOS` (directories, `:` between them) wins over `repos`. `OTTOSPACE_HOME` names another folder than `~/.ottospace`.

## Not there yet

- Quitting ends what runs in terminals: tabs, splits and folders come back, with their last lines and the command offered on the prompt, not the programs. An app that is not running is not started: its pane offers `Open <App>` (`restore_apps: true` starts it).
- Filing a task starts nothing: no agent and no worktree. The task gets its first tab when you open it.
- A running app cannot be pinned from the Apps section: pin it while it is closed, or name it in `apps`.
