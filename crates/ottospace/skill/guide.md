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
| `mode` | `normal`, `app` (a hosted app has the keyboard), `pick` (the picker is open) or `visual` (selecting terminal text with the keyboard) |
| `state.panes` | Every pane of every tab: `id`, `kind`, and `cwd` for a terminal opened by `open_repo`. `kind` is an object: `{"type":"terminal"}`, `{"type":"empty"}` or `{"type":"app","window":N,"title":"..."}` |
| `state.tabs`, `state.active` | The tabs (`id`, `title`) and the index of the one showing. Requests take the index, not the `id` |
| `tab_panes` | Pane ids of each tab, in tab order |
| `state.focused` | The pane that splits and keys act on |
| `state.live` | The app pane that has the keyboard, if any |
| `state.picker` | Null, or the open picker: `axis`, `new_tab`, `items` (`{"type":"terminal"}` or `{"type":"app","name":"..."}`), `query`, and `selected` (an index into the items that match `query`) |
| `state.keys` | True while the key reference covers the window |
| `state.sidebar` | The open rail section, or null |
| `terminals.<id>` | `cols`, `rows`, and `lines`: the visible text of that terminal pane. Also `history` (lines scrolled off the top), `display_offset` (how far the view is scrolled back; 0 is the bottom), `selection` (the selected text, or null) and `visual_cursor` |
| `copied` | The text of the last copy |
| `apps` | Running apps that `open_app` can host: `name`, `windows` (how many it has open) and `icon` (true if its icon was read) |
| `state.repos` | Repositories found on disk: `name`, `path`, `branch`, and `worktrees` (each `name`, `path`, `branch`) |
| `ax_overlay` | Null, or the accessibility overlay's last read: `pane`, `targets` (`label`, `frame` in window coordinates, `source`), `count`, `truncated`, `millis`, `error` |
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
repositories in the directories named by `$OTTOSPACE_REPOS` (default `~/code` and `~/notes`), each
with its git worktrees. `open_repo` opens a terminal whose shell starts in a repository or
worktree: it fills the focused pane if that is empty, else splits it. `path` must be a `path` from
`state.repos`; any other path is refused. A tab with no panes takes the repository's name.

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

A shell started by ottospace has `OTTOSPACE_PANE` (its pane id) and `OTTOSPACE_TAB` (its tab's
`id` in `state.tabs`, not the index) in its environment. A program in a pane can use them to
name its own pane.

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

`open_app` hosts a running app's main window in a pane. `enter` gives that app the keyboard;
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
