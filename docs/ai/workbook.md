# Workbook

Status: stages 1, 2a (several tabs) and 2b (terminal history and command) built. The owner's note (`docs/ideas/1-initial-brainstorm.md`, 7.a.2):
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
| Session restore | Everything that was open at quit, back as it was (`layout.json`: tabs and layout, built) | Nobody: automatic | Never |

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
- A `task` or a top-level `command` in a pane does nothing: the tab is of no task, and nothing
  is typed. Stage 2b's command lives in the pane's `history`.
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
  "Workbook", the section open on it with its name selected in the field, which has the
  keyboard: type a name and ↵ at once. Refused whole, with the reason in the status
  bar, rather than saved in part: more than 32 such tabs, or one that cannot be kept (split
  too deep).

A tab stands for one workbook's tab at most (`Workbook::live`, by place). Added to another
workbook, or saved with Save All, it leaves the first; that one keeps its record, closed.

**Titles.** A workbook of one tab opens it under the workbook's name, as stage 1 did, and
renaming the workbook renames that tab. Once a second tab is added, the first keeps the
workbook's name as its title, and each tab opens under its own title.

**Open.** Every tab of it that is not open is built, in order, each as a tab's mark is
(`rebuild_tab`), and the first is selected. A tab of it that is open is kept, not built again.
Each tab asks for its app windows as it is built; a window that fills a pane of a tab not on
show is moved off screen (`attach`, `display::parking_spot`), which is what a hosted window of
any background tab does today, and is placed when its tab is shown. So opening a workbook can start several apps
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
saved pane is still a clean record (keys a waiting pane kept from its file are dropped). A
save that would write a file larger than the reader takes (`MAX_FILE_BYTES`, 256 KB) is refused
with "the workbook would be too large to read back": what is saved always reads back.

**Who.** Add and Save All are `Origin::User` only, as Save is. Folding a row is not refused: it
writes nothing and opens nothing. The state lists each workbook's `tabs` (title, pane kinds,
summary, open tab) and `unfolded`; still no directory.

**Later: a scope on the tab bar.** Tasks are getting a tab-bar hierarchy, designed separately.
A workbook would use the same scope a task's tabs use (`Tab::task` today, `current_task` the
scope on show): opening a workbook would show its tabs alone, as selecting a task does, and
the scope's way back would leave them. `Workbook::live` already says which open tabs are whose,
so the scope needs no new record, only a second kind of owner beside a task.

## Stage 2b: a terminal's history and command, as built

The owner: "i'd expect the scrollback / history to be saved, but to have to have a command to
resume." What was on screen comes back as history; the command is offered, never run.

**Save.** For each terminal pane the shell reads, besides its directory:

- its last N lines of scrollback and screen as plain text (`workbook_scrollback_lines`, default
  500, at most 2000; 0 keeps no terminal text at all, the command line included);
- the command line of the program in front of its shell, if one runs. How: the PTY's
  foreground process group (`tcgetpgrp`); the core's `terminal::running` decides whether that
  is the shell at its prompt, a background job, or a program in front (the same rule as the
  quit question). Only for a program in front: the group leader's arguments by `sysctl
  KERN_PROCARGS2`, joined into one line a shell reads back the same (`terminal::command_line`
  quotes). Never parsed from the scrollback. The buffer is read by its layout alone
  (`process::parse_args`, pure and tested on built buffers): the count, the path, NULs to the
  next 8 bytes, then exactly that many NUL-ended arguments; the environment after them is never
  read. Anything off the layout (short buffer, count out of bounds, padding that is not NULs, no
  NUL, not UTF-8, an empty first argument as `exec -a ""` gives) is no command.

Reliable for the case that matters (`k9s`, `tail -f x`, `sleep 300`, `make dev`), with
limits: a program that set its own title (Node, npm via `process.title`) leaves its title in
`argv[0]` and the rest blank, so an `argv[0]` with a space is no command and trailing blanks go
(`terminal::saved_command`; `npm run dev` itself comes back as `node /path/npm run dev` or
not at all, by version); a program that rewrote its arguments otherwise shows them as they
are now; of a pipeline `a | b` only `a` (the group's leader); a job the shell
runs as a function or subshell shows the subshell (`sh`, `zsh`); another user's process (`sudo
x`) reads nothing, so no command is kept. The user sees the command before running it, which
is the safeguard for all of these.

Kept in the pane's record as `history`: `{"lines": [..], "command": "npm run dev", "saved":
<ms>}`. Version 1 files never had one: a `history` in a version 1 file is dropped as it is read.

**Cleaning and caps.** Every line has escape sequences (CSI, OSC, DCS, ...) and characters that
do not show (`capture::shows`: controls, bidi overrides, zero-width) taken out, a tab as a space;
a line is cut to 1000 characters; a terminal keeps at most 64 KB, oldest lines first to go; a
workbook keeps at most 1 MB of history in all, and terminals past that keep none (the file cap
went from 256 KB to 4 MB). A command is kept only if it is one line of characters that show, at
most 1024 characters, after trimming: one that is longer is dropped, not cut, since a cut
command is another command. All of this runs when the shell's text is saved, when a file is
read, and again when the history is used (`History::kept`, `history_of`): a file another
program edited is untrusted.

**Secrets, best effort.** `secrets.rs` in the core holds a small table of credential shapes:
`://user:pass@`; `key=value`, `--key=value` and `--key value` whose key holds token, secret,
password, passwd, pwd, apikey, api-key, api_key, auth, bearer, credential or private-key (any
case); `Bearer x`; `-p<password>` after `mysql`, `mysqldump`, `mysqladmin` or `mariadb`; an
`Authorization:` header (all after it); the prefixes `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_`,
`github_pat_`, `sk-`, `xoxa-`, `xoxb-`, `xoxp-`, `AKIA`, `eyJ` (JWTs); and 32 or more characters of
a token's alphabet with upper case, lower case and digits (a git hash has no upper case). In a
kept line each match becomes `[hidden]` and the line stays; a PEM block (`-----BEGIN` to
`-----END`, or the end of one whose start fell outside the kept lines) is left out whole. A
command with any match is not kept at all: a command with a part hidden would not run as it
did. Hiding again changes nothing, and a test of random lines checks that what is kept holds
no shape of the table. This is best effort: a secret of no known shape is kept as it was
printed. `workbook_scrollback_lines: 0` keeps no terminal text, and reads none.

