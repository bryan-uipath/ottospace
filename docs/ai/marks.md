# Marks

Status: built. The owner's ask: "bookmark certain panes / tabs (ie, a notes tab, a slack tab) -
maybe a 'marks' in the left pane".

A mark is a named, saved way back to a pane or a whole tab. Keys, requests and the state are in
`docs/reference.md` and `crates/ottospace/skill/guide.md`. This note says what a mark is made of,
what was decided, and how it sits beside other features.

## What a mark records

`~/.ottospace/marks.json` (under `OTTOSPACE_HOME`), written whole and renamed into place, mode
0600. Never in a test run that named no folder.

```json
{
  "version": 1,
  "marks": [
    { "id": "9f3a61c2d4e5b708", "name": "notes", "key": "n",
      "target": { "pane": { "id": 3, "kind": { "type": "terminal" }, "cwd": "/Users/me/notes" } } },
    { "id": "1b7c0e9a55d2f346", "name": "Slack",
      "target": { "pane": { "id": 4, "kind": { "type": "app", "bundle": "com.tinyspeck.slackmacgap",
                                               "name": "Slack", "title": "general - Slack" } } } },
    { "id": "name:review", "name": "review", "key": "r",
      "target": { "tab": { "title": "review", "task": 12, "focused": 1,
        "layout": { "split": { "axis": "right", "ratio": 0.6, "a": { "leaf": 1 }, "b": { "leaf": 2 } } },
        "panes": [ { "id": 1, "kind": { "type": "terminal" }, "cwd": "/Users/me/code/app" },
                   { "id": 2, "kind": { "type": "app", "name": "Safari", "title": "PR 61" } } ] } } }
  ]
}
```

- Never a window id or a pid: they do not survive a restart. The pane or tab a mark stood for
  last is kept in memory only, as a hint for "is it open now".
- A mark's `id` is its name for good: 16 hex digits made up when it is marked. A hand-written
  mark with none gets `name:<its name>`, the same at every read.
- `id` in a pane record is the pane's id when it was recorded. Only a tab's `layout` reads it; a
  pane opened from the record gets a new id.
- The record is taken when the mark is made. It does not follow the pane or tab afterwards. One
  exception: an app's `title` is read again each time the open mark is gone to. It is the title
  the window has then, not the one it had when it was hosted.
- **No commands.** A mark types nothing into a shell. A `command` key in the file is ignored,
  said once in the status bar ("marks.json: commands are not supported; ignored"), and left out
  of the next write.
