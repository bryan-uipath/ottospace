# Workbook

Status: stage 1 built. The owner's note (`docs/ideas/1-initial-brainstorm.md`, 7.a.2):
"Workbook: Maestro DRI -> tab with dashboards, tab with confluence docs, etc", and later: "we
should be able to save a tab".

Keys, the section and the state are in `docs/reference.md` and `crates/ottospace/skill/guide.md`.

## What a workbook is

A saved, named workspace for a context that comes back: an on-call rotation, a weekly review.
It has no agent and is never done.

| | Is | Made by | Ends |
|---|---|---|---|
| Workbook | A named layout to open again: what to open, where | ⌘⇧S on a tab | When the user deletes it |
| Task | Work with a state, an agent, a brief | ⌘N, an agent, a session | Done, cleaned |
| Mark | A way back to one pane or tab that is open now; opened again if it is not | ⌘B, ⌘⇧B | When taken off |
| Session restore | Everything that was open at quit, back as it was | Nobody: automatic | Never (not built) |

A tab's mark and a workbook record the same thing (`marks::TabRecord`) and are opened by the
same code (`rebuild_tab`). They differ in what they are for:

- A mark has a letter and jumps. A workbook has a name and a row, and is updated by saving again.
- A mark keeps an app window's title, to find that window among several. A workbook keeps
  none: see Privacy.
- A mark's tab belongs to its task. A workbook's tab is a tab of no task.
- Marks are one file. Workbooks are one file each.

## Stage 1, as built

**Save.** ⌘⇧S, File ▸ Save Tab as Workbook, + in the section's header, or `Save this
tab` in the empty section. The tab on show is recorded:

- its split tree with ratios, and which pane had focus;
- a terminal: its directory now (as a mark reads it). No scrollback, no command;
- an app pane: the app's bundle id and name. No window title, URL or document;
- an empty pane: empty.

A tab with nothing in it is not saved; the status bar says so. A pane of an opened workbook
that still waits for its app's window is saved as that app's, not as empty.

A new workbook is named for the tab. The section opens on it with the name selected in a text
field: type over it, ↵ keeps it, Esc keeps the tab's name. Names are unique (`DRI`, `DRI 2`).

**Save again.** A tab that came from a workbook, or was saved as one, updates that workbook.
It does not ask: the section says `Updated · Undo` for six seconds, and Undo puts the old
layout back.

**Open.** Click the row, `Open`, or ↵ in list mode. A new tab of no task, named for the
workbook, with the recorded splits:

- a terminal starts in its directory. A directory that is gone: the terminal starts in the
  user's home folder, and the status bar names the directory. Directories are looked for off
  the UI thread and believed for two seconds: one removed inside that time still starts its
  terminal in the home folder, with no line;
- an app pane asks the shell for the app's window with `Effect::HostWindow`, as a tab's mark
  does. That is the one path that fills a given pane; the Apps section's `HostApp` always
  makes a new pane, so it cannot keep a layout. The hosting rules are the same: a running
  app's free window (the only one, else the main one), else the app is started; a window
  another pane holds is never taken, and the pane stays empty and says why. So does a pane
  whose app is not installed;
- if the workbook's tab is open already, it is selected.

**Rename, delete.** `Rename` and `Delete` on the row (hover or list mode), `r` and `x` in list
mode. Renaming renames the open tab too. `Workbook deleted · Undo` for six seconds, `u` in
list mode. The open tab stays.

**Files.** `~/.ottospace/workbooks/<id>.json` (under `OTTOSPACE_HOME`; nothing in a test run
that named no folder). Folder 0700, files 0600, written whole and renamed into place.

```json
{
  "version": 1,
  "id": "938f8d900241a077",
  "name": "Maestro DRI",
  "tabs": [
    { "title": "Tab 1", "focused": 2,
      "layout": { "split": { "axis": "down", "ratio": 0.5, "a": { "leaf": 1 }, "b": { "leaf": 2 } } },
      "panes": [ { "id": 1, "kind": { "type": "terminal" }, "cwd": "/Users/me/code/dashboards" },
                 { "id": 2, "kind": { "type": "app", "bundle": "com.google.Chrome", "name": "Google Chrome", "title": "" } } ] }
  ]
}
```

- `id` is 16 hex digits and is the file's name. Nothing else becomes a path: an id is checked
  in the core and again where the path is made. A file whose name is not its id, a link, a
  file over 256 KB, or one that does not read is counted in the status bar and never touched.
- `tabs` is a list so several tabs need no new format. This build saves and opens the first;
  others are kept as they are.
- Keys this build does not know are kept: the file's and the tab's always, a pane's until
  the tab is saved again (a saved pane is a new record).
- A `task` or a `command` in a file does nothing: the tab is of no task, and nothing is typed.
- The list is sorted by name: a folder has no order.
- Two ottospaces on one folder: each writes only the files it changed. One sees the other's
  new workbooks at its next start.

**Who.** Saving, renaming and deleting write the user's files, and opening starts their apps:
all four are `Origin::User` only (`Model::refusal`, `workbook::USERS_ALONE`), by key, click,
label or list mode. No `Request` was added. The state lists `workbooks`: name, pane kinds and
app names, whether it is open. Not the directories; only the status bar (the state's
`notice`) names one that is gone, when the user opens its workbook.

**Privacy.** A window title can hold other people's text (a chat, a document's name). A
workbook is long-lived and may be copied between machines, so it holds none.

**Code.** The core: `ottospace-core/src/workbook.rs`. The shell: `ottospace/src/workbook.rs`,
and `config.rs` for the folder. A workbook's name is typed in the mode a mark's name is
(`MarkInput::Workbook`, `mode` is `mark`).

## Later stages

Not built.

1. **Several tabs.** "Tab with dashboards, tab with confluence docs": Save adds the tab on
   show to a workbook; Open opens them all. The file has room.
2. **What the app showed.** URLs, the Slack channel, the document: session restore's layer 4
   hooks, per app (`session-restore.md`). A workbook then keeps what the hook returns, with
   the user's say per app.
3. **A workbook on the rail**, one click away.
4. **Agents proposing one**: "you open these four every Monday". A proposal the user accepts,
   never a file written for them.

## Open questions

1. Save again updates without asking, with Undo. Should it ask instead?
2. Should a workbook's tabs be a scope on the tab bar, as a task's are, so that opening one
   hides the other tabs?
3. An app already in a pane elsewhere: today the workbook's pane stays empty and says where
   the window is. Move the window instead?
4. Should a terminal be allowed a command to run (`k9s`, `tail -f`)? Marks took commands out:
   a client could attach one. Here only the user saves, so it may be safe if the row shows it.
5. Order: by name today. Pin or reorder by hand?
6. `mode` is `mark` while a workbook's name is typed. Worth a mode of its own?
