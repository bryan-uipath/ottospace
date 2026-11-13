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
| Scrollbars on every list that scrolls, the key reference and a terminal's scrollback: drag the thumb, click the track to page (in a terminal a click beside the thumb is the terminal's). They follow the system's "Show scroll bars" setting | Works today |
| Visual mode: select terminal text with vi keys | Works today |
| Tasks inbox: what needs you first, then the other tasks with tabs open (Open), then Running, Captured, On hold, Done. Headings fold (click, or ↵ or space in list mode). A task that is done stays under Done with its tabs open. Each row shows its tags, what it has open and its age; hover it for start, snooze, hold and dismiss buttons. After one of those by mouse, a line at the top offers Undo for 6 seconds. Kept in `~/.ottospace/tasks` | Works today |
| "Pop next" (⌘⇧N): open the next task that needs you | Works today |
| Sessions in the Tasks section: the Claude Code and Codex sessions of the listed repositories that are no task's (one you started by hand in a terminal). Each row shows the runner, the repository, the session's title, active or idle, what it does now, when it was last active, and the model. Adopt makes a task of one; Hide removes it until the next start. Read-only: nothing is sent to a session | Works today |
| New task box (⌘N): one line, with [tags](#tags): `#project`, `>repo`, `@agent`, `/path`, `$skill`, `p1`–`p9`. `@agent`, `/path` and `$skill` are stored and shown; nothing acts on them yet | Works today |
| Home screen: a tab with no panes shows the new-task line, the tasks that need you, and each listed task as a node on a lattice (left to right by project; within a project the older task is farther). A click on a node or a row opens the task | Works today |
| Tasks own their tabs: opening a task shows only its tabs; the others keep running off the bar | Works today |
| Tabs, each with its own split tree | Works today |
| Pane picker: choose a terminal, a running app, or (once you type) any installed app for a new pane or tab | Works today |
| Icon rail with a secondary bar; Apps lists running apps with their icons | Works today |
| Repos: each repository's name over its checkouts (the branch, then `Default` or a linked worktree's directory); open a terminal in one | Works today |
| List mode (⌘⇧E): drive the bar with the keyboard | Works today |
| Sneak mode (⌘⌃G): a letter label on each of ottospace's own controls; typing it clicks the control | Works today |
| Marks (⌘B, ⌘⇧B, ⌘' and a letter): named ways back to a pane or a tab, kept in `~/.ottospace/marks.json` | Works today |
| Capture (⌃⌥⌘Space, from any app): a frozen picture of the pane in use to circle and write on, collected in a draft in the right bar; the task it becomes has a brief with the pictures, the notes and the text under each annotation (`task_brief`). Kept in `~/.ottospace/materials`. Starting an agent with the brief is not built | Works today |
| Workbooks (⌘⇧S): save a tab under a name (its splits, each terminal's directory, each app) and open it again from the Workbook section. Kept in `~/.ottospace/workbooks`. One tab each; an app comes back as its window, not on the page or document it showed | Works today |
| `settings.json`: terminal font, repository directories, tasks folder, apps to offer, window rules | Works today |

### Roadmap: designed, not built

None of these is in the app today.

| Capability | State |
|---|---|
| Other key modes: insert; sneak labels inside a hosted app's window | Designed |
| Tasks: a worktree at kickoff, a brief pane, automatic cleanup when done | Designed |
| Quick capture (⌘⌘): turn something on screen into a task and dispatch it to an agent | Designed |
| Focus blocks: hide Slack, with an agent that can break through | Designed |
| Fast app switcher | Designed |
| Workbooks: several tabs in one; the URL, channel or document each app showed; a workbook on the rail | Designed |
| TypeScript scripting, with contextual quick actions in an action deck (Zoom meeting layout, spin for stand-up) | Designed |
| Hide private panes while screen sharing | Designed |
| Session restore | Designed |
| Terminals survive quitting and updating | Designed |

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
| ⌘E | Pick what goes in a new pane: a terminal or a running app. Type to filter: then every installed app that matches is offered too, under "Not running" (those of the `apps` setting first); choosing one starts it in the background. ↑/↓ or ⌃N/⌃P move, ↵ opens, Esc closes |
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
| ⌘/ | Show every key. In a small window the list scrolls: J / K or the arrows, space or Page Down / Page Up, Home / End |
| F18 | From any app: take the keyboard back and bring ottospace forward |
| ⌘T | New tab: pick what opens in it |
| ⌘1 … ⌘9 | Select a tab on the bar |
| ⌘⇧] / ⌘⇧[ or ⌃Tab / ⌃⇧Tab | Next / previous tab |
| ⌘⇧W | Close the tab and its panes |
| ⌘⇧S | Save the tab on show as a [workbook](#workbooks). On a tab that came from one, or was saved as one: update it |
| ⌘Q | Quit. While a terminal runs something (a program in front of its shell, or a background or stopped job) it asks first, and lists them: ↵, Q or ⌘Q again quits; Esc, any other key or any other shortcut cancels and does nothing else |
| Wheel / trackpad | Scroll a terminal back through its history |
| ⇧PageUp / ⇧PageDown | Scroll a terminal a page |
| ⇧Home / ⇧End | Top / bottom of a terminal's history |
| Drag / double-click / triple-click | Select text / a word / a line in a terminal |
| ⌘C | Copy the terminal's selection (nothing selected: does nothing). In a [text field](#keys): its selection |
| ⌘V | Paste into the terminal, or into the open text field |
| ⌘N | New task: a box opens over the workspace, or in place on the home screen. Type what needs doing, with [tags](#tags): `#project`, `>repo`, `@agent`, `/path`, `$skill`, `^reference`, `p1` to `p9`; a dropdown finishes the one at the cursor. ↵ adds the task; ⌘↵ adds it and starts it; Esc closes. [Its keys](#keys) |
| ⌘⇧N | Next task: shows the tabs of the first task that needs you |
| ⌃⌥⌘Space | Capture, from any app: the pane in use (a hosted app you are in, else the focused pane) is frozen as a picture to draw on, and a draft opens in a bar on the right, with the new-task line as its title. A live app is parked first; "↩ back to …" in the bar returns to it. On the picture: drag circles, ⇧-drag draws a box, ⌥-drag a freehand line, a click places a note to type (↵ keeps it). ⌘Z and ⇧⌘Z go over the steps; a click on a shape's line selects it and ⌫ removes it. ↵ goes to the title, Esc leaves the picture; a click on another pane captures that one too. In the title ↵ adds the task and ⌘↵ adds and starts it; × or Esc cancels, after a question if anything was captured. The workspace stays in use beside the bar, and ⌘N goes to the draft's title. See [Capture](#capture) |
| ⌘B / ⌘⇧B | Mark the focused pane / the tab on show; on one that is marked, take the mark off (the Marks section offers it back for six seconds, and ⌘B again within them puts the same mark back). The mark is named for the app, the terminal's directory or the tab's title. It records what to open, not a window or a process: a terminal's directory as it is then, an app's bundle id, name and window title, a tab's title, task and split tree. A marked pane or tab shows ◆ and its letter |
| ⌘' then a letter | Go to the mark with that letter. The status bar shows `MARK?` while it waits; Esc, or a letter of no mark, ends the wait and nothing else happens. If the mark's pane or tab is open, its tab comes forward and the pane takes focus (an app pane is focused, not entered). Else it is opened: a terminal in the recorded directory, the app's window in a pane (the one with the recorded title, else its only window, else its main one; an app that is not running is started), or the tab with its splits. A tab opens with what can be opened: a pane whose directory is gone or whose window could not be hosted is empty and says why, and going to the tab again tries it again |
| ⌘⌥1 … ⌘⌥9 | Go to one of the first nine marks, in the order of the Marks section |
| ⌘⇧E | List mode: the bar (Apps, Workbook, Tasks, Repos, Marks) takes the keyboard. On a workbook `r` renames it, `x` deletes it and `u` takes the last delete or update back. On a mark `x` removes it, `r` renames it, `m` gives it a letter, ⇧J / ⇧K move it down / up, and `u` puts the last removed mark back. `j`/`k` or arrows move; ↵ opens the row (⇧↵ splits down); in Tasks a heading is a row too, and ↵ or space on it folds or unfolds it; Tab changes section; on a task `s` snoozes one hour, `h` holds, `x` dismisses, `u` undoes; on an app that is not running `p` pins or unpins it, `x` hides a suggestion and `u` takes the last of these back; in Apps `/` types in the filter of "All apps" (↵, ↓ or Esc gives the keys back to the rows); Esc or ⌘⇧E leaves |
| ⌘⌃G | Sneak mode: every control ottospace draws (rail icons, bar rows, tabs and their ×, top-bar buttons, pane headers and their ×, each pane, the home screen's line, rows and nodes) gets a short label, home-row letters first. Type a label and the control does what a click does; a pane's label focuses it, or gives an app pane the keyboard. A × (pane or tab) always has a two-letter label. Two-letter labels narrow on the first letter; ⌫ takes it back. Esc, a click, a letter that goes on with no label, or any ⌘ shortcut leaves; the shortcut then runs (⌘C copies, ⌘V pastes). Other keys do nothing. The labels follow what is drawn: if a control moves its label moves, and if it goes its label goes. What happens by itself (a shell exits, an agent files a task) does not end the mode. Starting it hides the accessibility overlay (⌘⌃A). Not ⌃G alone: a terminal needs that key. On a keyboard layout with no Latin letters, the keys type the letters they have under ⌘. From `APP` mode return first. Controls inside a hosted app's window get no label. Sneak presses no control inside another app and sets no value; entering an app pane raises its window, as a click on the pane does |
| ⌘⇧C | Visual mode on the terminal: `hjkl`/arrows, `w b e 0 ^ $`, `H M L` move; `v` / `V` select; `y` or ↵ copies and leaves; Esc leaves |

**Text fields.** The new-task box's line and the picker's filter are the same field, with the
keys of a macOS text field:

| Key | Action |
|---|---|
| ←/→ · ⌥←/→ · ⌘←/→ | Move by character, by word, to the start or end of the line. Also Home/End, ⌃A/⌃E, ⌃B/⌃F. With ⇧: select |
| ⌘A | Select everything |
| ⌫/⌦ · ⌥⌫/⌥⌦ · ⌘⌫/⌘⌦ | Delete a character, a word, to the start or end of the line. Also ⌃H, ⌃D, ⌃W, ⌃U, ⌃K |
| ⌘C / ⌘X / ⌘V | Copy, cut, paste. A pasted line break becomes a space |
| ⌘Z / ⇧⌘Z | Undo, redo: typing a word at a time, up to 100 steps |
| Edit menu | Undo, Redo, Cut, Copy, Paste and Select All act on the field while it has the keyboard |
| Mouse | A click places the cursor (⇧-click selects to it), a drag selects, a double click takes a word, a triple click the line |

Typed text replaces the selection. Movement and deletion go by whole characters as you see them
(an emoji, a letter with its accent). Input methods and dead keys compose in place, underlined.
A line wider than the field scrolls with the cursor. The picker's filter holds 200 characters.
While the quit question is over a field, nothing edits it.

**The new-task box.** ⌘N opens it over the workspace; while the home screen shows (a tab with
no panes) the home screen's own line is the box, in place, and a click on that line opens it.

| Key | Action |
|---|---|
| ↵ | Takes the dropdown's highlighted row. With no row highlighted, or when the word is that name already, adds the task. Over the workspace the box then closes; on the home screen the line stays, empty, for the next task |
| ⌘↵ | Adds the task and starts it (shows its tabs), whatever is highlighted |
| Tab | Takes the highlighted row, or the first |
| ↑ / ↓ or ⌃P / ⌃N | Move over the dropdown's rows. With no dropdown: to the line's start or end |
| Esc | Closes the dropdown. With no dropdown open: closes the box |
| ⌘O, or `+ file` | The system's file picker: what is chosen goes into the line as `/path` words |
| ⌥-click on a tab or a pane | Puts its `^reference` into the line. The tab is not selected, the pane not focused. While ⌥ is held the tab or pane under the pointer is outlined |

- **The dropdown** opens when the cursor is in a tag: up to 8 names for it, with what each is
  and a word of detail. Names that start with what is typed come first, then those where a
  word of the name does (`flow-app` for `ap`), then those that have its letters in order. The
  letters matched are drawn stronger. A bare mark (`#`) lists what there is and highlights
  nothing; once a letter of the name is typed the first row is highlighted. Typing narrows it;
  a click on a row takes it.
- **Taking a row** replaces the whole word under the cursor with the name and a space. A
  folder takes no space: the dropdown stays open on what is inside it. A space typed at
  the end of a name that is one of the rows takes that row, spelled as it is kept; inside a
  word a space is a space.
- **Tags that are understood** are drawn in their colour on a faint chip, in the line itself;
  a word that names nothing stays plain.
- **Files.** Files and folders dragged from Finder onto the box, picked with ⌘O, or copied in
  Finder and pasted (⌘V), each go in as a `/path` word, after the word the cursor is in: one
  inside the home folder as `~/..`, a space with a backslash before it.
- **When it cannot.** A name the line has no room for (it holds 500 characters) is not put in
  in part: the line stays as it is and the box says "no room". ↵ then adds the task as typed.
  Files left out (a name too long, one with a line break) are counted in the same place.

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

| Tag | Means | The dropdown offers |
|---|---|---|
| `#project` | The project the task serves | Projects other tasks name |
| `>repo` | The repository the task works in | The Repos list (repositories and worktrees) |
| `@agent` | Who works it: a runner (`@claude`, `@codex`: its default model), a runner and a model (`@claude-opus-5-5`, `@codex-gpt-6-astra`), or a saved agent | `claude`, its models, `codex`, its models, then each `~/.ottospace/agents/<name>.md` if that folder exists |
| `/path` | A path the task is about: a word that starts with `/` or `~/` | The files and directories under what is typed |
| `$skill` | A skill to load for the task | Skill folders (those with a `SKILL.md`) in `~/.claude/skills`, `$CLAUDE_CONFIG_DIR/skills`, `~/.codex/skills` (or `$CODEX_HOME/skills` if that is set), and the `.claude/skills` of the task's `>repo` |
| `^reference` | Something open that the task points at. A space in the name has a backslash before it: `^Tab\ 1` | What is open now: each tab (its title), each pane (a terminal by its directory's name, an app pane by the app's), each [mark](#marks), and the installed apps that are not running. Two of one name: the later one is `name-2` |
| `p1` to `p9` | The priority. 1 is most urgent; 3 when none is given | |

```
fix zoom flicker #canvas >canvas-app @claude p2
review the export PR >canvas-app @codex $code-review /src/export
```

- **A `^reference`** is kept on the task as it was when the task was added. One taken from
  the dropdown or by ⌥-click is kept even if what it names closes before ↵, and a name in a
  line means one thing as long as the line is there: a second pane of that name is `name-2`.
  A name is read as typed first (`^notes\ v1.`), then without the punctuation a clause ends in.
  - **In the task's file** (`tasks/<id>.json`, which may be in a folder you share): what it
    is (`tab`, `pane`, `mark`, `app`), its name, its id then, and the app's name. A terminal
    is named by its directory's name only.
  - **In memory, and in the state, until ottospace quits**: also a tab's title, an app
    pane's window title, a terminal's full directory and its program, where an app is
    installed. None of these is written to the file. Ids do not last past a quit.
  - Nothing acts on a reference yet: it is stored, counted in the task's row, and in the
    state as `refs`. Opening a task does not open what it points at.
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
  then the word without the punctuation. A space in a path or a reference is written with a
  backslash before it, `~/My\ Notes`, and so is any other character that would end the word (a
  no-break space, as in a macOS screenshot's name) and a backslash itself (`a\\b`); taking a
  row, a drop or ⌥-click writes them for you. The task keeps the name without them. A name
  with a line break or a tab in it cannot be in the line: it is left out, and the box says so.
- **A file dropped, picked or pasted** is that very file: `/tmp/report.txt` put in whole is
  not looked for inside the line's `>repo`, as a path you type is.
- **How many.** One `#project`, one `>repo`, one `@agent`, one priority: of two, the last one
  counts. Every `/path` and `$skill` is kept, each once (at most 20 of each). `@agent` and
  `$skill` are kept as their file or folder spells them, whatever the case typed.
- **A runner and a model.** `@claude-opus-5-5` is Claude Code with the model `claude-opus-5-5`
  (Claude's ids start with `claude-`, so the tag is the id); `@codex-gpt-6-astra` is Codex with
  `gpt-6-astra`. The dropdown offers `claude-fable-5-2`, `claude-opus-5-5`, `claude-sonnet-5-5`,
  `claude-haiku-5-5` and `codex-gpt-6-astra`; the `agent_models` [setting](#settings) replaces a
  runner's list. A word of the name finds a row: `@opus`. A model that is not listed works by
  typing it: any `@claude-<name>` or `@codex-<name>` (small letters; then letters, digits, `.`,
  `-`, `_`) is a runner and a model, kept as typed. So a saved agent whose file is named as a
  runner, or `claude-…` or `codex-…`, is not offered: the name means the runner or its model.
  Name saved agents otherwise.
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

## Capture

The capture key turns what a pane shows into material for a task. Each capture keeps the picture
as taken, the picture with your annotations drawn on it, the annotations themselves, what the pane
was (an app's name, bundle id and window title; a terminal's directory and the program in front),
and the text under each annotation: for an app pane the labelled accessibility nodes there (read
once, read-only: nothing in the app is pressed or set), for a terminal the rows.

- The task's row says how many materials it has. Opening the task shows its brief in the right bar.
- The files are in `~/.ottospace/materials/<task id>-<a word of its own>/`, readable by you alone: `brief.md`,
  `brief.json`, and for each capture `1.png`, `1.annotated.png`, `1.json`. They are never in the
  tasks folder. A cancelled draft leaves nothing.
- A terminal opened in a task's tab has `OTTOSPACE_TASK`; an agent there reads the brief with
  `ottospace ctl "{\"cmd\":\"task_brief\",\"id\":$OTTOSPACE_TASK}"`.
- In the brief, everything that came from another app or a terminal is quoted and labelled as
  untrusted: a message someone else wrote is data for the agent, not an instruction.
- A window your `windows` rules ignore is never captured.
- Only you start a capture. No socket request does (outside a test run), so no program can ask
  ottospace for a hosted app's picture or text.
- With no title typed, the task is named "Capture from an app" or "Capture from a terminal".
- ⌘Q with a draft that holds a capture asks before it discards it.
- If a window was minimised or on another Space, the brief says its picture may be stale.
- The annotated picture is what an agent is sent to first. Electron and Chrome windows (Slack,
  VS Code) expose little text to accessibility; their capture is the picture, your notes and
  the window's title. ottospace does not switch an app's accessibility mode on, and runs no OCR.
- Not built: starting an agent with the brief; removing a task's captures with the task.

Design: [capture.md](ai/capture.md).

## Sessions

An agent is a saved definition. A session is one running conversation of Claude Code or Codex.
The Tasks section lists, under **Sessions**, the sessions of your listed repositories (worktrees
included) that no task has adopted: for example a Claude Code you started by hand in a terminal.

- **What a row shows.** The runner (`claude` or `codex`), the repository or worktree, a dot that
  is lit while the session is active (its file changed in the last 30 seconds), what it does now
  (`shell · cargo test`, `network · git push`, `files · src/main.rs`), and when it was last
  active. Idle sessions older than a day are behind "N older".
- **Title and model.** Between the two lines a row shows the session's title, on one line, when
  the runner keeps one: the name you gave a Claude Code session (`/rename`), else the title
  Claude Code wrote for it; for Codex, the thread's name in its `session_index.jsonl`, which
  not every session has. ottospace writes no title itself and calls no model. The last line
  ends with the model in short: `claude-opus-4-7` shows as `opus-4.7`, another name as it is.
  A title is text a model wrote from what the session read, so it is only drawn: one line, at
  most 80 characters, no control or invisible characters. It never names a task, sets a tag or
  reaches a terminal.
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
  prompt, a file's contents or a command line. The one free text is the session's title (above).
  Design: [sessions.md](ai/sessions.md).
- Turn it off with `"sessions": false`. See [Settings](#settings) for where sessions are read from.

## Workbooks

A workbook is a saved, named tab for a context that comes back, e.g. "Maestro DRI". It is not a
task: no agent works it and it is never done.

- **Save**: ⌘⇧S, File ▸ Save Tab as Workbook, or `Save tab` in the Workbook section. It records
  the tab's splits and their sizes, each terminal's directory as it is then, and each app pane's
  app (bundle id and name). It records no scrollback, no command, and no window title, URL or
  document. A tab with nothing in it is not saved.
- The new workbook is named for the tab, and the section opens with the name selected: type
  over it, ↵ keeps it, Esc keeps the tab's name. No two workbooks share a name (`DRI`, `DRI 2`).
- **Save again** on that tab, or on a tab opened from a workbook, updates the workbook. The
  section says `Updated · Undo` for six seconds.
- **Open**: click its row, or ⌘⇧E and ↵. A new tab with that name and layout. Each terminal
  starts in its directory; if the directory is gone it starts in your home folder, and the
  status bar says so. Each app pane gets a window of its app as a tab's mark does: one no pane
  holds (the only one, else the main one); an app that is not running is started. A pane whose
  app is not installed, or whose window is in another pane, stays empty and says why. If the
  workbook's tab is open already, it is selected: a dot marks the row.
- **Rename, Delete**: the words on its row (hover it, or select it in list mode), or `r` and
  `x` in list mode. Deleting leaves the open tab alone; `Workbook deleted · Undo` shows for six
  seconds (`u` in list mode).
- Kept in `~/.ottospace/workbooks`, one file each, readable by you alone. A file there that
  ottospace cannot read is counted in the status bar and left as it is.
- Only you save, open, rename or delete a workbook: a socket client's key, click or label is
  refused. The state lists them (`workbooks`: names and pane kinds, no directories).

Not built: several tabs in one workbook, and bringing an app back on the URL, channel or
document it showed. Design: [workbook.md](ai/workbook.md).

## Terminal panes

A terminal pane runs your shell on a PTY. The pane closes when the shell exits. It keeps 10,000
lines of history. Typing returns to the bottom. In programs that use the mouse or the alternate
screen (vim, less, htop) the wheel does nothing, and ⇧PageUp and friends go to the program.
A click on a terminal focuses it and clears its selection; a drag there selects, even in an
unfocused pane. Paste is bracketed when the program asks for it.

## App panes

To put an app window in a pane, open Apps on the left rail and click the app. It fills the focused pane if that is
empty, otherwise it splits right. Shift-click splits down. New windows of a hosted app get their
own pane.

Below the running apps the section lists apps that are not running. A click starts one in the
background and puts its window in a pane (ottospace waits up to 30 seconds for the window):

- **Not running**: the apps you pinned (★), then the apps in your Dock, in the Dock's order. At
  most 12 of the Dock's show; "and N more" unfolds the rest. The Dock is read (its
  preferences file, never changed) when ottospace starts, when the Apps section opens, when
  list mode comes on or changes section (⌘⇧E, Tab), when the bar's ↻ is pressed, and when the
  picker opens. A change you make in the Dock shows at the next of those.
- **All apps (N)**: every other installed app, by name. Folded until you click the heading. Open,
  it has a filter: click it or press `/` in list mode, and type.

Hover a row (or select it in list mode) for its buttons. **Pin** (`p`) keeps the app at the top of
the list, and **Unpin** takes it off. **Hide** (`x`), on an app of the Dock, stops suggesting it;
it is still among all apps. Each of these writes `~/.ottospace/settings.json` (`apps`,
`apps_hidden`), and for six seconds a line at the top offers to undo it (`u` in list mode).

- The file is read again at each write, and one entry is added to or taken off the list as it is
  then: a line you added by hand since ottospace started, or one another ottospace wrote, stays.
- Only the list changed is touched: every other key and the file's spacing stay.
- Not written, with a notice in the status bar, and the pin then lasts until ottospace quits:
  a file that is not a JSON object; a list that is not a list of names (`"apps": ["Slack", 3]`):
  nothing in it is dropped for not being understood; a `settings.json` that is a link to a file
  that is not there.
- The new file takes the old one's place whole, with the old one's permissions (only you can
  read a new one); a link to a file elsewhere stays a link.
- An app is written by its name only if that name means the same bundle when read back: the
  first `Name.app` in `/Applications`, `~/Applications`, `/System/Applications`,
  `/Applications/Utilities`, `/System/Applications/Utilities`, in that order. Any other app
  (one in another folder, or the second of two with one name) is written by its path.
- Only your own key or click pins, unpins, hides or undoes: a socket client's key or sneak
  label is refused ("settings are the user's to change").
A running app has no Pin button yet. Original window frames are saved to disk and restored on quit or after a crash.

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
  "agent_models": { "claude": ["claude-opus-5-5"], "codex": ["gpt-6-astra"] },
  "capture_key": "ctrl-alt-cmd-space",
  "capture": { "keep_images": true, "ax": true },
  "apps": ["Microsoft Outlook", "Slack"],
  "apps_hidden": ["Music"],
  "suggest_dock_apps": true,
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
| `agent_models` | The models the `@agent` dropdown offers after each runner, in place of the built-in list: ids as the runner's CLI takes them. `claude` and `codex` are each a list; a runner left out keeps its own, an empty list offers the runner alone. An id that does not start with the runner's name gets it in the tag: `gpt-6-astra` is `@codex-gpt-6-astra`, `opus` is `@claude-opus`. An entry that is not a name (letters and digits, with `.`, `-` or `_` between them one at a time; at most 64) is left out and named in the status bar | `claude-fable-5-2`, `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`; `gpt-6-astra` |
| `capture_key` | The key that captures the pane in use, from any app: modifiers (`ctrl`, `alt`, `cmd`, `shift`) and a key, with `-` between them. `"off"`: no key. `OTTOSPACE_CAPTURE_KEY` wins over it. Not `ctrl-alt-space`: macOS uses it to change the input source | `ctrl-alt-cmd-space` |
| `capture.keep_images` | Keep a capture's pictures on disk. `false`: no picture is ever written | `true` |
| `capture.ax` | Read an app pane's accessibility tree with its picture. `false`: it is never read for a capture | `true` |
| `windows` | Rules for windows ottospace must leave alone. Each rule selects windows by `app` (the app's name or bundle id), `title` and `subrole`; every field given must match, `*` stands for any text, and case does not matter. `"do": "ignore"`: the window never becomes a pane, when its app is hosted or when it opens later. `"do": "manage"` undoes a built-in rule. Your rules come first. Name an app by its bundle id to be sure. A window is judged once, when it is first seen, on the title it has then; rules are read at start | Built in: only standard windows are ever put in a pane, and Zoom's sharing controls, annotation layer and pop-ups are ignored |
| `apps` | The pinned apps: listed first under "Not running" in Apps, and in the picker before anything is typed; choosing one starts it in the background and puts its window in a pane. Each is the name of the app as Finder shows it (`Visual Studio Code` for `Visual Studio Code.app`), looked for in `/Applications`, `~/Applications` and `/System/Applications` (and their `Utilities`), or the path of its bundle (`~/Tools/Lumen.app`). One that is not found is left out of the list and kept in the file. Pin and Unpin in the Apps section write this list: the name if it means that same bundle when read back (the first of that name in those folders, in that order), else its path. A name that is found only once ottospace runs (the app was installed since) is listed from the next scan. (Once you type in the picker it offers every installed app in those folders, these first) | none |
| `apps_hidden` | Apps of the Dock not to suggest in Apps, written as `apps` is. Hide in the Apps section writes this list | none |
| `suggest_dock_apps` | List the Dock's apps that are not running under "Not running". `false`: only pinned apps are listed there, and the Dock is not read | `true` |

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
ottospace ctl '{"cmd":"task_brief","id":7}'          # a task's brief: the ask, and what was captured for it
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

[capture.md](ai/capture.md): the capture key, annotate mode, what a capture stores, and how a brief quotes it.
[marks.md](ai/marks.md): marks, what one records, and how they sit beside session restore, pins and tasks.
[workbook.md](ai/workbook.md): workbooks beside tasks, marks and session restore; stage 1 as built, and what is later.
[terminal-persistence.md](ai/terminal-persistence.md): terminals and agents that outlive a quit or an update, held outside the app; with session restore, tasks come back as they were.

[routines.md](ai/routines.md): routines and agents from ottobot, in stages, toward ottospace subsuming it.

[`docs/ai/backlog.md`](ai/backlog.md) is an older backlog. `docs/ideas/` holds the owner's own
notes.
