# Driving ottospace

ottospace tiles terminals and real app windows in tabs and splits. This guide is printed by
`ottospace skill`, so it always matches the installed version.

You drive it with one command. It sends one JSON request to the running app and prints one JSON
reply:

```sh
ottospace ctl '{"cmd":"state"}'
```

- Exit status 0: the request was accepted, and the reply is the full state.
- Exit status 1: the request was refused (`{"ok":false,"error":"..."}`), or no ottospace is running.
- Unknown commands and unknown fields are refused. Do not guess field names.

## Rules

- **Look before you act.** Start with `state`. Pane ids and tab indexes come from it.
- **`type` is keyboard input to a shell.** It can run anything the user can. Send only commands
  the user asked for, and end a command with `\r` to run it.
- **`open_app` and `enter` move the user's real windows.** Use them only when the user asked to
  bring an app into ottospace.
- **Do not send `quit`, `close` or `close_tab`** unless the user asked. Closing a terminal pane
  ends its shell; closing a tab closes every pane in it.
- **Read output by polling `state`.** There is no "wait until done" yet. After `type`, read
  `terminals.<pane>.lines` until the output you expect appears, with a time limit.

## State

| Field | Meaning |
|---|---|
| `mode` | `normal`, `app` (a hosted app has the keyboard) or `pick` (the picker is open) |
| `state.panes` | Every pane: `id` and `kind` (`terminal`, `empty`, or `app` with `title`) |
| `state.tabs`, `state.active` | The tabs and the index of the one showing |
| `tab_panes` | Pane ids of each tab, in tab order |
| `state.focused` | The pane that splits and keys act on |
| `state.live` | The app pane that has the keyboard, if any |
| `terminals.<id>` | `cols`, `rows`, and `lines`: the visible text of that terminal pane |
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

`type` goes to the focused pane unless `pane` is given. It fails if that pane is not a terminal.

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

`section` is `apps`, `workbook`, `tasks` or `repos`; the open section's name closes the bar.
`keys` shows or hides the key reference.

## Recipes

Run a command in a new terminal beside the current pane, and read its output:

```sh
ottospace ctl '{"cmd":"split","axis":"right"}'          # reply: state.focused is the new pane
ottospace ctl '{"cmd":"type","text":"cargo test\r"}'
ottospace ctl '{"cmd":"state"}'                          # read terminals.<pane>.lines; repeat until done
```

Open a second workspace for a different job:

```sh
ottospace ctl '{"cmd":"new_tab"}'
ottospace ctl '{"cmd":"split"}'
```
