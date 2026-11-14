# ottospace reference

Everything beyond the [cheat sheet](cheat-sheet.md). Front page: [README](../README.md).
Install and update: [install.md](install.md). Working on ottospace: [development.md](development.md).

ottospace is a macOS workspace that tiles terminals and real app windows in one window. It is for
people who run several coding agents and want them, and the apps around them, in one place.
Agents and tests can drive it over a local control socket. Rust + [GPUI](https://gpui.rs).

## Status

Early prototype, macOS only. Today you can split a window into panes, run your shell in a pane,
host another app's window in a pane, move focus by keyboard, keep a tasks inbox, and drive all of
it over the control socket. Tabs and layout come back after quitting; terminals do not keep
running, and the rest of the design is not built: see the
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
| Tasks inbox, by state: Needs you, Running, Later (captured or on hold), Done (last, folded). Headings fold (click, or ↵ or space in list mode). Each row shows its tags, what it has open and its age, or while its session works what it does now; and the session's model. A click opens its conversation; hover it for its card (state, tags, started, agent time, workspace, tokens by model, context) and for snooze, hold and dismiss buttons. After one of those by mouse, a line at the top offers Undo for 6 seconds. Kept in `~/.ottospace/tasks` | Works today |
| Tasks by project: State \| Project in the section's header, kept in `tasks_view` in the settings. One foldable group a `#project`, by name, with a bar of its tasks' states (hover for the numbers), its open tasks (needs you, running, later), then a folded "Done · n"; the tasks of no project last. Under the list: `Today  n done · n running · agent time` | Works today |
| A task's progress, kept in its file: when its agent started and when it was done, time at work (activity until a quiet gap of two minutes), and tokens by model (input, output, cached; a sub-agent's apart), summed from its sessions, so they outlive the session files. Tokens only: no prices | Works today |
| "Pop next" (⌘⇧N): the conversation of the next task that needs you; the tab on show stays | Works today |
| Sessions in the Tasks section: the Claude Code and Codex sessions of the listed repositories that are no task's (one you started by hand in a terminal). Each row shows the runner, the repository, the session's title, active or idle, what it does now, when it was last active, and the model. A click opens its conversation in the drawer (`claude --resume`, `codex resume`), unless it is at work in another terminal; Adopt makes a task of one; Hide removes it until the next start | Works today |
| A task's agent in the right drawer: a click or ↵ on a task starts its agent (`claude` or `codex`, by its `@agent`, else `default_agent`) beside the workspace, which stays as it is. See [Flow of a task](#flow-of-a-task) | Works today |
| Native conversation for Claude Code: the drawer drives `claude -p` (stream-json) and draws the reply itself: markdown (headings, emphasis, lists, quotes, tables), code blocks with Copy, links that open only on your click (web pages in the browser; files of the task's repository revealed in Finder), local images of the repository as thumbnails, tool calls as compact rows, a file edit's change as a diff card under its call, a working row while a turn runs, Copy and Copy as Markdown on each message, and a composer (↵ send, ⇧↵ new line). Permission requests are cards: Allow this one request as shown, or Deny (with a message if you like); a request too long to show is opened in full before Allow is offered. `Chat \| Terminal` in the header switches to the CLI's own TUI and back on the same session. A session at work in another terminal is watched read-only (what it calls and edits, not what it says; "Watching · updates per message" in the header), with Take over once it is idle. Codex stays in a terminal. No text selection by dragging yet: copy by message or block | Works today |
| Right rail: a badge per open conversation of a live task (needs you first, then running, then the last opened; at most 8), and `+` for a new task. A dismissed task leaves it; a done one stays only while shown or its agent runs | Works today |
| New task (⌘N), in the drawer: one line, with [tags](#tags): `#project`, `>repo`, `@agent`, `/path`, `$skill`, `p1`–`p9`, picks for where and who, and a line that says what ↵ will do. A `/path` stays in the title the agent reads, and a folder taken from its dropdown is where the agent runs; `$skill` is stored and shown, nothing acts on it yet | Works today |
| Home screen: a tab with no panes says what goes there (Open an app ⌘E, Terminal ⌘D), the tasks that need you, and each listed task as a node on a lattice (left to right by project; within a project the older task is farther). A click on a node or a row opens its conversation. With nothing open at all, the drawer shows a new task by itself | Works today |
| Task groups on the one tab bar: a tab may be a task's, and each task's tabs sit side by side behind its badge (its two letters, bordered in its own colour, its state as a dot), underlined in its colour; the tabs of no task last. Starting a task takes the tab on show if that is no task's; ⌘T joins the task on show; drag a tab onto a group to move it there. The same badge on its panes, its row, the rail and the drawer. Selecting a task's tab shows its conversation in the open drawer | Works today |
| Tabs, each with its own split tree | Works today |
| Pane picker: choose a terminal, a running app, or (once you type) any installed app for a new pane or tab | Works today |
| Icon rail with a secondary bar; Apps lists running apps with their icons | Works today |
| Repos: each repository's name over its checkouts (the branch, then `Default` or a linked worktree's directory); open a terminal in one. + in the header adds a directory to `repos` in the settings; Remove on a name takes it off | Works today |
| List mode (⌘⌃E): drive the bar with the keyboard | Works today |
| Sneak mode (⌘; or ⌘⌃G): a letter label on each of ottospace's own controls, and on the buttons and text fields of the focused app pane; typing it clicks the control, presses the app's button, or focuses its text field | Works today |
| The wheel over an app pane you are not in brings its window up for the scroll (a peek): `PEEK` in the status bar, parked again when the pointer leaves or the wheel is quiet | Partly verified: on CI the window comes up, the app is in front and the scrolls after the first reach it. The first scroll is lost. The window parking again by itself is unit-tested, not yet seen on CI |
| [Command palette](#command-palette) (⌘P): every command of the menu bar by name, with its key; a Developer group | Works today |
| [Accessibility inspector](#accessibility-inspector): a hosted app's accessibility tree in the right drawer's place, read-only | Works today |
| Marks (⌘B, ⌘⇧B, ⌘' and a letter): named ways back to a pane or a tab, kept in `~/.ottospace/marks.json` | Works today |
| Capture (⌃⌥⌘Space, from any app): a frozen picture of the pane in use to circle and write on, collected in a draft in the drawer; the task it becomes has a brief with the pictures, the notes and the text under each annotation (`task_brief`), and its agent is told the brief's path. Kept in `~/.ottospace/materials` | Works today |
| Workbooks (⌘⇧S): save tabs under a name (their splits, each terminal's directory, each app) and open them again from the Workbook section. Several tabs each: add a tab to one, or save all the tabs on the bar as one. Kept in `~/.ottospace/workbooks`. An app comes back as its window, not on the page or document it showed | Works today |
| Tabs and layout come back after quitting: every tab (with its task and workbook), its splits and sizes, focus, the tab on show, the section and the drawer; terminals start again in their folders with their last lines and an offered command; running apps' windows go back in their panes. See [Restoring the layout](#restoring-the-layout) | Works today |
| `settings.json`: terminal font, repository directories, tasks folder, apps to offer, window rules | Works today |
| [Meeting timeline](#meeting-timeline): a calendar feed (`calendar_ics`) as a hairline across the top of the window, now at its left edge; the next meeting named beside the tab bar's buttons; a soft pulse as one comes near; a banner, a notification and a sound when one has started and you are in no call | Works today |

### Roadmap: designed, not built

None of these is in the app today.

| Capability | State |
|---|---|
| Other key modes: insert | Designed |
| Tasks: a worktree at kickoff, a brief pane, automatic cleanup when done | Designed |
| Quick capture (⌘⌘): turn something on screen into a task and dispatch it to an agent | Designed |
| Focus blocks: hide Slack, with an agent that can break through | Designed |
| Fast app switcher | Designed |
| Workbooks: the URL, channel or document each app showed; a workbook on the rail; its tabs as a scope on the tab bar | Designed |
| TypeScript scripting, with contextual quick actions in an action deck (Zoom meeting layout, spin for stand-up) | Designed |
| Hide private panes while screen sharing | Designed |
| Session restore beyond the layout: what each app showed (URLs, channels, documents) | Designed |
| Terminals survive quitting and updating (the programs in them keep running) | Designed |

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
| ⌘E / ⌘⇧E | Pick what goes in a new pane, right of / below the focused one: a terminal or a running app. Type to filter: then every installed app that matches is offered too, under "Not running" (those of the `apps` setting first); choosing one starts it in the background. ↑/↓ or ⌃N/⌃P move, ↵ opens, Esc closes |
| ⌘W | Close the focused pane |
| ⌘⌃←/→/↑/↓ or ⌘⌃H/L/K/J | Focus the pane in that direction. Past the leftmost pane, ← goes to the open section's list (on the row it was left on), then to the left rail; with no section open, straight to the rail. Past the rightmost pane, → goes into the drawer's conversation (as ⌘L), or the accessibility inspector's rows while it has the drawer's place, then to the right rail; with no conversation, straight to the right rail. On a rail ↑/↓ move (its cell gets a pink ring), → (left rail) or ← (right rail) and ↵ open it; the other way, or Esc, goes back to the pane you came from. Only your own keys cross: a client's move stops at the panes |
| ⌘] / ⌘[ | Focus the next / previous pane |
| ⌘⌥←/→/↑/↓ or ⌘⌥H/L/K/J | Resize: move the focused pane's split that way |
| ⌘⌃= | Even out the splits of the tab |
| ⌘⌃0 | Fit the focused pane to its app's window, when the app refused the pane's size |
| ⌘⇧⌥←/→/↑/↓ or ⌘⇧⌥H/L/K/J | Swap the focused pane with its neighbour on that side |
| ⌘⇧⌥1 … ⌘⇧⌥9 | Move the focused pane to that tab on the bar. US keyboard layout only |
| Drag a pane's header | Onto another pane's edge: split there. Onto its centre: swap. Onto a tab: move there. Esc cancels. An app you are in is parked: click it to go back in |
| ⌘⌃F | Zoom the window to fill the screen; again to restore |
| ⌘⌃A | Debug: outline the focused app pane's accessibility targets (parked panes only) |
| ⌘/ | Show every key. In a small window the list scrolls: J / K or the arrows, space or Page Down / Page Up, Home / End |
| ⌘P | [Command palette](#command-palette): run any command of the menu bar by name. Type to filter, ↑/↓ or ⌃N/⌃P move, ↵ runs, Esc or ⌘P closes |
| ⌘, | Open Settings: `settings.json` in your editor, in a tab of its own (see [Settings](#settings)) |
| F18 | From any app: take the keyboard back and bring ottospace forward |
| ⌘T | New tab, with an empty pane: ⌘E picks what goes in it |
| ⌘1 … ⌘9 | Select a tab on the bar |
| ⌘⇧] / ⌘⇧[ or ⌃Tab / ⌃⇧Tab | Next / previous tab |
| ⌘⇧W | Close the tab and its panes |
| ⌘⇧S | Save the tab on show as a [workbook](#workbooks). On a tab that came from one, or was saved into one: update its tab there |
| ⌘Q | Quit. While a terminal runs something (a program in front of its shell, or a background or stopped job) it asks first, and lists them: ↵, Q or ⌘Q again quits; Esc, any other key or any other shortcut cancels and does nothing else |
| Wheel / trackpad | Scroll a terminal back through its history |
| ⇧PageUp / ⇧PageDown | Scroll a terminal a page |
| ⇧Home / ⇧End | Top / bottom of a terminal's history |
| Drag / double-click / triple-click | Select text / a word / a line in a terminal |
| ⌘C | Copy the terminal's selection (nothing selected: does nothing). In a [text field](#keys): its selection |
| ⌘V | Paste into the terminal, or into the open text field |
| ⌘N | New task, in the drawer. Type what needs doing, with [tags](#tags): `#project`, `>repo`, `@agent`, `/path`, `$skill`, `^reference`, `p1` to `p9`; a dropdown finishes the one at the cursor. ↵ adds the task and starts its agent; ⇧↵ adds it for later; Esc closes. [Its keys](#keys) |
| ⌘L | The keyboard into the drawer's conversation, or back to the workspace |
| ⌘⇧L | Close the drawer to the right rail, or open it again. The agent goes on |
| ⌘↵ | With a task's conversation open: Go to its tabs (the one shown last); with none, Claim this tab (the tab on show, if it is no task's) or a New tab of it |
| ⌘⌃1 … ⌘⌃9 | Show the nth conversation of the right rail; on the one shown, close the drawer. Not ⌥1…⌥9: ⌥ is Meta in a terminal |
| ⌘⇧N | Next task: the conversation of the first task that needs you. The tab on show stays |
| ⌃⌥⌘Space | Capture, from any app: the pane in use (a hosted app you are in, else the focused pane) is frozen as a picture to draw on, and a draft opens in the drawer on the right, with the new-task line as its title. A live app is parked first; "↩ back to …" in the bar returns to it. On the picture: drag circles, ⇧-drag draws a box, ⌥-drag a freehand line, a click places a note to type (↵ keeps it). ⌘Z and ⇧⌘Z go over the steps; a click on a shape's line selects it and ⌫ removes it. ↵ goes to the title, Esc leaves the picture; a click on another pane captures that one too. In the title ↵ adds the task and starts its agent, ⇧↵ adds it for later; × or Esc cancels, after a question if anything was captured. The workspace stays in use beside the drawer, and ⌘N goes to the draft's title. See [Capture](#capture) |
| ⌘B / ⌘⇧B | Mark the focused pane / the tab on show; on one that is marked, take the mark off (the Marks section offers it back for six seconds, and ⌘B again within them puts the same mark back). The mark is named for the app, the terminal's directory or the tab's title. It records what to open, not a window or a process: a terminal's directory as it is then, an app's bundle id, name and window title, a tab's title, task and split tree. A marked pane or tab shows ◆ and its letter |
| ⌘' then a letter | Go to the mark with that letter. The status bar shows `MARK?` while it waits; Esc, or a letter of no mark, ends the wait and nothing else happens. If the mark's pane or tab is open, its tab comes forward and the pane takes focus (an app pane is focused, not entered). Else it is opened: a terminal in the recorded directory, the app's window in a pane (the one with the recorded title, else its only window, else its main one; an app that is not running is started), or the tab with its splits. A tab opens with what can be opened: a pane whose directory is gone or whose window could not be hosted is empty and says why, and going to the tab again tries it again |
| ⌘⌥1 … ⌘⌥9 | Go to one of the first nine marks, in the order of the Marks section |
| ⌘⌃E | List mode: the bar (Tasks, Calendar while a feed is set, Apps, Workbook, Repos, Marks) takes the keyboard, on the open section, or Apps if none is. On a workbook ↵ opens its tabs, space lists them under it, `a` adds the tab on show to it, `r` renames it, `x` deletes it and `u` takes the last delete or update back; on one of its tabs ↵ opens that tab, `r` renames it and `x` takes it out. On a mark `x` removes it, `r` renames it, `m` gives it a letter, ⇧J / ⇧K move it down / up, and `u` puts the last removed mark back. `j`/`k` or arrows move; ↵ opens the row (⇧↵ splits down); in Tasks a heading is a row too, and ↵ or space on it folds or unfolds it; Tab changes section; on a task `s` snoozes one hour, `h` holds, `x` dismisses, `u` undoes; on an app that is not running `p` pins or unpins it, `x` hides a suggestion and `u` takes the last of these back; in Apps `/` types in the filter of "All apps" (↵, ↓ or Esc gives the keys back to the rows); Esc or ⌘⌃E leaves |
| ⌘; or ⌘⌃G | Sneak mode: every control ottospace draws (rail icons, bar rows, tabs and their ×, top-bar buttons, pane headers and their ×, each pane, the drawer's buttons and picks, the right rail's badges, the home screen's buttons, rows and nodes) gets a short label, home-row letters first. Type a label and the control does what a click does; a pane's label focuses it, or gives an app pane the keyboard. A × (pane or tab) always has a two-letter label. Two-letter labels narrow on the first letter; ⌫ takes it back. Esc, a click, a letter that goes on with no label, or any ⌘ shortcut leaves; the shortcut then runs (⌘C copies, ⌘V pastes). Other keys do nothing. The labels follow what is drawn: if a control moves its label moves, and if it goes its label goes. What happens by itself (a shell exits, an agent files a task) does not end the mode. Starting it hides the accessibility overlay (⌘⌃A). Not ⌃G alone: a terminal needs that key. On a keyboard layout with no Latin letters, the keys type the letters they have under ⌘. From `APP` mode return first. The focused app pane's elements get labels too, in orange (APP mode's colour), always two letters or more so one stray key presses nothing in another app: those its accessibility tree says can be pressed (`AXPress`) or typed in (text fields), enabled, at least 4 points each way where the pane shows them, the innermost of nested ones, at most 150 (nearest the pane's top left). Their labels start with the letters ottospace's own leave free, 19 two-letter labels to each such letter (e.g. `ta`, `ts`, `td`); the first letter only narrows. Only when those letters run out do they go under a × label's first letter, or take three letters. They appear once the app's tree is read, a moment after ottospace's own, and follow it while the mode is on (read again each second); the status bar says how many, e.g. `12 controls in Calculator`. An orange label presses that element (`AXPress`) and the pane stays parked; a text field's label focuses the field and gives the app the keyboard, so you type there. Before it presses, ottospace finds the element again and checks it is the same (role, enabled, place in its window within 2 points); if not, nothing is pressed and the status bar says the control moved. Sneak sets no value, opens no menu and scrolls nothing in the app, and only your own typed letter presses: a socket client's letter of an orange label is refused. Apps that build their accessibility tree only on request (Electron, Chrome's pages, Spotify) show few elements, and get few labels: ottospace does not switch their fuller mode on. Entering an app pane raises its window, as a click on the pane does |
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

**The new-task box.** ⌘N, the Tasks section's `+` and the right rail's `+` open it in the drawer,
beside the workspace. Over it: picks for WHERE (the first listed repositories, "no repository")
and WHO (the default agent, then each runner and its models), which set the line's `>repo` and
`@agent`, and a line that says what ↵ will do: "↵ will start claude in app on main".

| Key | Action |
|---|---|
| ↵ | Takes the dropdown's highlighted row. With no row highlighted, or when the word is that name already, adds the task and starts its agent: the drawer becomes its conversation |
| ⇧↵ | Adds the task for later: the box closes, nothing starts |
| ⌘↵ | Adds the task and starts it, whatever is highlighted |
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
- **A path's rows.** Only a word that starts with `/`, `~/`, `./` or `../` (or a bare `~`)
  opens them: `and/or`, `11/14` and a link do not. Directories come first, then files; a name
  that starts with the last part typed before one that holds it, whatever the case. A hidden
  name shows once that part starts with `.`. At most eight, and "… n more" under them. While
  the next folder is read the rows shown stay; none can be taken until it is in. A name that
  hides or turns its text around (a bidi mark) is not offered. `./` and `../` need a `>repo`.
- **A folder you take** (Tab, ↵ or a click on a folder's row) is where the task's agent will
  run, in place of its `>repo`: the box shows it as "runs in ~/notes/ ×", and the line above
  says so. × takes it off and leaves the path in the line. Changed in the line, it is gone. A
  path typed out and never taken is only a word of the task.
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

## Flow of a task

1. **Write it.** ⌘N opens the drawer on a new task. ↵ adds it and starts its agent; ⇧↵ files it
   for later. With nothing open at all the drawer shows a new task by itself.
2. **It starts at once.** The agent (`@claude`, `@codex-gpt-6-astra`, …; with no `@agent`, the
   `default_agent` setting, else `claude`) runs in the drawer, in the task's repository (no
   `>repo`: the home folder), with the keyboard: Claude Code drawn natively (its structured
   mode, see step 11), Codex in a terminal (its TUI); `conversation_view: "terminal"` puts
   Claude Code in a terminal too. It is asked `Task: <title>`; a
   task with captures is told where its brief is. The workspace stays as it was: the same tab,
   panes and focus. That tab, if it is no task's and has panes, becomes the task's and gains its
   badge; on a tab of another task, or with nothing open, the task starts with no tab.
3. **One verb.** A click or ↵ on a task is its conversation: started if it never ran, its
   session gone on with if it has one and no agent runs, shown if one runs. A session is gone
   on with in the folder it started in, which may be below the repository; with the Tasks
   section closed that one session is read first. One at work in another terminal, or held
   open by another Claude Code, is not: the drawer says so. An agent you just stopped there is
   not another terminal's: it is gone on with at once. Going on with a session, the task runs.
   A saved agent cannot be started yet: the drawer says so too.
4. **The drawer.** Its header: the task's badge, title, model and time; Go to its tabs (⌘↵)
   shows the one of them shown last; a task with none offers Claim this tab (the tab on show is
   no task's) or New tab (a shell in its repository); Brief (when it has captures); Stop ends the agent;
   × (⌘⇧L) closes the drawer to the right rail and the agent goes on. ⌘L moves the keyboard in
   and out. Drag its left edge to make it wider or narrower. Opening a conversation while a new
   task is half written puts the line aside: the box gets it back when it opens again.
5. **The right rail.** The task's badge per open conversation: its border in the task's own
   colour, its state as the corner dot; hover for its card. A click shows it, and the tab stays;
   on the one shown, closes the drawer. A dismissed task leaves the rail and the drawer (the
   drawer shows nothing, not another conversation); a done one stays only while it is shown or its
   agent runs. Dismissing a task whose agent runs asks first: Stop its agent and dismiss?
6. **Tabs and the drawer.** Selecting a tab of a task shows that task's conversation in the
   open drawer, starting nothing and leaving the keyboard where it was; a closed drawer stays
   closed, and a tab of no task leaves it as it is. ⌘T makes a tab in the group on show; New Tab
   outside the Task, one of no task. Drag a tab onto another group's tab or badge to move it
   there, onto a tab of no task to unlink it; Remove Tab from Task does that from the palette,
   and Move Tab to Task… lists the tasks (and No task) to move the tab on show to.
7. **Following.** The task runs while its agent works. Claude is given the session's id; Codex
   names its own, and ottospace finds it where Codex wrote it: the one session that started in
   the task's folder after the agent did and before the next agent there, that nothing else
   has. When two could be its (another Codex started there at the same moment), neither is
   taken: the task runs, following none. When the agent ends while the task runs, the task needs you.
8. **A session that is no task's.** A click or ↵ on its row under Other sessions opens its
   conversation the same way, with no task: it gets a badge on the rail (from its title, else
   its repository) and Adopt in the drawer's header. Adopt, there or on the row, makes the task,
   and the conversation is the task's: the same agent if it runs, else gone on with. Adopt opens
   no terminal tab.
9. **Who may.** Only you start or stop an agent, and only you say which task a tab is of: a
   client's ↵ files the task with no tab, its `task_open` goes to tabs the task has, its
   `session_adopt` makes the task, and none starts anything.
10. **Progress.** While the task is open its session is read on: what it does now and its model
   show on its row, and the task keeps in its file when its agent started, when it was done, its
   time at work (activity until a quiet gap of two minutes) and its tokens by model (input,
   output, cached; a sub-agent's apart), summed over its sessions. They stay when the session
   files go. Hover the task, or its badge, for its card: state and what it does now, tags,
   Started, Agent time, Workspace (tabs · panes), the tokens and how full its context is. The
   Tasks section groups by state (Needs you, Running, Later, Done) or by project (State |
   Project in its header), and says under the list what was done today, what runs, and how
   long agents worked today. A key of a task's file this build does not know is kept as it is.
11. **The native conversation.** The reply is drawn as it streams: markdown, code blocks (a
   Copy on each), tool calls as rows (a click shows what the call does, as plain text), a
   working row with the call it waits on and the time. A call that edits a file (Claude `Edit`,
   `MultiEdit`, `Write`, `NotebookEdit`; Codex's patch) has a diff card under it: the path
   (relative to the repository; a click reveals it in Finder if it is inside), removed and added
   lines in red and green, plain text, at most 40 lines (`… n more lines` for the rest). Hover a message for Copy and Copy as
   Markdown. Links show their address on hover and open only on your click: `http`/`https`
   in the browser, a file inside the task's repository revealed in Finder; anything else is
   text. Nothing is fetched: an image on the web is a link, a local image in the repository a
   thumbnail. The composer at the bottom: ↵ sends, ⇧↵ starts a new line, Esc leaves; while a
   turn runs ↵ sends nothing (write the next message meanwhile), Esc Esc or Stop ends the turn
   and the session goes on. A permission request is a card in place of the composer: the
   request as the CLI asked it, a labelled line per field, and the agent's own words apart.
   `a` allows this one request, `d` denies it, `m` writes a message for the Deny (↵ denies with
   it), `s` shows all of a request cut at 4 KB, which must be seen before `a` works. ↵ does
   nothing on a card. No "always allow": your Claude Code permission mode (shown in the header)
   decides how often cards come. `Chat | Terminal` in the header ends the process and goes on
   with the session in the other view once it is gone; never two at once. Not while a turn runs
   or a card waits in Chat (Stop first); from the terminal at any time, as Stop. A session at
   work in another terminal is watched: a read-only list of what it calls, with its edits' diff
   cards, a message at a time as its file grows (the header says "Watching · updates per
   message"; nothing streams); Take over, once it is idle, goes on with it here. Only you send, answer, stop a turn or switch.

## Command palette

⌘P, or View ▸ Command Palette…, opens a box with every command of the menu bar that needs no
argument: its group (Panes, Tabs, Tasks, Workbook, Marks, View, Developer, App), its name and its
key. Type to filter: the letters in order, ignoring case and spaces (`tr` finds Terminal Right);
a word's first letter and letters side by side rank first, then a group's name (`dev` lists the
Developer group). ↑/↓ or ⌃N/⌃P move, a click or ↵ runs the command as its menu item does, Esc, ⌘P
or a click outside closes it. A command that nothing matches keeps the box open.

Open Settings (App, also ottospace ▸ Open Settings, ⌘,) opens `settings.json` in your editor in a
tab titled Settings; see [Settings](#settings).

Move Tab to Task… (Tabs, also File ▸ Move Tab to Task…) turns the box into a list of the tasks
not dismissed, in their order, then No task. Type to filter by title; ↵ or a click moves the
tab on show to the end of that task's group, or out of any task.

The Developer group, also the Developer menu: Accessibility Overlay (⌘⌃A), [Accessibility
Inspector](#accessibility-inspector), Copy State as JSON
(the reply of `{"cmd":"state"}`, to the clipboard), Open the Settings File and Open the ottospace
Folder (in the Finder; a test run that named no folder has none), and Bring All Windows Back
(see [Getting a window back](#getting-a-window-back)).

The palette is the user's: a socket client cannot open it, type in it or choose (its keys are
refused while it is open), and pane requests wait behind it as they do behind the picker. ⌘P is a
key of ottospace's, so it does nothing while a hosted app has the keyboard (`APP` mode). Not ⌘K:
in a terminal it clears the screen, and is kept for that.

The Tasks group also holds the drawer's commands: Type in the Conversation (⌘L), Open or Close the
Conversation (⌘⇧L), Go to Its Tabs (⌘↵) and Stop the Agent (the drawer's Stop), each for
the conversation the drawer shows. ⌘P while you type in the conversation opens the palette over
it: the agent gets no key while it is open, and when it closes the keyboard goes back to it.

## Accessibility inspector

Developer ▸ Accessibility Inspector, or "inspector" in the [command palette](#command-palette),
with an app pane focused: that app's accessibility tree, as ottospace reads it, in the right
drawer's place. The drawer (a conversation, the new-task box, a draft, a brief) is covered, not
closed, and comes back as it was when the inspector closes; the right rail stays. Showing
something in the drawer (⌘N, a conversation from the rail, a capture) closes the inspector.
Again, ×, or closing the pane closes it. "Filter the Accessibility Inspector" and "Read the
Inspected App Again" are in the palette and the Developer menu too.

- The header names the app and counts what it exposes: "Spotify: 3 nodes, 0 pressable", then
  the most common roles. An app that exposes fewer than 25 nodes says so: Electron and Chromium
  apps (Spotify, Slack) build their full tree only when an assistive app asks, and ottospace
  does not switch that on. That is why sneak mode and the overlay find almost nothing in them.
- Each row: the role, then the title, label and value (cleaned of characters that do not show,
  60 characters at most), and `press`, `set` or `focus` when the node presses like a button, its
  value can be set, or it has the keyboard. ▸ / ▾ folds a node.
- A click on a row selects it: its frame is outlined on the pane (a parked pane: a live window
  covers ottospace), and the strip at the bottom shows its role, subrole, identifier, frame and
  its actions' names.
- The filter (click it) keeps the rows whose role or text holds what is typed, and those above
  them. While it has the keyboard (`INSPECT`), ↑/↓ select, ↵ or Esc gives the keyboard back.
- Read again every 2 seconds while it shows (Refresh reads at once), off the main thread, within
  30 levels, 2000 nodes and 750 ms, each call to the app 0.5 s at most. The header says when a
  read stopped at one of them.

Read-only: it presses nothing and sets nothing in the app. The tree holds the app's text (names,
messages): it is shown on screen only. The state snapshot has the counts, the roles and each
row's flags (no rows while a filter is typed: which match would tell the text), never a title,
label or value, and a socket client cannot open the inspector or move
in it.

## Tags

One line in the new-task box (⌘N), or in `task_new`, becomes a task. Words with a tag mark are
read out of the line; the rest is the title.

| Tag | Means | The dropdown offers |
|---|---|---|
| `#project` | The project the task serves | Projects other tasks name |
| `>repo` | The repository the task works in | The Repos list (repositories and worktrees) |
| `@agent` | Who works it: a runner (`@claude`, `@codex`: its default model), a runner and a model (`@claude-opus-5-5`, `@codex-gpt-6-astra`), or a saved agent | `claude`, its models, `codex`, its models, then each `~/.ottospace/agents/<name>.md` if that folder exists |
| `/path` | A path the task is about: a word that starts with `/`, `~/` (or is `~`), `./` or `../`. It stays in the title. A folder taken from the dropdown is where the agent runs | The directories, then the files, of the folder typed |
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
- **A `/path`** is under the home folder (`~/notes.md`), from the root of the disk
  (`/Users/me/notes.md`), or, when the line names a `>repo`, inside that repository:
  `./src/export` with `>canvas-app` is `src/export` in it, `../other` beside it. It counts if a
  file or directory is there. The task keeps the word as typed (`paths`) and, beside it, the
  absolute path that was found (`resolved`). The word also stays in the title, so the agent
  reads it.
- **Punctuation and spaces.** A comma or full stop after a tag is not part of it: `#canvas,`
  and `p1,` work. For a path the word as typed is tried first, so a file named `v1.` is found,
  then the word without the punctuation. A space in a path or a reference is written with a
  backslash before it, `~/My\ Notes`, and so is any other character that would end the word (a
  no-break space, as in a macOS screenshot's name) and a backslash itself (`a\\b`); taking a
  row, a drop or ⌥-click writes them for you. The task keeps the name without them. A name
  with a line break or a tab in it cannot be in the line: it is left out, and the box says so.
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
  a task's agent runs in its repository, or in the folder taken from a path's dropdown.
  `@agent` says which agent starts in the drawer ([Flow of a task](#flow-of-a-task)). A
  `/path` is in the title the agent reads, and stored on the task; `$skill` is stored and
  shown on its row; nothing loads a skill yet. What must be checked before anything does:
  [routines.md](ai/routines.md#6-trust-and-safety).
- **Reading is off the window's thread.** Names and paths are looked up on another thread, so a
  slow disk does not stop typing. A tag's chip shows a moment after the word is typed; ↵ files
  the line with the tags that have shown.
- `OTTOSPACE_SKILLS` (directories, `:` between them) replaces the runners' skill folders.
- `OTTOSPACE_PATH_HOME` is the folder `~` stands for in a `/path`. A test run (`OTTOSPACE_E2E`)
  that names none has no `~`, and looks at no path outside the temp folder.
- `@name` named a repository before. It does not now. Titles of tasks already filed are not read
  again.

## Capture

The capture key turns what a pane shows into material for a task. Each capture keeps the picture
as taken, the picture with your annotations drawn on it, the annotations themselves, what the pane
was (an app's name, bundle id and window title; a terminal's directory and the program in front),
and the text under each annotation: for an app pane the labelled accessibility nodes there (read
once, read-only: nothing in the app is pressed or set), for a terminal the rows.

- The task's row says how many materials it has. Opening the task shows its brief in the drawer; so does Brief in its conversation's header.
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
The Tasks section lists, under **Other sessions**, the sessions of your listed repositories
(worktrees included) that no task has adopted: for example a Claude Code you started by hand in a
terminal. The group is folded at first: its heading shows the count, and a click, ↵ or space
unfolds it. A session at work does not unfold it.

- **What a row shows.** The runner (`claude` or `codex`), the repository or worktree, a dot that
  is lit while the session is active (its file changed in the last 30 seconds), what it does now
  (`shell · cargo test`, `network · git push`, `files · src/main.rs`), and when it was last
  active.
- **Range.** Unfolded, its heading has a switch, Today | 7 days | 30 days | All (kept in the settings as
  `sessions_range`): Today lists the sessions active since the local day began, the others
  those active within that many days, All every one listed. A session at work is always
  listed. The heading counts those in range; hover it for how many there are in all. The
  range only narrows what was read: at most 200 sessions are listed, the most recent, so All
  shows those 200.
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
- **Open** (a click on the row, or ↵ in list mode) shows its conversation in the drawer, and the
  workspace stays as it is. No task is made. An idle session is gone on with there: `claude
  --resume <id>` or `codex resume <id>`, in the folder the session started in, which must be in
  a listed repository or worktree. One at work (its dot lit) or held by a live Claude Code
  process is not: the drawer says it is live in another terminal, and nothing is started. When
  the row says "in pane N", that pane is focused too.
- **Adopt** makes a task of the session: named after the repository, with `>repo` set to that
  repository (wherever in it the session started) and `@agent` to its runner. The session is
  that task's from then on and leaves the group. Rename the task later. The task keeps the
  session's runner and id, and no path from the session's file. Its conversation opens in the
  drawer as the task's (see [Flow of a task](#flow-of-a-task)).
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
- **"In pane N"** shows only when there is no doubt: the session is at work, exactly one
  terminal on the tab on show was opened in its directory, and no other listed session at work
  started there. A client's click on a row (`session_click`) focuses that pane, else selects the
  row: it never opens a conversation.
- **Fresh or not shown.** The list is emptied when the Tasks section closes and read again when
  it opens, so nothing shows what a session did a while ago. If the reader stops, the status
  bar says so once.
- **Read, then gone on with only by you.** ottospace reads the runners' own session files, every
  5 seconds while the Tasks section is open and never while it is closed. It does not attach to
  a session, type into it or signal it. Only Open or Adopt, by you, starts the runner's own
  resume command on it. Only derived facts are shown: names from fixed tables, counts and
  times. Never a prompt, a file's contents or a command line. The one free text is the
  session's title (above). Design: [sessions.md](ai/sessions.md).
- Turn it off with `"sessions": false`. See [Settings](#settings) for where sessions are read from.

## Workbooks

A workbook is a saved, named set of tabs for a context that comes back, e.g. "Maestro DRI": a
tab of dashboards, a tab of docs. It is not a task: no agent works it and it is never done.

- **Save**: ⌘⇧S, File ▸ Save Tab as Workbook, or + in the Workbook section's header. It records
  the tab's splits and their sizes, and each app pane's app (bundle id and name); for each
  terminal, its directory as it is then, its last 500 lines (`workbook_scrollback_lines`; `0`
  keeps none; only once you typed in it: a shell left at its prompt keeps none) and the command line of the program in front of its shell, if one runs. No window
  title, URL or document. A tab with nothing in it is not saved.
- **Add a tab**: `Add tab` on a workbook's row, `a` on it in list mode, or File ▸ Add Tab to
  Workbook… (it opens the section in list mode: pick the row, `a`). **All tabs**: File ▸ Save All
  Tabs as Workbook saves every tab on the bar that has something in it, in bar order, as one new
  workbook (at most 32; with more it saves none and says so). A tab stands for one workbook's tab at most: added to another, it leaves the first.
- A new workbook is named for the tab (all tabs: "Workbook"), and the section opens with the
  name selected: type over it, ↵ keeps it, Esc keeps it as it is. No two workbooks share a name
  (`DRI`, `DRI 2`).
- **Save again** on a tab that came from a workbook, or was saved into one, updates that tab of
  it. The section says `Updated · Undo` for six seconds.
- **Open**: click its row, or ⌘⌃E and ↵. Each of its tabs opens in a new tab, in order, and the
  first is selected; a tab of it that is open already is kept, not opened twice. A workbook of
  one tab opens under the workbook's name; of several, each tab under its own title. Each
  terminal starts a new shell in its directory; if the directory is gone it starts in your home
  folder, and the status bar says so. Above its prompt, in grey, are the lines it kept, then
  `── saved <date> ──`. A command it kept is typed on the prompt, not run: ↵ runs it, or edit or
  clear it first. The divider names it too (`↵ runs: npm run dev`). The program itself did not
  survive: this is not session restore. Each app pane gets a window of its app as a tab's mark does: one no
  pane holds (the only one, else the main one); an app that is not running is started. A pane
  whose app is not installed, or whose window is in another pane, stays empty and says why. A
  dot marks the row while a tab of it is open, and the tab bar marks its tabs with the workbook
  icon (hover: its name).
- **Its tabs**: ▸ before a workbook of several tabs (space in list mode) lists them under it.
  Each opens alone (click, ↵), and has `Rename` (its title, `r`) and `Remove` (`x`: out of the
  workbook; the last one deletes the workbook).
- **Rename, Delete**: the words on its row (hover it, or select it in list mode), or `r` and
  `x` in list mode. Deleting leaves the open tabs alone; `Workbook deleted · Undo` shows for six
  seconds (`u` in list mode).
- Kept lines and commands can hold secrets a tool printed, or a token in a command line. They
  stay in your workbook files, plain text with escape sequences taken out, at most 64 KB a
  terminal and 1 MB a workbook. Credential-shaped text is hidden as `[hidden]` (tokens, `key=`
  passwords, `user:pass@` URLs, `Authorization:` headers; a private key block is left out), and
  a command holding any is not kept. This is best effort: a secret of another shape is kept as
  printed. Set `workbook_scrollback_lines` to `0` and no terminal text is saved at all. The save
  says what it kept in the status bar, and the row says `history` while a workbook holds any;
  deleting the workbook deletes its file. Never in the control socket's state (it counts them:
  `lines`, `has_command`).
- Kept in `~/.ottospace/workbooks`, one file each, readable by you alone. A file there that
  ottospace cannot read is counted in the status bar and left as it is.
- Only you save, open, rename or delete a workbook: a socket client's key, click or label is
  refused. The state lists them (`workbooks`: names, tabs and pane kinds, no directories).

Not built: bringing an app back on the URL, channel or document it showed, and a workbook's
tabs as a scope of their own on the tab bar. Design: [workbook.md](ai/workbook.md).

## Restoring the layout

Quit ottospace and start it again: the workspace comes back. `~/.ottospace/layout.json` holds it,
written a second after a change and at quit, readable by you alone.

- **What comes back**: every tab in bar order, each with its task (a task's tabs together, tabs of
  no task last) and the workbook it came from; its splits and their sizes; which pane had focus;
  the tab on show; the section that was open; the drawer (the conversation it showed, open or
  closed, its width). A task's tabs live here, keyed by the task's number, not in the task's file;
  its badge colour is in the task's file. A tab of a task that is gone or dismissed comes back as
  a tab of no task.
- **Terminals** start a new shell in the folder they were in (one that is gone: your home folder,
  and the status bar says so). As a [workbook](#workbooks)'s do, they show their last lines in
  grey above the prompt, and a program that ran in front is typed on the prompt, not run: ↵ runs
  it. The same setting (`workbook_scrollback_lines`; `0` keeps no terminal text) and the same
  cleaning apply: escape sequences taken out, credential-shaped text hidden, capped. The program
  itself did not survive. The grey lines are kept once: a start in which you typed nothing there
  keeps them as they were; one in which you did adds what it printed below a second divider.
- **App panes** keep the app (bundle id and name), never a window title. A running app's window
  goes back in its pane; an app that is not running is not started: its pane says so and offers
  `Open <App>`. `restore_apps: true` starts it instead. Opening a workbook starts its apps: you
  asked for them; a start does not.
- **The drawer** shows the task's conversation again, not running: a click on its task goes on
  with its session, as always. No agent starts by itself.
- A tab whose task no longer exists comes back as a tab of no task, so nothing is lost.
- A file that cannot be read is set aside as `layout.json.bad`, and ottospace starts empty and
  says so. A file a newer build wrote is left as it is and not written over.
- **File ▸ Start Fresh** (or the palette): the next start is empty. The file (and a
  `layout.json.bad`) is removed and not written again this run. `restore_layout: false` turns it all off: the file is neither read nor
  written (one that is there is left alone).
- Marks that stood for a tab or a pane stand for it again.

## Meeting timeline

Set `calendar_ics` to your calendar's feed (an `https://` link, such as a calendar's "secret
address in iCal format", or a file's path) and ottospace shows your day's meetings, never in
the way.

- **The hairline**: a 6-point band along the very top of the window, above the window's buttons,
  so it costs no room. Its left edge is now; it shows the next `calendar_window_hours` (8). Each
  meeting is a block as long as it lasts; blocks slide left as time passes and go when they end.
  A meeting that runs starts at the left edge and shrinks. Faint ticks mark whole hours; the hour
  shows on hover. Meetings that overlap share two half-height lanes. All-day events are one small
  dot at the left end. Hover a block for its title, time and place.
- **The chip**: beside the new-tab and split buttons, the next meeting today: `Design review ·
  in 12m`, `· now` while it runs. Hidden when nothing is left today. Its tooltip says when the
  feed was last read (`calendar: not updated since 09:40` when the last read failed).
- **The pulse**: from 10 minutes before a meeting its block and the chip glow softly, slowly;
  from 2 minutes, faster; while it runs, steadily. With Reduce motion (System Settings,
  Accessibility, Display) the colour steps up instead, with no movement. Nothing else in
  ottospace pulses.
- **The status bar** says `Design review in 4m` in amber from 5 minutes before.
- **At the start**, if you are in no call, or still in an earlier one (on from 10 minutes before,
  or the meeting before, running over): a banner under the title strip (the title, how long
  ago it started, **Join** if the meeting has an `https:` link, **Dismiss**), which stays until
  you dismiss it, join, a call starts or the meeting ends; and a system notification with the
  alert sound, so you see it when ottospace is behind other windows. Once more 2 minutes later
  if you have not joined; never a third time. Only in a meeting's first 10 minutes: one you
  leave early alerts nothing. `calendar_notify: false` turns alerts off; `calendar_notify_for:
  "links"` alerts only meetings with a Zoom, Meet or Teams link (the default, `all`, alerts every
  meeting, as one with no link is still one you would miss).
- **In a call** means a microphone is in use by any app (Zoom, Meet in a browser, Teams), as
  macOS's audio system says. ottospace reads no window, title or log for it and asks for no
  permission. A microphone in use for something else (dictation, a recording) also counts as a
  call, and then no alert comes, unless it is an earlier call (above). Until
  it has looked (it looks every 2 seconds from a minute before a meeting, and while a call it saw
  goes on) no alert comes either. Leaving one call and joining the next within 2 seconds reads as
  one call: Dismiss the alert.
- **Join** opens the meeting's link with the app that handles it (Zoom for a Zoom link), only
  when you click it or choose it, and only an `https:` link. Never by itself.
- **The feed** is read at start and every `calendar_refresh_minutes` (5), off the main thread,
  through `/usr/bin/curl`: `https:` only, no redirects (a feed that has moved is not read: the
  status bar asks for its final address), only from a host on the internet (looked up once,
  checked, and curl pinned to that address), at most 5 MB, 30 seconds. A proxy set in your
  environment (`https_proxy`) applies. It is ottospace's only network access. The link is handed to curl on its standard input, not in
  its arguments, and is never shown, logged or put in the state. A read that fails keeps the
  events of the last one. Recurring meetings, time zones, cancelled meetings and moved
  occurrences are followed; see [meeting-timeline.md](ai/meeting-timeline.md) for what is not.
- **Declined meetings**: set `calendar_email` to your address as the feed lists you among a
  meeting's attendees, and a meeting (or one occurrence) you declined is left out everywhere: the
  hairline, the chip, the section and the alerts. A feed published with limited detail (free/busy
  only, or without attendees) does not say who declined; then nothing is left out. The address
  stays in `settings.json`: it is not sent with the feed's request, shown, logged or in the state.
- **The Calendar section** (its rail cell, second, is there only while `calendar_ics` is set):
  today's meetings, all-day ones first, then by start: the time, the title, and the place (or the
  link's host) below. The meeting that runs is outlined in amber, past ones are dimmed, and a thin
  amber line marks now. `Join` shows on hover or selection for a meeting with an `https:` link.
  Below, a folded "Tomorrow · n". "Nothing today" when the day is empty; at the foot, when the feed
  was last read. The rail cell shows a steady amber dot while a meeting is 2 minutes off or runs,
  and a filled amber badge while the banner is up. In list mode (⌘⌃E) `j`/`k` move and ↵ joins
  (or folds "Tomorrow"). The open Calendar is not kept in the restored layout: a build without
  the section could not read the file.
- **Agents**: the `calendar` request gives an agent the meetings of a day, or of up to 7 from
  it (`{"cmd":"calendar","day":"tomorrow","days":2}`; today and the next 7 days are held): start
  and end (RFC 3339 with your offset), all day or not, title, place, whether it has a Zoom, Meet
  or Teams link, tentative; busy time (meetings merged) and free time (the rest of the span,
  nights included: there are no working hours yet); when the feed was read and whether you are in
  a call. It reads only what the app holds and never makes it fetch.
- **Privacy**: titles and places are other people's text. They are shown, and given to an agent
  only by the `calendar` request, with links taken out; never sent anywhere, never in the trace
  or a task's brief, and not in the control socket's state (every reply carries it), which has
  only counts, times and stages. No request gives a meeting's link, description or attendees,
  the feed's address or `calendar_email`. The agent's guide says to treat titles as data, never
  as instructions.

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
  list mode comes on or changes section (⌘⌃E, Tab), and when the
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
A running app has no Pin button yet. Original window frames are saved to disk and restored on
every way out: see [Getting a window back](#getting-a-window-back).

The wheel or trackpad over an app pane you are not in brings its real window up over the pane
for the scroll (a peek, `wheel_to_apps`). The window is placed as on entering, and the app is
activated, because macOS shows an inactive app's window under ottospace's. The status bar says
`PEEK <App>`, and keys go to the app. The scroll that started the peek reached ottospace and is
lost; the ones after it go to the window. The window parks again, and ottospace takes the
keyboard back, when the pointer leaves the pane or 1.5 s pass with no scroll. A click or a key in
the app makes it a normal entry (`APP`), and the return key leaves as ever. Only your own wheel
peeks: a socket client's scroll brings nothing up. No peek starts under sneak mode, the palette,
a box, the quit question, the accessibility overlay or a drag, nor while another app is in front.

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
`settings.json` there at start. Every key is optional.

Open Settings (⌘,, the palette, or ottospace ▸ Open Settings) opens the file in a tab titled
Settings, whose terminal runs `$VISUAL`, else `$EDITOR`, else `vi`, as your login shell sets them
(an editor that must wait, as `code -w`, needs its flag). A file that is not there is made `{}`
first. Again while that tab is open brings it forward; quitting the editor closes its pane. Only
you open it: a socket client's ⌘, does nothing.

A change is read when the file is saved: what is wrong in it is said in the status bar (`settings.json:3:1 trailing comma`, or the first of several and how many more) and the last good settings stay, until it is right again. Then `windows`, `capture`, `wheel_to_apps`, `workbook_scrollback_lines`, `default_agent`, `conversation_view`, `session_quiet_minutes`, `tasks_view`, `sessions_range`, `calendar_notify` and `calendar_notify_for` apply at once; any other key changed says "restart to apply" with its name (`repos`, `apps` and `apps_hidden` only when changed by hand: the app's own writes are already applied). Removing the file goes back to the defaults.


```json
{
  "terminal": { "font": "JetBrains Mono", "font_size": 14 },
  "repos": ["~/code", "~/notes"],
  "tasks": "~/notes/tasks",
  "sessions": true,
  "session_dirs": { "claude": ["~/.claude"], "codex": "~/.codex" },
  "session_quiet_minutes": 3,
  "agent_models": { "claude": ["claude-opus-5-5"], "codex": ["gpt-6-astra"] },
  "default_agent": "claude",
  "conversation_view": "native",
  "capture_key": "ctrl-alt-cmd-space",
  "capture": { "keep_images": true, "ax": true },
  "workbook_scrollback_lines": 500,
  "restore_layout": true,
  "restore_apps": false,
  "apps": ["Microsoft Outlook", "Slack"],
  "apps_hidden": ["Music"],
  "suggest_dock_apps": true,
  "wheel_to_apps": true,
  "windows": [{ "app": "us.zoom.xos", "title": "Zoom Meeting", "do": "ignore" }]
}
```

| Key | Meaning | Default |
|---|---|---|
| `terminal.font` | The terminal's font, by family name. Use a monospaced one | `Menlo` |
| `terminal.font_size` | In points, 6 to 72 | `13` |
| `repos` | Directories that are repositories or hold them, for the Repos section. `OTTOSPACE_REPOS` (`:` between directories) wins over it. The section's + adds the folder you pick (a repository, or one that holds some), by its full path; if the file named none, `~/code` and `~/notes` are written with it. Remove, on a repository's name under the pointer, takes off a directory listed as itself; one that comes from a folder holding several is yours to edit. At most 64 through + | `~/code`, `~/notes` |
| `tasks` | Where tasks are kept, one JSON file each. A relative path is inside `~/.ottospace` | `tasks` |
| `sessions` | List agent sessions that are no task's under Other sessions in the Tasks section. `false`: no session file is ever read | `true` |
| `session_dirs` | Where the runners keep their sessions: `claude` is a list of Claude Code config directories (one for each account), `codex` is Codex's home. A runner left out keeps its default. An app started from the Dock does not see `CLAUDE_CONFIG_DIR` or `CODEX_HOME`: name other directories here | `~/.claude`, `~/.codex` |
| `session_quiet_minutes` | A task adopted from a session moves to Needs you once the session has been quiet this many minutes. `0`: a quiet session never moves its task | `3` |
| `agent_models` | The models the `@agent` dropdown offers after each runner, in place of the built-in list: ids as the runner's CLI takes them. `claude` and `codex` are each a list; a runner left out keeps its own, an empty list offers the runner alone. An id that does not start with the runner's name gets it in the tag: `gpt-6-astra` is `@codex-gpt-6-astra`, `opus` is `@claude-opus`. An entry that is not a name (letters and digits, with `.`, `-` or `_` between them one at a time; at most 64) is left out and named in the status bar | `claude-fable-5-2`, `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`; `gpt-6-astra` |
| `default_agent` | The agent a task with no `@agent` starts: a runner (`claude`, `codex`) or a runner and a model (`claude-opus-5-5`, `codex-gpt-6-astra`). The first WHO pick in the drawer | `claude` |
| `conversation_view` | How the drawer shows a Claude Code conversation: `native` (drawn by ottospace: markdown, cards for permission requests, a composer) or `terminal` (the CLI's own TUI). The header's `Chat \| Terminal` switches one conversation, and the layout keeps the pick. Codex always shows in a terminal for now | `native` |
| `workbook_scrollback_lines` | How many of a terminal's last lines a [workbook](#workbooks), and the [restored layout](#restoring-the-layout), keeps, shown again above its prompt when it opens. At most `2000`. `0`: none is kept | `500` |
| `restore_layout` | [Restore the layout](#restoring-the-layout) at start and keep it in `layout.json`. `false`: every start is empty | `true` |
| `restore_apps` | At a restore, start an app a pane names that is not running. `false`: its pane waits and offers `Open <App>` | `false` |
| `tasks_view` | How the Tasks section groups: `state` or `project`. Written when you pick one in the section's header | `state` |
| `sessions_range` | Which sessions Other sessions lists: `today`, `week` (7 days), `month` (30 days) or `all`. Written when you pick one on its heading | `today` |
| `capture_key` | The key that captures the pane in use, from any app: modifiers (`ctrl`, `alt`, `cmd`, `shift`) and a key, with `-` between them. `"off"`: no key. `OTTOSPACE_CAPTURE_KEY` wins over it. Not `ctrl-alt-space`: macOS uses it to change the input source | `ctrl-alt-cmd-space` |
| `capture.keep_images` | Keep a capture's pictures on disk. `false`: no picture is ever written | `true` |
| `capture.ax` | Read an app pane's accessibility tree with its picture. `false`: it is never read for a capture | `true` |
| `windows` | Rules for windows ottospace must leave alone. Each rule selects windows by `app` (the app's name or bundle id), `title` and `subrole`; every field given must match, `*` stands for any text, and case does not matter. `"do": "ignore"`: the window never becomes a pane, when its app is hosted or when it opens later. `"do": "manage"` undoes a built-in rule. Your rules come first. Name an app by its bundle id to be sure. A window is judged once, when it is first seen, on the title it has then; a changed rule applies to windows seen after it | Built in: only standard windows are ever put in a pane, and Zoom's sharing controls, annotation layer and pop-ups are ignored |
| `apps` | The pinned apps: listed first under "Not running" in Apps, and in the picker before anything is typed; choosing one starts it in the background and puts its window in a pane. Each is the name of the app as Finder shows it (`Visual Studio Code` for `Visual Studio Code.app`), looked for in `/Applications`, `~/Applications` and `/System/Applications` (and their `Utilities`), or the path of its bundle (`~/Tools/Lumen.app`). One that is not found is left out of the list and kept in the file. Pin and Unpin in the Apps section write this list: the name if it means that same bundle when read back (the first of that name in those folders, in that order), else its path. A name that is found only once ottospace runs (the app was installed since) is listed from the next scan. (Once you type in the picker it offers every installed app in those folders, these first) | none |
| `apps_hidden` | Apps of the Dock not to suggest in Apps, written as `apps` is. Hide in the Apps section writes this list | none |
| `wheel_to_apps` | The wheel or trackpad over an app pane you are not in brings its window up for the scroll (a peek). `false`: the wheel there does nothing | `true` |
| `calendar_ics` | The [meeting timeline](#meeting-timeline)'s calendar feed: an `https://` link (to a host on the internet, not an address or `localhost`) or a file's path (`/…` or `~/…`). Never shown: a private feed's link holds a secret | none: no timeline |
| `calendar_window_hours` | How many hours the hairline shows from now, 1 to 24 | `8` |
| `calendar_refresh_minutes` | How often the feed is read again, at least 1 | `5` |
| `calendar_notify` | A meeting that starts while you are in no call, or in another call, shows the banner, a notification and a sound | `true` |
| `calendar_notify_for` | Which meetings alert: `all`, or only those with a Zoom, Meet or Teams `links` | `all` |
| `calendar_email` | Your address among a meeting's attendees in the feed: a meeting you declined is left out. Kept on this machine, never shown | none: nothing is left out |
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
ottospace ctl '{"cmd":"calendar","days":2}'          # today's and tomorrow's meetings, busy and free time
ottospace ctl '{"cmd":"sneak"}'                      # labels on ottospace's controls: `sneak.targets`
ottospace ctl '{"cmd":"key","keys":"a"}'             # type a label: that control acts as if clicked
ottospace ctl '{"cmd":"state"}'
ottospace ctl '{"cmd":"trace"}'                      # the last 2,000 events: names, times and ids, never text
```

`OTTOSPACE_TRACE=1` also writes the trace to `~/.ottospace/trace.jsonl` when the app panics or gets
`kill -USR1`.

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
and replays the click, so input is native. The wheel over a parked pane places the window the
same way for as long as the scroll lasts (a peek). A pane shows one title bar: the stream leaves out the
app's own, and the live window's covers the pane header. Measurements and risks: [docs/ai/hosting-windows.md](ai/hosting-windows.md).

### Getting a window back

ottospace records each hosted window's original frame in `~/.ottospace/hosted.json` (pids,
window ids and frames; no titles) and puts the window back on every way out:

| How ottospace ends | What puts the window back |
|---|---|
| ⌘Q, Quit in the menu, the close button, Dock ▸ Quit, logout, `ottospace ctl '{"cmd":"quit"}'` | The quit itself |
| `kill`, ^C in the terminal that started it, that terminal closing (TERM, INT, HUP) | The same quit. A second signal ends it at once, and the guardian takes over |
| A crash, `kill -9`, a hang ended by Force Quit | The guardian: a small `ottospace guard` process the app starts, which waits for it to end and puts its windows back within about two seconds |
| None of these ran (the guardian lacked Accessibility, say) | The next start |

A window goes back to its original frame, read back to be sure. If it would not be reachable
there (at least 120×60 points of its top inside a screen's visible area: an original on a display
since unplugged, say) or the app refuses, it goes to the middle of the main screen, at its own size
or the screen's. A window found already off screen when hosted is recorded at the middle of the
main screen, and the status bar says so. If an app replaced its window while hosted, its windows
left in the parking spot come back instead. ottospace only moves and sizes windows: it never
minimises or hides them, nor moves them to another Space.

To bring back any window stranded off screen, whatever left it there: `ottospace rescue` in a
terminal (no app need run), or Developer ▸ Bring All Windows Back (also in the ⌘P palette). Each
standard window of a running app that lies wholly off every display, or in ottospace's parking
spot, moves to the middle of the main screen. Visible windows, and windows a running ottospace
hosts, are left alone. A socket client cannot run it.

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
- [sneak-apps.md](ai/sneak-apps.md): sneak labels that press controls inside the focused app pane, and why agents still cannot.
- [multi-monitor.md](ai/1-initial-brainstorm/multi-monitor.md): options for more than one display, and a staged recommendation. Nothing measured yet.
- `background/`: mockup images.

[capture.md](ai/capture.md): the capture key, annotate mode, what a capture stores, and how a brief quotes it.
[marks.md](ai/marks.md): marks, what one records, and how they sit beside session restore, pins and tasks.
[workbook.md](ai/workbook.md): workbooks beside tasks, marks and session restore; stages 1 and 2 as built, and what is later.
[session-restore.md](ai/session-restore.md): the workspace back after quitting, in layers; layer 1, the layout file, as built.
[terminal-persistence.md](ai/terminal-persistence.md): terminals and agents that outlive a quit or an update, held outside the app; with session restore, tasks come back as they were.
[task-conversation.md](ai/task-conversation.md): a task's agent in a right drawer, the three zones, how an agent is started; stage 1 as built (the agent's terminal in the drawer), and what is later.

[routines.md](ai/routines.md): routines and agents from ottobot, in stages, toward ottospace subsuming it.

[e2e-reliability.md](ai/e2e-reliability.md): replies that wait for the frame that draws them, failure folders, the trace, and repeated runs to find flaky tests.

[agent-runners.md](ai/agent-runners.md): a native (non-terminal) agent conversation: driving Claude Code and Codex in their structured modes, permission prompts, trust rules. Not wired in yet.

[meeting-timeline.md](ai/meeting-timeline.md): the meeting timeline as built, how a call is detected, the network decision, and what the calendar reader does not follow.

[`docs/ai/backlog.md`](ai/backlog.md) is an older backlog. `docs/ideas/` holds the owner's own
notes.