**In plain sight.** The save says what was kept in the status bar (`Saved "DRI": 2 tabs, 500
lines of history, 1 command`), and a workbook's row ends in `· history` while it holds any.
Deleting a workbook removes its file, history and all (the e2e test checks the folder is
empty, not only the list).

**Open.** The core builds the tab as before, and for each terminal that started emits
`Effect::History { pane, history }` after its `Opened`. The shell, starting that terminal, hands
the history to `Terminal::spawn`, which writes `workbook::history_view` into the terminal's
view through its own parser before the PTY's event loop starts: the lines in grey, then
`── saved 2026-11-13 15:02 ──`, then the shell's own prompt below. Written to the view,
never to the shell: nothing of it is echoed or run. Its only escape codes are its own grey and
reset; the text was cleaned again first.

A command is typed on the prompt as keys, with no ↵: once the shell has printed, been quiet
for 300 ms, and runs nothing in front of it (`terminal::running` is `None`), so a startup
program that asks a question is not typed into. There is no deadline: no prompt, no typing.
The user presses ↵ to run it, or edits or clears it. If the user (or a client's `type`) types
into the pane first, it is not typed at all. Limit: a question the shell's own startup script
asks with a builtin (`read -q`) looks like a prompt, and the command could be typed into it.
The rule "printed, quiet, nothing in front" stays; the real fix is prompt integration (OSC 133:
the shell marks where its prompt starts and ends), later. Saving the tab again before the user touched the pane keeps the
command (the program is not running yet, but it was offered). The divider says it too, `── saved <date> · on the prompt, ↵ runs: npm run dev ──`, so a
command from a file another program changed is in plain sight before ↵. This is Zellij's "Press
ENTER to run" without the daemon.

The date is local time. The file keeps milliseconds since 1970; the shell reads the time
zone's offset at that moment (`ottospace_mac::time::utc_offset`, `localtime_r`) and passes it
to `history_view`, which formats it: the core stays pure.

**Trust and privacy.** Scrollback and command lines can hold secrets (a token a tool printed,
`export FOO=...` on a command line).

- Files stay 0600 in a 0700 folder, as stage 1 wrote them.
- The state never has the text: a row's terminal pane lists `lines` (a count) and
  `has_command`. A property-test `check` sends histories marked `SAVEDHISTORY` and fails if
  that marker reaches the state or the rows. `Effect`s are not in any reply.
- Only the user saves or opens (stage 1's rule), so no socket client can make the app read a
  terminal's scrollback into a file, or type a command from one. No request reads a workbook.
- The pre-typed command comes only from a workbook's own file: never from a mark (`retry_unfilled`
  is shared with marks, so the effect is emitted by `workbook_open`, not by the rebuild), and not
  from a top-level `command` key.
- Text restored from a workbook is ordinary terminal text once shown: `capture` and the
  state's `terminals[..].lines` show it, as they show anything a shell printed, so a client that
  can read a terminal can read it. It was cleaned of credential-shaped text when saved, best
  effort. "Never in the state" is the rule for the workbook files and rows, which list counts
  only. Why that is enough: the socket is same-user, and a client can already read any
  terminal. Saving the tab again saves the lines again (the divider too): what is on screen.
- No per-pane "do not keep"; the setting is global.

**App panes** are as before: the app's identity only.

**A separate track: keeping the process alive.** What 2b does not do is keep `npm run dev`
running when ottospace quits. That needs the PTY and its shell to outlive the app: a small
daemon that owns the PTYs (as SupaCode/zmx do, or tmux), with ottospace attaching to it at
start and the workbook naming the daemon's session instead of a command. It changes what a
pane is (attached, not owned), what quitting means (the quit question no longer ends work), and
the trust model (a long-lived process holding the user's shells, reachable over its own socket).
Not built; designed in `terminal-persistence.md`. It would sit beside session restore, and a
workbook could then offer "attach" where 2b offers "↵ runs".

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
4. Should a terminal be allowed a command to run (`k9s`, `tail -f`)? Stage 2b: it is typed on
   the prompt and shown in the divider, never run. Should the row show it too (the state
   would still count it only)?
5. Order: by name today. Pin or reorder by hand?
6. `mode` is `mark` while a workbook's name is typed. Worth a mode of its own?
7. Add Tab to Workbook… from the menu goes through list mode. A small picker would be clearer.
8. A tab added to a second workbook leaves the first. Should a tab be able to belong to two?
9. A per-pane "do not keep this terminal's history", for a pane that shows secrets?
10. A restored history saved again keeps the old divider as a line. Drop lines above a divider?
11. History opt-in per workbook, and a "Save without history" action? Recommendation: add
    "Save Tab without History" beside Save (one menu item, the same save with no text read)
    before any per-workbook setting. It answers the case that matters, a tab that shows
    secrets today, at the moment of saving, where the global setting is a decision made once
    and forgotten; a per-workbook flag would then follow only if users ask to keep a workbook
    textless across saves.
