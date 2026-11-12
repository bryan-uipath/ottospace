# Driving ottospace

ottospace tiles terminals and real app windows in tabs and splits. This guide is printed by
`ottospace skill`, so it always matches the installed version.

You drive it with one command. It sends one JSON request to the running app and prints one JSON
reply:

```sh
ottospace ctl '{"cmd":"state"}'
```

- Exit status 0: the request was understood, and the reply is the full state after it.
- Exit status 1: the request was refused (`{"ok":false,"error":"..."}`), or no ottospace is running.
- Exit status 2: `ottospace` was called wrongly (for example `ctl` with no request).
- Unknown commands and unknown fields are refused. Do not guess field names.
- **Exit 0 does not mean it happened.** A request is ignored, with no error, when it names a pane
  or tab that no longer exists, or while the picker or the key reference is open
  (`state.picker` is not null, or `state.keys` is true). Read the reply and check that the state
  changed the way you meant.

## Rules

- **Look before you act.** Start with `state`. Pane ids and tab indexes come from it.
- **`type` is keyboard input to a shell.** It can run anything the user can. Send only commands
  the user asked for, and end a command with `\r` to run it.
- **Type only into panes you opened, and always name the pane.** Focus is shared with the user,
  who may click elsewhere between your requests. `type` without `pane` goes to whatever is
  focused at that instant. `type` also works on panes in background tabs.
- **Name what you close.** `close` without `pane` closes the focused pane; `close_tab` without
  `index` closes the tab the user is looking at. Pass the id or index every time.
- **`focus` and `enter` on a pane in another tab switch the tab the user sees.**
- **Other panes are not yours to read aloud.** Every reply includes the visible text of every
  terminal pane, which may show secrets. Use only the lines of panes you opened; do not quote
  or store the rest.
- **An app's accessibility tree is that app's screen.** `ax_tree` returns titles, labels and
  field values of whatever the app shows, which may include messages, documents or secrets.
  Treat it like other panes' text: use only what the user's task needs.
- **`open_app` and `enter` move the user's real windows.** Use them only when the user asked to
  bring an app into ottospace.
- **Do not send `quit`, `close` or `close_tab`** unless the user asked. Closing a terminal pane
  ends its shell; closing a tab closes every pane in it.
- **Read output with `wait` and `capture`.** After `type`, `wait` for a line that marks the end,
  then `capture` the pane. Both read one pane only, so they do not return other panes' text.

## State