- Keys this build does not know (of the file, a mark, a tab, a pane, an app's `kind`) are kept
  and written back.

## What is read from the file

`marks.json` is input, not trusted text.

- A file that is not JSON, or has another `version`, is left as it is. Nothing is written that
  run, and the status bar says so.
- A record is unsound when its directory is not an absolute path, when a directory, title,
  bundle id or name is too long or has a control character, when an app has no name, or when a
  tab's layout names a pane it does not list exactly once. That mark is left out and named in
  the status bar; the others load. The file is then left as it is for that run: writing would
  drop the mark.
- The notice about `marks.json` has its own place: another notice from the start (a wrong
  setting) does not hide it.
- An app is started by the path the system gives: Launch Services by its bundle id, else
  `<name>.app` in the folders apps are installed in. A path in the file is never run.

## Two ottospaces on one folder

Each writes the whole list. Before a write, the file is read again. If it changed since this
ottospace last read or wrote it, the two lists are merged by mark `id` (`marks::merge`): what
either added is there, what either removed is gone, a mark this one changed is this one's, and
the order is the file's unless this one moved a mark. A file that no longer reads is left alone
from then on, with a notice. The other's changes are seen at this one's next write, not before.

## Open now, and going to a mark

| Kind | Open now when | Going to it when it is not |
|---|---|---|
| Terminal | The pane it stood for last is still there. Its directory does not matter: the mark is of the pane | A terminal in the recorded directory: it fills the focused pane if that is empty, else splits it, as a Repos row does |
| App | That pane is still a window of the same app; else any pane holds a window of that app with the recorded title | The shell hosts the app's window: among windows no pane holds, the one with the title, else the only one, else the main one. An app that is not running is started with `open -g`, and the window is picked once the app's windows have stopped appearing for 1.5 seconds (30 at most). No pane exists until the window is in hand |
| Tab | The tab it stood for last is still open | A new tab with the recorded splits and ratios, built with what can be opened |

Open now: the tab comes forward and the pane takes focus. An app pane is focused, not entered.

A pane mark is unavailable, with the reason, when it is not open and its directory is gone or
its app is neither installed nor running. A tab is unavailable only when none of its panes can
open. Else it is built: a pane whose directory is gone, whose app is missing or whose window
could not be hosted is an empty pane that says why, in the pane and in the mark's row. Going to
the open tab again tries those panes again.

Whether a directory or an app is there is looked up off the UI thread, and believed for two
seconds: a directory on a mount that hangs does not freeze the window.

## Taking a mark off

⌘B on a marked pane, × on a row, or `x` in list mode. The Marks section then offers "Mark
removed · Undo" for six seconds, the same line the Tasks section uses. `u` in list mode puts
the last removed mark back while the Marks section is open (in other sections `u` is a task's
undo). ⌘B again on the same pane or tab within the six seconds puts the same mark back, with
its name and letter.

## Decisions

- **The core owns** the mark type, matching, what going does (as effects), order, one letter a
  mark, toggle and undo, what is sound in the file, the merge, and which window a record gets
  (`pick_window`). The shell reads directories and the app's windows, and hosts.
- **A terminal that changed directory** keeps its mark, and the mark keeps the directory it
  recorded. Two shells in one directory cannot be told apart, so a terminal is matched by its
  pane only.
- **Names are unique** (`notes`, `notes 2`): the socket finds a mark by its name.
- **A window in a pane is never taken for another pane.** With no free window, going to the mark
  goes to the pane that holds the titled one.
- **A move that would act behind the picker, the new-task box, the key reference or visual mode
  is refused with the reason**, not answered `ok`.
- **Not built:** drag to reorder (list mode's ⇧J / ⇧K do it), chips in the top bar (the tabs take
  that width).

## Later

- **Commands.** A first version let a mark carry a command typed into the terminal it opened. It
  was taken out: a socket client could attach one to a mark the user made, nothing showed it,
  and the user's own key typed it later. If commands return, a command is shown in the mark's
  row, checked (no control characters, a length cap, never on a tab's panes), and one that the
  user's own input did not set asks before its first run.
- A second ottospace sees the first one's marks only when it next writes. Reading the file
  again when the Marks section opens would close that gap.

## Beside other features

**Session restore** (`session-restore.md`). `PaneRecord` and `TabRecord` are the entries of layer
1's `session.json`: `kind` with `type`, `bundle`, `name`, `title`; `cwd`; a tab's `title`, `task`,
`focused`, `layout` (leaves name pane ids) and `panes`. `Node` now deserializes, with its
`ratio`. `Kind::App` now holds `bundle` and `name`, which that note lists as missing. Session
restore adds `key`, `pid` and `window` to the pane record (unknown keys are kept already), and
can use `rebuild_tab`, `Action::Fill`, `Effect::HostWindow`, the empty pane that says why, the
wait for a started app's windows to settle, and the file's checks as they are.

**The planned `^reference` tag in the new-task box.** A mark has a unique name, so `^notes`
could name one: the task then opens with that pane or tab beside its own. `Marks::find` by name
is the lookup; the tag is not built.

**Pinned apps in the Apps section.** A pin is an app: it stays in the list whether it runs or
not, and choosing it hosts the app's main window. A mark is one window (by title) or one pane,
with a name and a letter of its own. Two Slack windows are one pin and two marks.

**Tasks own tabs.** A tab's mark records its task. Going to it shows that task's tabs, as
selecting the tab does. Built again, the tab is the task's if the task still exists, else a tab
of no task.
