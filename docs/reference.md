# ottospace reference

Everything beyond the [cheat sheet](cheat-sheet.md). Front page: [README](../README.md).
Install and update: [install.md](install.md). Working on ottospace: [development.md](development.md).

ottospace is a macOS workspace that tiles terminals and real app windows in one window. It is for
people who run several coding agents and want them, and the apps around them, in one place.
Agents and tests can drive it over a local control socket. Rust + [GPUI](https://gpui.rs).

## Status

Early prototype, macOS only. Today you can split a window into panes, run your shell in a pane,
host another app's window in a pane, move focus by keyboard, keep a tasks inbox, and drive all of
it over the control socket. Session restore and the rest of the design are not built: see the
[roadmap](#roadmap-designed-not-built) and the [plan](ai/1-initial-brainstorm/plan.md).

### Works today

| Capability | State |
|---|---|
| Split panes mixing terminals and real app windows | Works today |
| Key bindings and a status bar showing `NORMAL` / `APP` mode | Works today |
| Control socket and `ottospace ctl` for agents and tests | Works today |
| Terminal scrollback, mouse selection, copy and paste | Works today |
| Visual mode: select terminal text with vi keys | Works today |
| Tasks inbox: what needs you first, then the other tasks with tabs open (Open), then Running, Captured, On hold, Done. Headings fold (click, or ↵ or space in list mode). A task that is done stays under Done with its tabs open. Each row shows its tags, what it has open and its age; hover it for start, snooze, hold and dismiss buttons. After one of those by mouse, a line at the top offers Undo for 6 seconds. Kept in `~/.ottospace/tasks` | Works today |
| "Pop next" (⌘⇧N): open the next task that needs you | Works today |
| Sessions in the Tasks section: the Claude Code and Codex sessions of the listed repositories that are no task's (one you started by hand in a terminal). Each row shows the runner, the repository, active or idle, what it does now, and when it was last active. Adopt makes a task of one; Hide removes it until the next start. Read-only: nothing is sent to a session | Works today |
| New task box (⌘N): one line, with [tags](#tags): `#project`, `>repo`, `@agent`, `/path`, `$skill`, `p1`–`p9`. `@agent`, `/path` and `$skill` are stored and shown; nothing acts on them yet | Works today |
| Home screen: a tab with no panes shows the new-task line, the tasks that need you, and each listed task as a node on a lattice (left to right by project; within a project the older task is farther). A click on a node or a row opens the task | Works today |
| Tasks own their tabs: opening a task shows only its tabs; the others keep running off the bar | Works today |
| Tabs, each with its own split tree | Works today |
| Pane picker: choose a terminal or a running app for a new pane or tab | Works today |
| Icon rail with a secondary bar; Apps lists running apps with their icons | Works today |
| Repos: repositories and their worktrees; open a terminal in one | Works today |
| List mode (⌘⇧E): drive the bar with the keyboard | Works today |
| Sneak mode (⌘⌃G): a letter label on each of ottospace's own controls; typing it clicks the control | Works today |
| Marks (⌘B, ⌘⇧B, ⌘' and a letter): named ways back to a pane or a tab, kept in `~/.ottospace/marks.json` | Works today |
| `settings.json`: terminal font, repository directories, tasks folder, apps to offer, window rules | Works today |

### Roadmap: designed, not built

None of these is in the app today.

| Capability | State |
|---|---|
| Other key modes: insert; sneak labels inside a hosted app's window | Designed |
| Tasks: a worktree at kickoff, a brief pane, automatic cleanup when done | Designed |
| Quick capture (⌘⌘): turn something on screen into a task and dispatch it to an agent | Designed |
| Focus blocks: hide Slack, with an agent that can break through | Designed |
| Fast app switcher; Workbook section | Designed |
| TypeScript scripting, with contextual quick actions in an action deck (Zoom meeting layout, spin for stand-up) | Designed |
| Hide private panes while screen sharing | Designed |
| Session restore | Designed |

## Requirements and permissions

- macOS 14 or later.
- Accessibility permission for the terminal that launches ottospace: System Settings > Privacy &
  Security > Accessibility. The app exits at start without it.
- Screen Recording permission for the same terminal: System Settings > Privacy & Security > Screen
  & System Audio Recording. macOS asks for it at launch, when ottospace lists running apps.

An installed `ottospace` command uses the permissions of the terminal you run it from. The app
bundle asks for them itself: see [install.md](install.md).

## Keys

Short version: [cheat-sheet.md](cheat-sheet.md).

| Key | Action |
|---|---|
| ⌘D | Split right, new terminal |
| ⌘⇧D | Split down, new terminal |
| ⌘E | Pick what goes in a new pane: a terminal or a running app |
| ⌘W | Close the focused pane |
| ⌘⌥←/→/↑/↓ or ⌘⌥H/L/K/J | Focus the pane in that direction |
| ⌘] / ⌘[ | Focus the next / previous pane |
| ⌘⌃←/→/↑/↓ or ⌘⌃H/L/K/J | Resize: move the focused pane's split that way |
| ⌘⌃= | Even out the splits of the tab |
| ⌘⌃0 | Fit the focused pane to its app's window, when the app refused the pane's size |
| ⌘⇧⌥←/→/↑/↓ or ⌘⇧⌥H/L/K/J | Swap the focused pane with its neighbour on that side |
| ⌘⇧⌥1 … ⌘⇧⌥9 | Move the focused pane to that tab on the bar. US keyboard layout only |
| Drag a pane's header | Onto another pane's edge: split there. Onto its centre: swap. Onto a tab: move there. Esc cancels. An app you are in is parked: click it to go back in |
| ⌘⌃F | Zoom the window to fill the screen; again to restore |
| ⌘⌃A | Debug: outline the focused app pane's accessibility targets (parked panes only) |
| ⌘/ | Show every key |
| F18 | From any app: take the keyboard back and bring ottospace forward |
| ⌘T | New tab: pick what opens in it |
| ⌘1 … ⌘9 | Select a tab on the bar |
| ⌘⇧] / ⌘⇧[ or ⌃Tab / ⌃⇧Tab | Next / previous tab |
| ⌘⇧W | Close the tab and its panes |
| ⌘Q | Quit. While a terminal runs something (a program in front of its shell, or a background or stopped job) it asks first, and lists them: ↵, Q or ⌘Q again quits; Esc, any other key or any other shortcut cancels and does nothing else |
| Wheel / trackpad | Scroll a terminal back through its history |
| ⇧PageUp / ⇧PageDown | Scroll a terminal a page |
| ⇧Home / ⇧End | Top / bottom of a terminal's history |
| Drag / double-click / triple-click | Select text / a word / a line in a terminal |
| ⌘C | Copy the terminal's selection (nothing selected: does nothing) |
| ⌘V | Paste into the terminal |
| ⌘N | New task: a box opens over the workspace. Type what needs doing, with [tags](#tags): `#project`, `>repo`, `@agent`, `/path`, `$skill`, `p1` to `p9` (Tab completes the one being typed). ↵ adds the task and closes the box; ⌘↵ adds it and starts it (shows its tabs); Esc closes |
| ⌘⇧N | Next task: shows the tabs of the first task that needs you |
| ⌘B / ⌘⇧B | Mark the focused pane / the tab on show; on one that is marked, take the mark off (the Marks section offers it back for six seconds, and ⌘B again within them puts the same mark back). The mark is named for the app, the terminal's directory or the tab's title. It records what to open, not a window or a process: a terminal's directory as it is then, an app's bundle id, name and window title, a tab's title, task and split tree. A marked pane or tab shows ◆ and its letter |
| ⌘' then a letter | Go to the mark with that letter. The status bar shows `MARK?` while it waits; Esc, or a letter of no mark, ends the wait and nothing else happens. If the mark's pane or tab is open, its tab comes forward and the pane takes focus (an app pane is focused, not entered). Else it is opened: a terminal in the recorded directory, the app's window in a pane (the one with the recorded title, else its only window, else its main one; an app that is not running is started), or the tab with its splits. A tab opens with what can be opened: a pane whose directory is gone or whose window could not be hosted is empty and says why, and going to the tab again tries it again |
| ⌘⌥1 … ⌘⌥9 | Go to one of the first nine marks, in the order of the Marks section |
| ⌘⇧E | List mode: the bar (Apps, Tasks, Repos, Marks) takes the keyboard. On a mark `x` removes it, `r` renames it, `m` gives it a letter, ⇧J / ⇧K move it down / up, and `u` puts the last removed mark back. `j`/`k` or arrows move; ↵ opens the row (⇧↵ splits down); in Tasks a heading is a row too, and ↵ or space on it folds or unfolds it; Tab changes section; on a task `s` snoozes one hour, `h` holds, `x` dismisses, `u` undoes; Esc or ⌘⇧E leaves |
| ⌘⌃G | Sneak mode: every control ottospace draws (rail icons, bar rows, tabs and their ×, top-bar buttons, pane headers and their ×, each pane, the home screen's line, rows and nodes) gets a short label, home-row letters first. Type a label and the control does what a click does; a pane's label focuses it, or gives an app pane the keyboard. A × (pane or tab) always has a two-letter label. Two-letter labels narrow on the first letter; ⌫ takes it back. Esc, a click, a letter that goes on with no label, or any ⌘ shortcut leaves; the shortcut then runs (⌘C copies, ⌘V pastes). Other keys do nothing. The labels follow what is drawn: if a control moves its label moves, and if it goes its label goes. What happens by itself (a shell exits, an agent files a task) does not end the mode. Starting it hides the accessibility overlay (⌘⌃A). Not ⌃G alone: a terminal needs that key. On a keyboard layout with no Latin letters, the keys type the letters they have under ⌘. From `APP` mode return first. Controls inside a hosted app's window get no label. Sneak presses no control inside another app and sets no value; entering an app pane raises its window, as a click on the pane does |
| ⌘⇧C | Visual mode on the terminal: `hjkl`/arrows, `w b e 0 ^ $`, `H M L` move; `v` / `V` select; `y` or ↵ copies and leaves; Esc leaves |

**The quit question.** ⌘Q, Quit in the menu and the window's red close button all ask the same
question (status chip `QUIT?`). A quit ends every terminal and what runs in it, so it asks when:

| In a terminal | Asks? |
|---|---|
| A shell at its prompt, no jobs | No |
| A program in front of the shell: `cargo`, `vim`, `ssh`, an agent waiting at its own prompt | Yes |
| A background or stopped job of the shell (`sleep 100 &`, a ⌃Z'd `vim`) | Yes: listed as `sleep (background)` |
| A job that is no longer the shell's child (it forked away, as a daemon does) | No |
| `tmux`, `screen` or a nested shell in the pane | Yes, always |
| A program the shell `exec`'d: it took the shell's place | No |
| A shell that exited, pane not closed yet | No |

- While the question is up, ↵, Q or ⌘Q quits. Every other key or shortcut cancels it and does
  nothing else: ⌘W closes nothing, ⌘V pastes nothing. A held key answers nothing. A click outside
  the box cancels.
- A terminal that closes by itself leaves the list. When the list empties the question goes away;
  ottospace does not quit.
- Not asked: Quit in the Dock menu, and logout, restart or shutdown. macOS ends the app directly
  there, and GPUI 0.2.2 gives no hook to answer first. The socket's `quit` never asks.
- A shell whose prompt keeps a helper process as its child is asked about every time.

Other keys go to the focused terminal. ⌃ sends control codes, ⌥ is Meta.

## Tags

One line in the new-task box (⌘N), or in `task_new`, becomes a task. Words with a tag mark are
read out of the line; the rest is the title.

| Tag | Means | Tab completes from |
|---|---|---|
| `#project` | The project the task serves | Projects other tasks name |
| `>repo` | The repository the task works in | The Repos list (repositories and worktrees) |
| `@agent` | Who works it | `claude` and `codex`, then each `~/.ottospace/agents/<name>.md` if that folder exists |
| `/path` | A path the task is about: a word that starts with `/` or `~/` | The files and directories under what is typed |
| `$skill` | A skill to load for the task | Skill folders (those with a `SKILL.md`) in `~/.claude/skills`, `$CLAUDE_CONFIG_DIR/skills`, `~/.codex/skills` (or `$CODEX_HOME/skills` if that is set), and the `.claude/skills` of the task's `>repo` |
| `p1` to `p9` | The priority. 1 is most urgent; 3 when none is given | |

```
fix zoom flicker #canvas >canvas-app @claude p2
review the export PR >canvas-app @codex $code-review /src/export
```

- **A tag counts only if its name is known.** A `>word`, `@word` or `$word` that names nothing
  stays in the title where it was typed, and so does a `/word` that is not a file or directory.
  `ask @ana`, `costs $5` and `use > 3 workers` are safe. A name starts with a letter, so `#123`,
  `>3` and `$5` are never tags. `#project` always counts: a new name is a new project. A name
  longer than 200 characters is no tag either: it stays in the title.
- **A `/path`** is under the home folder (`~/notes.md`), or, when the line names a `>repo`,
  inside that repository: `/src/export` with `>canvas-app` is `src/export` in it. If nothing is
  there (or no repository is named), it is absolute: `/Users/me/notes.md`. So with a `>repo` the
  repository is looked in first, then the root of the disk; Tab lists the repository's entries
  before the root's. It counts if a file or directory is there. The task keeps the word as typed
  (`paths`) and, beside it, the absolute path that was found (`resolved`).
- **Punctuation and spaces.** A comma or full stop after a tag is not part of it: `#canvas,`
  and `p1,` work. For a path the word as typed is tried first, so a file named `v1.` is found,
  then the word without the punctuation. A space in a path is written with a backslash before
  it, `~/My\ Notes`; Tab writes it for you. The task keeps the path without the backslash.
- **How many.** One `#project`, one `>repo`, one `@agent`, one priority: of two, the last one
  counts. Every `/path` and `$skill` is kept, each once (at most 20 of each). `@agent` and
  `$skill` are kept as their file or folder spells them, whatever the case typed.
- **What they do today.** `#project` and `>repo` are used: the home screen groups by project, and
  a task opens a terminal in its repository. `@agent`, `/path` and `$skill` are stored on the
  task and shown on its row. Nothing starts an agent or loads a skill yet. What must be
  checked before anything does: [routines.md](ai/routines.md#6-trust-and-safety).
- **Reading is off the window's thread.** Names and paths are looked up on another thread, so a
  slow disk does not stop typing. A tag's chip shows a moment after the word is typed; ↵ files
  the line with the tags that have shown.
- `OTTOSPACE_SKILLS` (directories, `:` between them) replaces the runners' skill folders.
- `@name` named a repository before. It does not now. Titles of tasks already filed are not read
  again.

## Sessions

An agent is a saved definition. A session is one running conversation of Claude Code or Codex.
The Tasks section lists, under **Sessions**, the sessions of your listed repositories (worktrees
included) that no task has adopted: for example a Claude Code you started by hand in a terminal.

- **What a row shows.** The runner (`claude` or `codex`), the repository or worktree, a dot that
  is lit while the session is active (its file changed in the last 30 seconds), what it does now
  (`shell · cargo test`, `network · git push`, `files · src/main.rs`), and when it was last
  active. Idle sessions older than a day are behind "N older".
- **Order.** Sessions at work come first, in the order they started, so a row does not move
  while you reach for it. Then the idle ones, the most recently active first.
- **Adopt** makes a task of the session: named after the repository, with `>repo` set to that
  repository (wherever in it the session started) and `@agent` to its runner. The session is
  that task's from then on and leaves the group. Rename the task later. The task keeps the
  session's runner and id, and no path from the session's file.
- **An adopted task follows its session.** While the session works, the task is Running. When
  the session has been quiet for 3 minutes (`session_quiet_minutes`), the task moves to Needs
  you, once. When it works again, Running again. If you or an agent set the task's state (hold
  it, mark it done), that state stays. Quiet is longer than idle on purpose: a row reads idle
  after 30 seconds, but a build that prints nothing for a minute is not a question for you.
- **With the Tasks section closed** adopted tasks follow their sessions all the same, so the
  status bar's count of tasks that need you is right. Every 20 seconds ottospace looks at when
  the files of those sessions last changed (one `stat` each): no directory is listed and no
  file is opened. The one exception: after a start, where the files of sessions adopted in an
  earlier run are is not known, so once for all of them the session folders are listed and
  each session file's first records are read to see which directory it started in. No
  transcript is read further. A session that this does not find is not looked for again until
  you open the Tasks section. With no adopted task, or with `"sessions": false`, nothing is
  read.
- **Hide** removes the row until ottospace starts again.
- **A click** focuses the session's pane when the row says "in pane N", and selects the row
  otherwise. "In pane N" shows only when there is no doubt: the session is at work, exactly one
  terminal on the tab on show was opened in its directory, and no other listed session at work
  started there.
- **Fresh or not shown.** The list is emptied when the Tasks section closes and read again when
  it opens, so nothing shows what a session did a while ago. If the reader stops, the status
  bar says so once.
- **Read-only.** ottospace reads the runners' own session files, every 5 seconds while the Tasks
  section is open and never while it is closed. It does not attach to a session, type into it or
  signal it. Only derived facts are shown: names from fixed tables, counts and times. Never a
  prompt, a file's contents or a command line. Design: [sessions.md](ai/sessions.md).
- Turn it off with `"sessions": false`. See [Settings](#settings) for where sessions are read from.

## Terminal panes

A terminal pane runs your shell on a PTY. The pane closes when the shell exits. It keeps 10,000
lines of history. Typing returns to the bottom. In programs that use the mouse or the alternate
screen (vim, less, htop) the wheel does nothing, and ⇧PageUp and friends go to the program.
A click on a terminal focuses it and clears its selection; a drag there selects, even in an
unfocused pane. Paste is bracketed when the program asks for it.

## App panes

To put an app window in a pane, open Apps on the left rail and click the app. Apps you name in the settings are
listed there while they are not running: a click starts one (ottospace waits up to 30 seconds for its window). It fills the focused pane if that is
empty, otherwise it splits right. Shift-click splits down. New windows of a hosted app get their
own pane. Original window frames are saved to disk and restored on quit or after a crash.

Click an app pane to give it the keyboard (`APP` mode). If you bring a hosted app forward from
outside (its Dock icon, ⌘Tab) and the window it shows is one ottospace holds, ottospace comes
forward on that window's pane and gives it the keyboard, within half a second. ottospace shortcuts do not fire in that
mode. Press the return key, or click ottospace itself (rail, top bar, another pane), to return.

An app that refuses the pane's size is shown at its own size from the pane's top left, parked and
live: a larger window is cut off, a smaller one leaves a gap. A click in the gap gives the app the
keyboard; a click there while the app has it returns to ottospace, so a double-click in the gap
ends in ottospace. The preview follows an app that resizes its own window, within half a second.
The pane header says what the
window kept (`needs 120 pt more width`, `fixed size 232×320`). A window less than 24 pt smaller
than its pane gets no hint: some apps snap their size, as a terminal emulator does to whole
cells. `fit` beside the hint, or ⌘⌃0, moves the splits around the pane so it has the window's
size, as far as each side's 10% allows; a split at that limit passes the rest to the next split
above it. A pane that spans the tab on an axis keeps its size there. When no split can move,
the word `fit` is not shown and the hint says why (`fixed size 160×120 · no split to move`,
`· splits at their limit`). In a narrow pane the hint is cut short; `fit` and × stay. Nothing
is resized unless you ask.

## The return key

The return key works while any app is frontmost. It is F18 by default; set `OTTOSPACE_RETURN_KEY`
to another of `f13`-`f20`. If another program already holds the key, ottospace logs that and runs
without a return key. Most keyboards have no F18, so map Caps Lock to it (lasts until restart):

```sh
hidutil property --set '{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":0x700000039,"HIDKeyboardModifierMappingDst":0x70000006D}]}'
```

After this, Caps Lock no longer toggles capitals anywhere. The command replaces any key mapping
you already have. To undo it: `hidutil property --set '{"UserKeyMapping":[]}'`.

## Settings

ottospace keeps its files in `~/.ottospace` (another folder: set `OTTOSPACE_HOME`). It reads
`settings.json` there at start, so a change applies the next time you start it. Every key is
optional:

```json
{
  "terminal": { "font": "JetBrains Mono", "font_size": 14 },
  "repos": ["~/code", "~/notes"],
  "tasks": "~/notes/tasks",
  "sessions": true,
  "session_dirs": { "claude": ["~/.claude"], "codex": "~/.codex" },
  "session_quiet_minutes": 3,
  "apps": ["Microsoft Outlook", "Slack"],
  "windows": [{ "app": "us.zoom.xos", "title": "Zoom Meeting", "do": "ignore" }]
}
```

| Key | Meaning | Default |
|---|---|---|
| `terminal.font` | The terminal's font, by family name. Use a monospaced one | `Menlo` |
| `terminal.font_size` | In points, 6 to 72 | `13` |
| `repos` | Directories that are repositories or hold them, for the Repos section. `OTTOSPACE_REPOS` (`:` between directories) wins over it | `~/code`, `~/notes` |
| `tasks` | Where tasks are kept, one JSON file each. A relative path is inside `~/.ottospace` | `tasks` |
| `sessions` | List agent sessions that are no task's under Sessions in the Tasks section. `false`: no session file is ever read | `true` |
| `session_dirs` | Where the runners keep their sessions: `claude` is a list of Claude Code config directories (one for each account), `codex` is Codex's home. A runner left out keeps its default. An app started from the Dock does not see `CLAUDE_CONFIG_DIR` or `CODEX_HOME`: name other directories here | `~/.claude`, `~/.codex` |
| `session_quiet_minutes` | A task adopted from a session moves to Needs you once the session has been quiet this many minutes. `0`: a quiet session never moves its task | `3` |
| `windows` | Rules for windows ottospace must leave alone. Each rule selects windows by `app` (the app's name or bundle id), `title` and `subrole`; every field given must match, `*` stands for any text, and case does not matter. `"do": "ignore"`: the window never becomes a pane, when its app is hosted or when it opens later. `"do": "manage"` undoes a built-in rule. Your rules come first. Name an app by its bundle id to be sure. A window is judged once, when it is first seen, on the title it has then; rules are read at start | Built in: only standard windows are ever put in a pane, and Zoom's sharing controls, annotation layer and pop-ups are ignored |
| `apps` | Apps to offer while they are not running, by the name of the app as Finder shows it (`Visual Studio Code` for `Visual Studio Code.app`). It must be in `/Applications`, `~/Applications` or `/System/Applications` (or their `Utilities`); a name that is not found is left out. They show under "Not running" in Apps and in the picker; choosing one starts it in the background and puts its window in a pane | none |

A mistake in the file (a key that does not exist, a wrong value, a font that is not installed) is
shown in the status bar. That key takes its default and the others still apply. If the file is
not a JSON object, every default applies.

## Driving it from outside

ottospace listens on a Unix socket so tests and agents can inspect and drive it. The socket is mode
0600, at `$OTTOSPACE_SOCKET` or `$TMPDIR/ottospace.sock`. Send one JSON request per line. The reply
is one JSON line: the full state (model, mode, pane bounds, terminal grids) or
`{"ok":false,"error":"..."}`. `ottospace ctl` sends one request, prints the reply, and exits non-zero
if the request was refused. `cargo run` does not put `ottospace` on your `PATH`: use the built binary.

```sh
alias ottospace=./target/debug/ottospace
ottospace ctl '{"cmd":"split","axis":"down"}'
ottospace ctl '{"cmd":"type","text":"ls\r"}'
ottospace ctl '{"cmd":"open_app","name":"Safari"}'
ottospace ctl '{"cmd":"section","section":"apps"}'
ottospace ctl '{"cmd":"mark_add","pane":3,"name":"notes","key":"n"}'   # a way back to pane 3
ottospace ctl '{"cmd":"mark_go","name":"notes"}'     # go to it, or open it again
ottospace ctl '{"cmd":"sneak"}'                      # labels on ottospace's controls: `sneak.targets`
ottospace ctl '{"cmd":"key","keys":"a"}'             # type a label: that control acts as if clicked
ottospace ctl '{"cmd":"state"}'
```

`ottospace skill` prints every request with an example, and what the state fields mean. Fields are
in `Request` in [`crates/ottospace/src/control.rs`](../crates/ottospace/src/control.rs). Unknown commands and
fields are refused.

A typed TypeScript client for this socket is in [`sdk/`](../sdk/README.md) (early).

The `alias` above is for a source build. An installed `ottospace` is on your `PATH` already.

## Agent skill

`ottospace skill` prints a guide for coding agents: the requests, the state fields, and rules for
using them safely. To make it a Claude Code skill, install the stub that points at it:

```sh
mkdir -p ~/.claude/skills/ottospace
ottospace skill --stub > ~/.claude/skills/ottospace/SKILL.md
```

The stub tells the agent to run `ottospace skill`, so the guide always matches the installed
version. A test checks that every request shown in the guide is one the app accepts.

The guide itself: [`crates/ottospace/skill/guide.md`](../crates/ottospace/skill/guide.md).

## How app windows are hosted

macOS cannot reparent another process's window, so ottospace fakes embedding with the Accessibility
(AX) API and ScreenCaptureKit (SCK). An inactive app window is parked off-screen with one pixel on
a display, and its pane shows an SCK stream of it. Clicking the pane moves the real window over it
and replays the click, so input is native. A pane shows one title bar: the stream leaves out the
app's own, and the live window's covers the pane header. Measurements and risks: [docs/ai/hosting-windows.md](ai/hosting-windows.md).

## Design docs

[`docs/ai/1-initial-brainstorm/`](ai/1-initial-brainstorm/) holds the AI-derived notes for the
first brainstorm:

- [plan.md](ai/1-initial-brainstorm/plan.md): model, phases and decisions.
- [quick-capture.md](ai/1-initial-brainstorm/quick-capture.md): quick capture and task lifecycle.
- [screen-share-privacy.md](ai/1-initial-brainstorm/screen-share-privacy.md): hiding panes from screen sharing.
- [scripting.md](ai/1-initial-brainstorm/scripting.md): scripting in TypeScript.
- [zoom-meeting.md](ai/1-initial-brainstorm/zoom-meeting.md): Zoom meeting layout and participants.
- [zoom-ax.md](ai/1-initial-brainstorm/zoom-ax.md): what a Zoom meeting exposes through accessibility, measured.
- [accessibility.md](ai/1-initial-brainstorm/accessibility.md): reading app accessibility trees, toward sneak mode and agent actions.
- [multi-monitor.md](ai/1-initial-brainstorm/multi-monitor.md): options for more than one display, and a staged recommendation. Nothing measured yet.
- `background/`: mockup images.

[marks.md](ai/marks.md): marks, what one records, and how they sit beside session restore, pins and tasks.

[routines.md](ai/routines.md): routines and agents from ottobot, in stages, toward ottospace subsuming it.

[`docs/ai/backlog.md`](ai/backlog.md) is an older backlog. `docs/ideas/` holds the owner's own
notes.