| Field | Meaning |
|---|---|
| `mode` | `normal`, `app` (a hosted app has the keyboard), `pick` (the picker is open), `visual` (selecting terminal text with the keyboard), `list` (the secondary bar has the keyboard), `compose` (the new-task box is open) or `sneak` (labels are on ottospace's controls; a typed label activates one) |
| `state.list` | Null, or the index of the selected row of the open section: the secondary bar has the keyboard (`mode` is `list`, or `pick` while the picker is open over it) |
| `state.panes` | Every pane of every tab: `id`, `kind`, and `cwd` for a terminal opened by `open_repo`. `kind` is an object: `{"type":"terminal"}`, `{"type":"empty"}` or `{"type":"app","window":N,"title":"..."}` |
| `state.tabs`, `state.active` | Every tab (`id`, `title`, and `task`: the id of the task it belongs to, or null) and the index of the one showing. Requests take the index, not the `id` |
| `state.current_task` | The task whose tabs are on the user's tab bar, or null for the tabs of no task. Tabs of other tasks keep running, off the bar |
| `tab_panes` | Pane ids of each tab, in tab order |
| `state.focused` | The pane that splits and keys act on |
| `state.layout` | The active tab's split tree, or null: `{"leaf":3}` or `{"split":{"axis":"right","ratio":0.5,"a":..,"b":..}}`. `ratio` is the share of the split that `a` (the left or top side) takes |
| `state.live` | The app pane that has the keyboard, if any |
| `state.picker` | Null, or the open picker: `axis`, `new_tab`, `items` (`{"type":"terminal"}` or `{"type":"app","name":"..."}`), `query`, and `selected` (an index into the items that match `query`) |
| `state.keys` | True while the key reference covers the window |
| `state.sidebar` | The open rail section, or null |
| `terminals.<id>` | `cols`, `rows`, and `lines`: the visible text of that terminal pane. Also `history` (lines scrolled off the top), `display_offset` (how far the view is scrolled back; 0 is the bottom), `selection` (the selected text, or null) and `visual_cursor` |
| `copied` | The text of the last copy |
| `version` | The build, as the status bar shows it: version and the first seven characters of the commit (`0.1.0 · a4a2582`), or `0.1.0 · local` for a build from source |
| `font`, `cell` | The terminal font in use (`name`, `size`) and the size of one character cell `[w, h]` in points |
| `notice` | Null, or the problem the status bar shows (a mistake in the settings, an app that could not be hosted). It is not `error`, which means the request failed |
| `closed_apps` | Apps the user's settings name that are not running. `open_app` starts one |
| `apps` | Running apps that `open_app` can host: `name`, `windows` (how many it has open) and `icon` (true if its icon was read) |
| `state.tasks.list` | Every task: `id`, `title`, `state`, `priority`, `snoozed_until`, `dismissed`, `project` (from a `#tag`, or null), `repo` (a repository path, or null), `agent`, `paths`, `resolved` and `skills` |
| `compose_read` | True once the tags of the line in the new-task box have been looked up (names and paths are read on another thread, a moment after the line changes). Wait for it before pressing ↵ for the user |
| `state.compose` | Null, or the text in the new-task box while it is open |
| `inbox`, `needs_you` | The tasks the Tasks section lists, in order, each with its `group`; and how many are under `needs_you` |
| `home` | Null while the tab on show has a pane. Else the home screen: `nodes` (one for each of the first 12 inbox tasks: `task`, `group`, `lane` (its project, by name; tasks of no project share the last), `depth` (0 is the newest task; in a lane the older task is drawn farther) and `crossing`, the `[x, z]` of the lattice it is drawn on; no node is under the text, so a small window has fewer), `needs_you` (up to 3 tasks) and `counts` for each group |
| `home_drawn` | What the home screen drew, in window points: `area`, `panel` (the new-task line with its list) and `nodes` (each node's click target `[x, y, w, h]` by task id). `lattice_builds` and `lattice_paints` count how often lattice lines were built and painted; a frame that moves nothing paints without building |
| `state.repos` | Repositories found on disk: `name`, `path`, `branch`, and `worktrees` (each `name`, `path`, `branch`) |
| `ax_overlay` | Null, or the accessibility overlay's last read: `pane`, `targets` (`label`, `frame` in window coordinates, `source`), `count`, `truncated`, `millis`, `error` |
| `sneak` | Null, or sneak mode's labels: `typed` (the letters so far) and `targets`, each `label`, `frame` (`x`, `y`, `w`, `h` in window points) and `source` (`{"type":"own","id":"rail:tasks"}`) |
| `bounds.<id>` | Where each pane of the active tab is drawn: `[x, y, w, h]` |

## Requests

Panes:

```sh
ottospace ctl '{"cmd":"split"}'
ottospace ctl '{"cmd":"split","axis":"down","kind":"terminal"}'
ottospace ctl '{"cmd":"focus","pane":2}'
ottospace ctl '{"cmd":"focus_dir","dir":"left"}'
ottospace ctl '{"cmd":"focus_cycle","delta":1}'
ottospace ctl '{"cmd":"close","pane":2}'
```

`split` puts a new pane beside the focused one: `axis` is `right` (default) or `down`; `kind` is
`terminal` (default) or `empty`. `dir` is `left`, `right`, `up` or `down`.

Arranging panes:

```sh
ottospace ctl '{"cmd":"resize","dir":"left"}'
ottospace ctl '{"cmd":"equalize"}'
ottospace ctl '{"cmd":"swap","dir":"right"}'
ottospace ctl '{"cmd":"move_pane","tab":1}'
ottospace ctl '{"cmd":"move_pane","pane":3,"tab":0}'
ottospace ctl '{"cmd":"drop_pane","source":3,"target":1,"zone":"left"}'
ottospace ctl '{"cmd":"drop_pane","source":3,"target":1,"zone":"center"}'
```

- `resize` moves one border of the focused pane 5% of its split toward `dir`: the pane's right
  (or bottom) border, or for the last pane of a row (or column) its left (or top) one. Each side
  of a split keeps at least 10%; at that limit nothing moves. Read the result from
  `state.layout` (`ratio`) or `bounds`. `equalize` evens out every split of the active tab.
- `swap` trades the focused pane with its neighbour on that side (the pane `focus_dir` would go
  to). The pane keeps focus.
- `move_pane` moves a pane (default: the focused one) to another tab, beside that tab's focused
  pane, and shows that tab with the pane focused. `tab` is the place on the tab bar, from 0: among
  the tabs of `state.current_task` only, so with no task open it equals the index in
  `state.tabs`. A tab left with no panes stays, empty. `$OTTOSPACE_TAB` in a moved terminal
  still names the tab its shell started in.
- `drop_pane` does what dragging pane `source` by its header onto pane `target` does. `zone` is
  `left`, `right`, `top` or `bottom`: `target` is split in two and `source` takes that side. Or
  `center`: the two trade places. `source` takes focus and its tab comes forward. It fails if
  either pane does not exist or they are the same pane. Unlike `move_pane`, it is not limited to
  the tabs on the bar: the panes may be in different tabs, also tabs of another task, and then
  that task's tabs come on show (`state.current_task` changes). A pane dropped where it already
  is keeps its split's `ratio`. The user's own drag parks a live app; `drop_pane` leaves a live
  `source` live.

During a pane drag `drag` in the state is `{"target":1,"zone":"left"}`: where a release would
drop it, both `null` over anything that is no drop zone. Otherwise `drag` is `null`.

For tests only: the pointer. `mouse` and `drag` answer only when ottospace runs under
`OTTOSPACE_E2E`; otherwise they are refused. To act, use the requests above (`drop_pane`,
`focus`, `close`, `enter`).

```sh
ottospace ctl '{"cmd":"mouse","action":"down","x":300,"y":60}'
ottospace ctl '{"cmd":"mouse","action":"move","x":600,"y":400}'
ottospace ctl '{"cmd":"mouse","action":"up","x":600,"y":400}'
ottospace ctl '{"cmd":"mouse","action":"down","x":300,"y":60,"clicks":2,"modifiers":["shift"]}'
ottospace ctl '{"cmd":"drag","from":[300,60],"to":[600,400],"steps":8}'
```

- `mouse` is one left-button pointer event at `x`, `y`: points from the top-left of ottospace's
  window, the same as every rect in the state (`bounds`, `frames`, `headers`, `closes` and
  `tabs`: each drawn pane's header and its ×, by pane id, and each tab on the bar, by place).
  `action` is `down`, `move` or `up`; `modifiers` are `cmd`, `ctrl`, `alt`, `shift`. A `move`
  between a `down` and its `up` is a drag. `drag` is a press at `from`, `steps` moves (default
  8) and a release at `to`.
- Caps: `clicks` is at most 3, `steps` is 1 to 256.
- A `down` must be followed by an `up`. Esc ends the hold, and so does 5 s with no `mouse`
  request: the button is released outside the window, which drops nothing.
- **The event never leaves ottospace.** It is handed to ottospace's window inside the app: it
  is not posted to the system, the real pointer does not move, and no other app's window gets
  it, a hosted app that is live over its pane included. But a click does what the control under
  it does, with the same effects outside ottospace as `open_app`, `enter`, `leave` and `close`:
  a click on a parked app pane raises that app's window. Only the replay of the click inside
  the app is left out.

Terminals:

```sh
ottospace ctl '{"cmd":"type","text":"cargo test\r"}'
ottospace ctl '{"cmd":"type","pane":3,"text":"q"}'
```

`type` goes to the focused pane unless `pane` is given; give it. It fails if that pane is not a
terminal. Pane ids are never reused.

Repositories:

```sh
ottospace ctl '{"cmd":"repos"}'
ottospace ctl '{"cmd":"open_repo","path":"/Users/me/code/app"}'
ottospace ctl '{"cmd":"open_repo","path":"/Users/me/code/app-2","axis":"down"}'
```

`repos` reads the list from disk again; it is `state.repos` in every reply. It holds the
repositories in the directories named by `$OTTOSPACE_REPOS`, else by `repos` in the user's
`~/.ottospace/settings.json` (default `~/code` and `~/notes`), each
with its git worktrees. `open_repo` opens a terminal whose shell starts in a repository or
worktree: it fills the focused pane if that is empty, else splits it. `path` must be a `path` from
`state.repos`; any other path is refused. A tab with no panes takes the repository's name.

Tasks (the inbox in the Tasks section):

```sh
ottospace ctl '{"cmd":"task_add","title":"Fix the zoom flicker"}'
ottospace ctl '{"cmd":"task_add","title":"Review the retry fix","priority":1,"state":"needs_you","tab":2}'
ottospace ctl '{"cmd":"task_add","title":"Review the export PR","agent":"codex","paths":["/src/export"],"skills":["code-review"]}'
ottospace ctl '{"cmd":"task_new","line":"fix zoom flicker #canvas >flow-workbench @claude p1"}'
ottospace ctl '{"cmd":"task_new","line":"review the export PR >flow-workbench @codex $code-review /src/export"}'
ottospace ctl '{"cmd":"compose","text":"reply to Ana about the retry bug #support"}'
ottospace ctl '{"cmd":"task_set_state","id":1,"state":"needs_you"}'
ottospace ctl '{"cmd":"task_list"}'
ottospace ctl '{"cmd":"task_pop"}'
ottospace ctl '{"cmd":"task_open","id":1}'
ottospace ctl '{"cmd":"task_leave"}'
ottospace ctl '{"cmd":"task_snooze","id":1,"seconds":3600}'
ottospace ctl '{"cmd":"task_hold","id":1}'
ottospace ctl '{"cmd":"task_dismiss","id":1}'
ottospace ctl '{"cmd":"task_undo"}'
```

A task has a `title`, a `priority` (1 is most urgent, default 3) and a `state`. A task has its
own tabs: `state.tabs[].task` says which task a tab belongs to. `"tab"` on `task_add` gives the
task an open tab (a tab `id`): do that for the tab you work in, so the user finds your work
under the task. `$OTTOSPACE_TAB` is the tab your shell started in; the pane may have been moved
since, so read the tab that holds `$OTTOSPACE_PANE` now (below). It must be a tab of no task; a tab that
is gone or that another task has is refused. To file a follow-up from inside a task, leave `tab` out.
States: `captured` (default), `triaged`, `running`, `needs_you`, `review`, `stale`, `parked`,
`done`, `cleaned`. The inbox lists tasks in groups, in this order: `needs_you` (states
`needs_you`, `stale`), `running` (`running`, `review`), `captured` (`captured`, `triaged`),
`on_hold` (`parked`), `done`. In a group: priority, then oldest first. `cleaned` tasks are not listed.

- `task_new` files a task from one line, the way the user's new-task box (⌘N) reads it. Tags:

  | Tag | Means | Counts when |
  |---|---|---|
  | `#project` | the project the task serves | always |
  | `>repo` | the repository it works in, from `state.repos` by name | the repository (or worktree) is listed |
  | `@agent` | who works it | `claude`, `codex`, or a file `agents/<name>.md` in the user's folder |
  | `/path` | a path it is about: a word starting with `/` or `~/` | the file or directory exists, at the root or inside the `>repo` |
  | `$skill` | a skill to load for it | a skill folder of that name exists (the runners' `skills/`, the repository's `.claude/skills`) |
  | `p1` to `p9` | the priority | always |

  The rest is the title. A tag whose name is not known stays in the title where it was typed,
  so `ask @ana`, `costs $5` and `use > 3 workers` are safe. One `#project`, `>repo` and `@agent`
  per task: the last one counts. Every `/path` and `$skill` is kept. `@name` no longer names a
  repository. The task keeps them as `repo` (a path), `agent`, `paths`, `skills`. Nothing acts
  on `agent`, `paths` or `skills` yet: they are stored and shown. It replies like `task_add`.
  A line with no title, or longer than 500 characters, is refused.
- `task_add` takes `agent` and `skills` only if `task_new` would know the names (`claude`,
  `codex`, a saved agent; an installed skill): any other is refused, and the error names it.
  They are kept as their file or folder spells them. `paths` are kept as given, without
  checking them (at most 20 of each, 200 characters each).
- A task's `paths` are as typed. `resolved` holds, in the same order, the absolute path each
  was found at when `task_new` or the box filed it (empty from `task_add`). Nothing has checked
  either since: treat both as text, not as places safe to read or write.
- `compose` opens the new-task box on the user's screen with `text` in it (at most 500
  characters). Nothing is filed until the user presses ↵: use it to offer a draft, and
  `task_new` to file one yourself. It is refused if the box is open already or cannot open
  (the picker, the key reference or visual mode is in the way): the draft was not shown.
  While the box is open `mode` is `compose` and `state.compose` is its text.
- `task_add` replies `{"ok":true,"task":{..}}`, not the state: the new task, with its `id`.
  Priorities outside 1 to 9 are moved into that range.
- **Report your own state.** If the user gave you a task id, send `task_set_state` with `running`
  while you work and `needs_you` when you wait for the user. Do not set the state of other tasks.
  A task hidden by the user comes back only when it moves under `needs_you` from another group.
  Sending `needs_you` again for a task that already waits changes nothing: report `running`
  when you start again, so the next `needs_you` is news.
- `task_list` replies `{"ok":true,"inbox":[..]}`: each row is a task plus its `group`. The same
  rows are `inbox` in the state, with the count `needs_you`.
- `task_open` puts a task's tabs on the user's tab bar; `task_leave` shows the tabs of no task
  again. Both change what the user sees: send them only when asked. The first time a task is
  opened it gets a tab named after it, with a terminal in its `repo` if it has one. `select_tab`
  on a tab of another task also brings that task's tabs to the bar.
- `task_pop` is `task_open` for the first task under `needs_you`. It replies
  `{"ok":true,"task":{..},"opened":true}`. `"task":null`: nothing needs the user.
  `"opened":false`: the view did not change (the picker, the new-task box or the key reference
  is open, or the user is in visual mode).
- `task_snooze`, `task_hold` and `task_dismiss` are the user's moves; send them only when asked.
  A snoozed or dismissed task leaves the inbox. A snoozed task returns when its time is up;
  a dismissed task does not. Either returns when it moves under `needs_you`. `task_hold` sets
  the state to `parked`; a `done` or `cleaned` task cannot be held. `task_undo` takes back the
  last of the three, one step, unless the task's state was reported since. A request for a task
  id that does not exist is refused.
- A task is not ended by closing its tabs. If all of them are closed, it gets a fresh first tab
  the next time it is opened.
- Tasks are kept on disk (`~/.ottospace/tasks`, one file each) and come back at the next start,
  without their tabs. Change tasks through these requests, not by editing the files: the running
  app does not read them again.

Reading a terminal pane:

```sh
ottospace ctl '{"cmd":"capture","pane":3}'
ottospace ctl '{"cmd":"capture","pane":3,"history":true}'
ottospace ctl '{"cmd":"wait","pane":3,"pattern":"^done-7f3a$"}'
ottospace ctl '{"cmd":"wait","pane":3,"pattern":"^done-7f3a-[0-9]+$","timeout_ms":60000}'
```

Neither replies with the state. `capture` replies `{"ok":true,"pane":3,"text":".."}`: the bottom
screen, and with `history` everything scrolled off above it too. A wrapped line is one line.
`wait` replies `{"ok":true,"pane":3,"line":".."}` with the first line that matches the regular
expression `pattern`, and fails after `timeout_ms` (default 10000, at most 600000) or if the pane
closes. Each line is matched alone, so `^` and `$` are the ends of a line and a pattern cannot
span lines. While a full-screen program (vim, less) is up, both requests see only its screen.
`wait` reads the history too, so:

- Text already in the pane matches at once, also from an earlier run. Put a token you make up
  for this run in the marker (`7f3a` above), not only a fixed word.
- The command you typed is in the pane too. Anchor the marker with `^` so the echoed command
  line (which starts with the prompt) does not match.

A shell started by ottospace has `OTTOSPACE_PANE` (its pane id) and `OTTOSPACE_TAB` (the `id`,
not the index, of the tab it started in) in its environment. A program in a pane can use
`OTTOSPACE_PANE` to name its own pane. A pane can be moved to another tab, so `OTTOSPACE_TAB`
may be out of date. The `id` of the tab that holds the pane now:

```sh
ottospace ctl '{"cmd":"state"}' | jq --argjson pane "$OTTOSPACE_PANE" '.state.tabs[.tab_panes | map(any(. == $pane)) | index(true)].id'
```

Scrolling back and selecting text in a terminal pane:

```sh
ottospace ctl '{"cmd":"scroll","pane":3,"lines":40}'
ottospace ctl '{"cmd":"mouse_down","pane":3,"x":2,"y":5,"clicks":2}'
ottospace ctl '{"cmd":"mouse_drag","pane":3,"x":20,"y":5}'
ottospace ctl '{"cmd":"key","keys":"cmd-c"}'
```

`scroll` turns the wheel: positive `lines` scrolls back into `history`, so output that left the
screen can be read from `lines` again. `mouse_down` presses at a cell (`x`, `y` from the pane's
top-left; `clicks` 2 selects a word, 3 a line) and `mouse_drag` extends the selection; read it
from `terminals.<id>.selection`. `key` presses a key combination as the keyboard would, through
the app's own bindings, so it acts on the focused pane: check `state.focused` first.

Tabs:

```sh
ottospace ctl '{"cmd":"new_tab"}'
ottospace ctl '{"cmd":"select_tab","index":0}'
ottospace ctl '{"cmd":"cycle_tab","delta":1}'
ottospace ctl '{"cmd":"close_tab","index":1}'
```

Apps:

```sh
ottospace ctl '{"cmd":"open_app","name":"Safari","axis":"right"}'
ottospace ctl '{"cmd":"enter","pane":4}'
ottospace ctl '{"cmd":"leave"}'
```

`open_app` hosts a running app's main window in a pane. For an app in `closed_apps` it starts
the app first and hosts the window when it appears, so the reply comes before the pane exists:
read `state` again. `enter` gives that app the keyboard;
`leave` takes it back.

Reading an app pane's controls (read-only; nothing is pressed):

```sh
ottospace ctl '{"cmd":"ax_tree","pane":4}'
ottospace ctl '{"cmd":"ax_tree","pane":4,"depth":20,"nodes":500}'
ottospace ctl '{"cmd":"ax_overlay"}'
ottospace ctl '{"cmd":"ax_overlay","refresh":true}'
```

`ax_tree` replies with the app window's accessibility tree, not the state:
`{"ok":true,"pane":4,"window":{"x":..,"y":..,"w":..,"h":..},"millis":..,"tree":{"root":{..},"count":N,"truncated":{"depth":false,"nodes":false,"timeout":false}}}`.
Each node has `role` (e.g. `AXButton`), and when present `subrole`, `title`, `description`,
`value` (cut to 120 characters), `frame` (global screen points), `enabled`, `actions` and
`children`. Limits: `depth` (default 40) and `nodes` (default 3000); a slow app ends the read
early. Any `truncated` flag means the tree is partial. Electron and Chrome apps expose little more
than their window buttons. `ax_overlay` toggles a debug overlay that labels the focused app
pane's controls; `refresh` reads again. Both fail on a pane that is not an app.

Picker, bars and the rest:

```sh
ottospace ctl '{"cmd":"picker","axis":"right","new_tab":false}'
ottospace ctl '{"cmd":"picker_query","query":"term"}'
ottospace ctl '{"cmd":"picker_move","delta":1}'
ottospace ctl '{"cmd":"picker_choose"}'
ottospace ctl '{"cmd":"picker_close"}'
ottospace ctl '{"cmd":"section","section":"apps"}'
ottospace ctl '{"cmd":"keys"}'
ottospace ctl '{"cmd":"quit"}'
```

`picker_choose` opens the selected match, or the match at `"index":N`. `section` is `apps`,
`workbook`, `tasks` or `repos`; the open section's name closes the bar.
`keys` shows or hides the key reference.

Sneak mode: acting on ottospace's own controls as a click would, by a typed label.

```sh
ottospace ctl '{"cmd":"sneak"}'
ottospace ctl '{"cmd":"key","keys":"a s"}'
ottospace ctl '{"cmd":"key","keys":"escape"}'
```

`sneak` puts a letter label on every control ottospace draws and replies with them in
`sneak.targets`. Find the control by `source.id`, then send its `label` with `key`, one letter
per keystroke: the control acts as if clicked and the mode ends. A first letter of a two-letter
label only narrows; `backspace` takes it back; `escape` leaves with nothing done. So does a letter
that goes on with no label, and the keys after it in the same `key` request are dropped, as are
those after a spelled label. Keys that are not letters do nothing. Ids: `rail:apps`
(`workbook`, `tasks`, `repos`), `bar:refresh`, `bar:close`, `app:<name>`, `repo:<path>`,
`task:<id>`, `tab:<tab id>`, `tab-close:<tab id>`, `top:new_tab`, `top:split_right`,
`top:split_down`, `top:task_leave`, `header:<pane>` (focus), `close:<pane>`, `pane:<pane>` (focus;
an app pane gets the keyboard, as `enter`), `home:new_task`, `home-row:<task>`, `home-node:<task>`.
A `close:` or `tab-close:` label always has two letters.

The labels are those of a frame painted after the request came. `sneak` fails with "the window
has not been drawn" if no frame is painted (a hidden or minimised window), and unless `mode` is
`normal` or `list`: send `leave` or close the box first. If the mode is already on it stays on,
and the reply has the labels as they are. Starting it hides the accessibility overlay.

Only what is drawn has a label: a background tab's panes do not. While the mode is on the labels
follow what is drawn: a control that moves keeps its letters, one that is no longer drawn loses
its label, and a new one gets none. News does not end the mode (an agent's `task_add`, a shell
that exits); a request that is a move does (`focus`, `split`, `select_tab`). So read
`sneak.targets` from the latest reply before you type a label.

Controls inside a hosted app's window get no label. Sneak presses no control inside another app and sets no value; entering an app pane raises its window, as a click on the pane does. A request with its own name
(`focus`, `select_tab`, `section`) is simpler when there is one; `close` and `tab-close` labels end
shells, as `close` and `close_tab` do.

## Recipes

Run a command in a new terminal beside the current pane, and read its output:

```sh
ottospace ctl '{"cmd":"state"}'                                    # note the pane ids that exist
ottospace ctl '{"cmd":"split","axis":"right"}'                    # reply: state.focused is the new pane, say 3
ottospace ctl '{"cmd":"type","pane":3,"text":"cargo test; echo done-7f3a-$?\r"}'   # name the pane: focus may have moved
ottospace ctl '{"cmd":"wait","pane":3,"pattern":"^done-7f3a-[0-9]+$","timeout_ms":600000}'
ottospace ctl '{"cmd":"capture","pane":3,"history":true}'
```

`7f3a` is a token made up for this run; the number after it is the command's exit status.

Check that the id from the split reply was not in the first `state`. If it was, the split was
ignored (the picker or key reference was open): do not type.

Open a second workspace for a different job:

```sh
ottospace ctl '{"cmd":"new_tab"}'
ottospace ctl '{"cmd":"split"}'
```
