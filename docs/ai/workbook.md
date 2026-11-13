# Workbook

Status: stages 1 and 2a (several tabs) built. The owner's note (`docs/ideas/1-initial-brainstorm.md`, 7.a.2):
"Workbook: Maestro DRI -> tab with dashboards, tab with confluence docs, etc", and later: "we
should be able to save a tab".

Keys, the section and the state are in `docs/reference.md` and `crates/ottospace/skill/guide.md`.

## What a workbook is

A saved, named workspace for a context that comes back: an on-call rotation, a weekly review.
It has no agent and is never done.

| | Is | Made by | Ends |
|---|---|---|---|
| Workbook | Named tabs to open again: what to open, where | ⌘⇧S on a tab, Add tab, Save All Tabs | When the user deletes it |
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
- `tabs` is a list so several tabs need no new format. Stage 1 saved and opened the first.
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

## Stage 2a: several tabs, as built

The owner: "a workbook could have multiple tabs". A workbook is a list of tabs.

**Save.** ⌘⇧S keeps its meaning on a tab of no workbook: a new workbook of that one tab. On a
tab opened from a workbook, or saved or added into one, it updates that tab's place in it
(`Updated · Undo`). Two ways to grow one:

- **Add tab**: the word on a workbook's row, `a` on it in list mode, or File ▸ Add Tab to
  Workbook… (a menu item cannot ask which, so it opens the section in list mode and the status
  bar says `pick a workbook: A adds this tab to it`; with no workbook yet it saves a new one).
  The tab on show goes at the end. Its tab is that one's already: it is updated in place.
- **Save All Tabs as Workbook** (File menu): every tab on the bar (of the current task's scope,
  as the bar shows it) that has something in it, in bar order, as one new workbook named
  "Workbook", its name selected to type over.

A tab stands for one workbook's tab at most (`Workbook::live`, by place). Added to another
workbook, or saved with Save All, it leaves the first; that one keeps its record, closed.

**Titles.** A workbook of one tab opens it under the workbook's name, as stage 1 did, and
renaming the workbook renames that tab. Once a second tab is added, the first keeps the
workbook's name as its title, and each tab opens under its own title.

**Open.** Every tab of it that is not open is built, in order, each as a tab's mark is
(`rebuild_tab`), and the first is selected. A tab of it that is open is kept, not built again.
Each tab asks for its app windows as it is built; the shell parks the windows of tabs not on
show, as for any pane filled in a background tab. So opening a workbook can start several apps
at once: the user asked for them.

**The row.** `3 tabs · 4 terminals · Chrome, Slack`; one tab: `2 terminals · Chrome, Slack`.
▸ before a workbook of several tabs (space in list mode) lists its tabs under it, folded by
default and not kept between runs. A tab's row opens it alone (click, ↵) and has `Rename` (its
title, `r`) and `Remove` (`x`: out of the workbook, `Updated · Undo`; its last tab deletes the
workbook). The row's words are now `Open`, `Add tab`, `Rename`, `Delete`.

**The tab bar.** A tab of a workbook shows the workbook icon; hovering the tab names the
workbook. Its tabs are ordinary tabs otherwise: no scope hides the others.

**File.** `version` 2. A version 1 file reads as a workbook of its tabs (stage 1 wrote one) and
is written as version 2 at its next change, not at start. Each tab must be sound now, not only
the first. At most 32 tabs (`MAX_TABS`); the depth, pane and size caps of stage 1 stay, and a
saved pane is still a clean record (keys a waiting pane kept from its file are dropped).

**Who.** Add and Save All are `Origin::User` only, as Save is. Folding a row is not refused: it
writes nothing and opens nothing. The state lists each workbook's `tabs` (title, pane kinds,
summary, open tab) and `unfolded`; still no directory.

**Later: a scope on the tab bar.** Tasks are getting a tab-bar hierarchy, designed separately.
A workbook would use the same scope a task's tabs use (`Tab::task` today, `current_task` the
scope on show): opening a workbook would show its tabs alone, as selecting a task does, and
the scope's way back would leave them. `Workbook::live` already says which open tabs are whose,
so the scope needs no new record, only a second kind of owner beside a task.

## Later stages

Not built.

1. **What the app showed.** URLs, the Slack channel, the document: session restore's layer 4
   hooks, per app (`session-restore.md`). A workbook then keeps what the hook returns, with
   the user's say per app.
2. **A workbook on the rail**, one click away.
3. **Agents proposing one**: "you open these four every Monday". A proposal the user accepts,
   never a file written for them.

## Open questions

1. Save again updates without asking, with Undo. Should it ask instead?
2. Should a workbook's tabs be a scope on the tab bar, as a task's are, so that opening one
   hides the other tabs? (Stage 2a: no; see "Later: a scope on the tab bar".)
3. An app already in a pane elsewhere: today the workbook's pane stays empty and says where
   the window is. Move the window instead?
4. Should a terminal be allowed a command to run (`k9s`, `tail -f`)? Marks took commands out:
   a client could attach one. Here only the user saves, so it may be safe if the row shows it.
5. Order: by name today. Pin or reorder by hand?
6. `mode` is `mark` while a workbook's name is typed. Worth a mode of its own?
7. Save All names the workbook "Workbook". Name it for the task whose tabs are on show instead?
8. Add Tab to Workbook… from the menu goes through list mode. A small picker would be clearer.
9. A tab added to a second workbook leaves the first. Should a tab be able to belong to two?
