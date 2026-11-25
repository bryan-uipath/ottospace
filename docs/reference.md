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
| [File viewer](#file-viewer): a file read only in a pane (markdown drawn, code coloured and text with line numbers, pictures), reloaded when it changes, vim moves with counts, ⌃G edits it beside | Works today |
| Key bindings and a status bar showing the mode where nothing else shows where keys go (`APP`, `LIST`, `SELECT`, `GO TO`, `LETTER`; else a dimmed `NORMAL`) | Works today |
| [Status bar](#status-bar): where you are, what's next, who's working | Works today |
| [Toasts](#toasts): app-level messages as cards at the window's bottom right, over the panes (they take no room), red, amber or blue by level (error, warning, information) | Works today |
| Control socket and `ottospace ctl` for agents and tests | Works today |
| Terminal scrollback, mouse selection, copy and paste | Works today |
| Scrollbars on every list that scrolls, the key reference and a terminal's scrollback: drag the thumb, click the track to page (in a terminal a click beside the thumb is the terminal's). They follow the system's "Show scroll bars" setting. A list's edge fades where more lies beyond it; tabs past the second row's width scroll across | Works today |
| Visual mode: select terminal text with vi keys | Works today |
| Tasks inbox, by state: Needs you, Running, Later (captured or on hold), Done (last, folded). Headings fold (click, or ↵ or space in list mode). Each row is one line: its badge with the state dot, its title (`P1` before an urgent one) and one fact at the right: how long it has run, waited or been done (none under Needs you: the dot says it); while its session works, a muted line under says what it does now (while its native conversation runs no call, a verb such as `Pondering…`, the same as the drawer's), a soft light sweeping across it while the task runs (never under Needs you; still under Reduce Motion). A click opens its conversation; hover it for its card (priority, state, what it does now and its model, tags and counts of paths, skills, references and captures, filed, started, agent time, workspace, tokens by model, context) and for Snooze and Done (dismiss) buttons; a done task's row has Clear alone (off the list, as `x`; only yours: a socket client's click is refused); hold is `h` in list mode. After one of those by mouse, a line at the top offers Undo for 6 seconds. Kept in `~/.ottospace/tasks` | Works today |
| Tasks by project: State \| Project in the section's header, kept in `tasks_view` in the settings. One foldable group a `#project`, by name, with a bar of its tasks' states (hover for the numbers), its open tasks (needs you, running, later), then a folded "Done · n"; the tasks of no project last. Under the list: `Today  n done · n running · agent time` | Works today |
| A task's progress, kept in its file: when its agent started and when it was done, time at work (activity until a quiet gap of two minutes), and tokens by model (input, output, cached; a sub-agent's apart), summed from its sessions, so they outlive the session files. Tokens only: no prices | Works today |
| "Pop next" (⌘J): the conversation of the next task that needs you; the tab on show stays | Works today |
| Sessions in the Tasks section: the Claude Code and Codex sessions of the listed repositories that are no task's (one you started by hand in a terminal). Under Elsewhere, each row is one line: a dot lit while it works, its title (else its repository) and its runner; hover it for its card (repository, what it does now, model, when last active, its pane). Listed: the last 7 days. A click opens its conversation in the drawer (`claude --resume`, `codex resume`), unless it is at work in another terminal; Track makes a task of one; Hide removes it until the next start | Works today |
| A task's agent in the right drawer: ↵ in the new-task box, or Start agent in the drawer, starts its agent (`claude` or `codex`, by its `@agent`, else `default_agent`) beside the workspace, which stays as it is. A click on a task only shows its conversation; Resume session goes on with one that ended. See [Flow of a task](#flow-of-a-task) | Works today |
| Native conversation for Claude Code: the drawer drives `claude -p` (stream-json) and draws the reply itself: markdown (headings, emphasis, lists with their marks on one edge, quotes, tables; inline code in monospace on one box, still code inside bold), code blocks with Copy (a `text` one wraps, no label; any other scrolls across), links that open only on your click (web pages in the browser; files of the task's repository revealed in Finder), local images of the repository as thumbnails, tool calls as compact rows that show what ran (a command in monospace with its description under it, a file relative to the repository, a search's pattern, a fetch's host, a query, a sub-agent's description with its own calls indented under it; a click opens a command whole with what it printed), what the CLI says besides as muted rows (a subtask's end, a call denied by your settings, a notification, a model fallback), a file edit's change as a diff card under its call, a working row while a turn runs (the call it waits on, else a verb that changes every few seconds, `Pondering…`; a soft light sweeps across it while the agent works, still under Reduce Motion), Copy and Copy as Markdown on each message, and a composer (↵ send, ⇧↵ or ⌥↵ new line; it wraps and grows to 8 rows, then scrolls). Permission requests are cards: Allow this one request as shown, or Deny (with a message if you like); a request too long to show is opened in full before Allow is offered. When the agent asks you (`AskUserQuestion`), a question card instead: each question with its options and an Other… row for your own words, several as tabs with a Review; ↵ answers (and goes on, or sends), Esc declines. btw (⌘⇧K, or "btw" at the composer's foot): a side question to the running agent in a column beside the conversation, answered without entering it; "Use as my answer" fills the card's Other… or the composer, unsent. `Chat \| Terminal` in the header switches to the CLI's own TUI and back on the same session. A session at work in another terminal is watched read-only (what it calls and edits, and your messages and its replies as a chat shows them; "Watching · updates per message" in the header, "Not updating" while its file cannot be read), with Take over once it is idle. Codex stays in a terminal. Text in the transcript selects as in a text field (drag, double-click a word, triple-click a line or paragraph, ⇧-click extends; ⌘A all of it, ⌘C copies, Esc clears), and Copy on hover copies a message, a reply, a code block or a call's input or output. An opened call's input wraps, JSON pretty-printed | Works today |
| New task (⌘N), in the drawer: a title line and, if you like, more lines after it (⇧↵ or ⌥↵), with [tags](#tags): `#project`, `>repo`, `@agent`, `/path`, `$skill`, `p1`–`p9`, pickers for project, repository and agent in its edge (the default agent shown from the start), and one line under it that says what ↵ will do (`↵ starts @codex in app on main · ⌘⇧↵ later · Esc`); the tags show on demand ("? tags", ⌘/). A `/path` stays in the title the agent reads, and a folder taken from its dropdown is where the agent runs; `$skill` is stored and shown, nothing acts on it yet | Works today |
| Home screen: under the house and the word Home, what goes there (Open an app ⌘E, Terminal ⌘D, New task ⌘N: each key a cap, as the keymap has it; a row's click does what its key does; a held ⌘ lights its caps), the tasks that need you, and each listed task as a node on a lattice (left to right by project; within a project the older task is farther). A click on a node or a row opens its conversation. A place with nothing in it shows one empty state, the same everywhere (see [Empty places](#empty-places)). With nothing open at all, the drawer shows a new task by itself | Works today |
| Home: the top row's first item, a house (no word, no ×). It holds no pane: what you open there (a pane, ⌘T, a Repos row) gets a new workspace, selected; ⌘N there gives the task a space of its own. Tabs of nothing (a dismissed task's, a deleted workspace's, a client's) go to the workspace selected last. Its screen is the home screen. ⌘⌃1, or Go to Home (palette, View menu) | Works today |
| Two rows of tabs: the top row holds Home, then each open task (its badge and title) and workspace (its name; holding one app pane and nothing else, that app's icon, else a small dot). Each says what goes on in it by the Repos dot's rule: a task's badge border breathes slowly while its agent works and is amber while it needs you; a workspace's dot is lit (and breathes) while a session works in its worktree. Under Reduce Motion nothing breathes: lit, and a working badge carries the working dot; under it, the selected one's tabs, when it has more than one. One selection: the drawer shows the selected task's chat. × hides a task or a workspace (nothing stops); its row opens it again. + at the row's end makes a new, empty workspace. A right click on a tab: Rename, Fold out (with `multi_monitor`), Close (as its ×); a double-click on its title, or Rename Tab (palette, File menu), renames it too: the name is typed in the tab, ↵ keeps it, Esc does not, and empty gives the automatic title back. Your name wins over the automatic one from then on and is kept between runs. A right click on a workspace on the top row: Rename (typed in its item), and Keep while it is temporary. Short of room, the tasks not selected are their badges alone; shorter still, those that do not fit are a count (+3), and the selected one always shows. Drag a tab onto a task to move it there | Works today |
| Tabs, each with its own split tree | Works today |
| Pane picker: choose a terminal, a running app, or (once you type) any installed app for a new pane or tab | Works today |
| Icon rail with a secondary bar | Works today |
| The Apps section: the Dock's apps, pins, "Not running" and "All apps" | Works today, behind the `apps_section` [flag](#features), off by default |
| Repos: each repository's name over its checkouts, one line each (a dot for what happens there: grey while idle, lit in the task's colour while its agent works there (in the working colour for a session no task has), amber while a task there needs you, one short pulse when a commit lands (none under Reduce Motion); the branch; `default` on the repository's own folder; its open pull request's mark; the badge of a task there not done). The chevron folds a repository (a count while folded, then one dot for its checkouts: amber while a task there needs you, else lit while one works, none while all are idle; no pulse; kept in `layout.json`; shown while a task there runs or needs you). Open a terminal in one. Opt-in, in your order: + in the bar's header (Local folder…, the folder picker, ↵ at once; or From GitHub…: paste `owner/name` or a GitHub link, see the full destination `<clone_dir>/<name>`, ↵ clones it with your `gh` and lists it; refused if that folder is there; it is cloned into a hidden `.ottospace-clone-<name>` beside it and moved into place only once done, never over a folder that came meanwhile; Esc stops a clone and removes that hidden folder, a click outside hides the dialog while it goes on), a dropped folder or Add Folder to Repos adds a repository to the end of `repos`; a name dragged to another place moves it there in `repos` (a pinned one among the pinned; Esc puts it back); a folder of repositories from older settings is one folded group. On a name under the pointer, + makes a worktree (`worktrees_dir`) and the trash stops listing it (asked first while a task works there; the status bar says nothing on disk changed). A right click on a checkout's row: Open in explorer ([File explorer](#file-explorer)); on a linked worktree's also Delete worktree…, after a question that names its branch and full path (⌘↵ or its red Delete worktree deletes, ↵ alone does nothing; Esc, Keep it, ×, any other key or a click outside keeps it); `git worktree remove`, never `--force`, so a worktree with changes, or a locked one, stays and the notice says why; its branch stays; no Undo. Not offered on the repository's own folder; dimmed, with why, while a terminal is in it or a task works there (not done, or its agent runs) | Works today |
| List mode (⌘⌃E): drive the bar with the keyboard | Works today |
| Sneak mode (⌘; or ⌘⌃G): a letter label on each of ottospace's own controls, and on the buttons and text fields of the focused app pane; typing it clicks the control, presses the app's button, or focuses its text field | Works today |
| Scrolling over an app pane you are not in enters it (`APP`), as a click does, and that first scroll reaches the app | Verified on CI (`e2e-apps`): the wheel enters, the window comes up under the pointer and the canary receives the first scroll |
| [Command palette](#command-palette) (⌘⇧P): every command of the menu bar by name, with its key; a Developer group | Works today |
| [Go to](#go-to) (⌘P): the files of the checkout on show, fuzzy, as you type; ⌫ for a workspace, a listed checkout or a task by name, then a file in its folder | Works today |
| [Accessibility inspector](#accessibility-inspector): a hosted app's accessibility tree in the right drawer's place, read-only | Works today |
| [Zoom out](#zoom-out) (⌘⇧\\): every tab of the workspace on show as a tile on a grid, each a live miniature; arrows select, ↵ goes in, Esc goes back | Works today, behind the `zoom_out` [flag](#features), off by default |
| Marks (⌘B, ⌘⇧B, ⌘' and a letter): named ways back to a pane or a tab, kept in `~/.ottospace/marks.json` | Works today, behind the `marks` [flag](#features), off by default |
| Capture (⌃⌥⌘Space, from any app): a frozen picture of the pane in use to circle and write on, collected in a draft in the drawer; the task it becomes has a brief with the pictures, the notes and the text under each annotation (`task_brief`), and its agent is told the brief's path. Kept in `~/.ottospace/materials` | Works today |
| Workspaces: + on the top row or in the Workspaces section makes a new, empty one, listed there; temporary (its name muted and italic) until it gets a task, a repository, a name or Keep, and then kept between runs. ⌘⇧S saves a tab into one (its splits, each terminal's directory, each app), to open again from the section. Several tabs each: add a tab to one, or save all the tabs on the bar as one. Kept in `~/.ottospace/workbooks`. An app comes back as its window, not on the page or document it showed | Works today |
| Tabs and layout come back after quitting: every tab (with its task and workspace), its splits and sizes, focus, the tab on show, the section and the drawer; terminals start again in their folders with their last lines and an offered command; running apps' windows go back in their panes. See [Restoring the layout](#restoring-the-layout) | Works today |
| `settings.json`: terminal font, repository directories, tasks folder, apps to offer, window rules | Works today |
| [Meeting timeline](#meeting-timeline): a calendar feed (`calendar_ics`) as a thin band of the workday above the status bar, a tick for now; a soft pulse as one comes near; a meeting notice on a toast, a notification and a sound when one has started and you are in no call | Works today |

### Roadmap: designed, not built

None of these is in the app today.

| Capability | State |
|---|---|
| Other key modes: insert | Designed |
| Tasks: a worktree at kickoff, a brief pane, automatic cleanup when done | Designed |
| Quick capture (⌘⌘): turn something on screen into a task and dispatch it to an agent | Designed |
| Focus blocks: hide Slack, with an agent that can break through | Designed |
| Fast app switcher | Designed |
| Workspaces: the URL, channel or document each app showed; a workspace on the rail; a workspace of a task or a worktree | Designed |
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
| ⌘E / ⌘⇧E | Pick what goes in a new pane, right of / below the focused one: a terminal, a file (File…: the open panel, then a [file viewer](#file-viewer)) or a running app. A running app lists "New window" next (only an app named below, or one with a menu item titled "New Window"), then its windows, a pop-up it floated too: each by title (windows of one title also by size, display or kind, as offers say: [App panes](#app-panes)), "in pane N" if a pane holds it (choosing that one shows its pane). A window of the list is shared: given back when its pane closes. New window opens one of the app's own (Chrome, Edge, Brave and other Chromium browsers by `--new-window`, VS Code and its forks by `-n`, Safari, Finder, Terminal and iTerm2 by an Apple Event, any other app by its "New Window" menu item); it goes to the tab it was asked from, even if you moved on; it is owned: closing its pane closes it, asking first when the app says it has unsaved changes. No window within about 3 seconds, or an ask that failed: a toast says so. Type to filter: then every installed app that matches is offered too, under "Not running" (those of the `apps` setting first); choosing one starts it in the background. ↑/↓ or ⌃N/⌃P move, ↵ opens, Esc closes |
| ⌘W | Close the focused pane. With the keyboard in the drawer's conversation: Later on its task |
| ⌘⌥←/→/↑/↓ or ⌘⌥H/L/K/J | Focus the pane in that direction. Past the leftmost pane, ← goes to the open section's list (on the row it was left on), then to the left rail; with no section open, straight to the rail. Past the rightmost pane, → goes into the drawer's conversation (as ⌘L), or the accessibility inspector's rows while it has the drawer's place. On the rail ↑/↓ move (its cell gets a pink ring), → and ↵ open it; ←, or Esc, goes back to the pane you came from. Only your own keys cross: a client's move stops at the panes |
| ⌘] / ⌘[ | Focus the next / previous pane |
| ⌘⌥⇧←/→/↑/↓ or ⌘⌥⇧H/L/K/J | Resize: move the focused pane's split that way |
| ⌘⌃= | Even out the splits of the tab |
| Drag the divider between two panes | Resize those two: every other divider stays where it is, until a pane reaches its least (a tenth of its split) and pushes the next. Double-click evens it out with its row. Esc during the drag puts it back. A hosted app's window is sized once, on release. Yours alone: a client resizes with `resize` |
| Drag a workspace's row in the Workspaces section, or any space on the top row | Move it there. Any tab but Home goes anywhere after Home; a workspace's tab and its row share one order (the top row shows the open ones in it). A task's or a session's tab of no workspace has no row: it keeps its place on the top row, the workspaces' tabs taking the other places in the list's order, and dragging past it changes no row. A new workspace's tab goes after the one before it in the list, any other new tab last. While carried it is not drawn: the others close up, and a bar shows where it lands. Release drops it, Esc or a release elsewhere puts it back; a press that moves under 4 points is a click. The first drag ends the order by name: it is yours from then on, kept in the workspaces' files, and a new one goes last. A press on a space never moves the window; the top row's empty part still does. Yours alone |
| ⌘⌃0 | Fit the focused pane to its app's window, when the app refused the pane's size |
| ⌘⌃←/→/↑/↓ or ⌘⌃H/L/K/J | Swap the focused pane with its neighbour on that side |
| ⌘⇧⌥1 … ⌘⇧⌥9 | Move the focused pane to that tab on the bar. US keyboard layout only |
| Drag a pane's header | Onto another pane's edge: split there. Onto its centre: swap. Onto a tab: move there. Esc cancels. An app you are in is parked: click it to go back in |
| ⌃⌘F | Fill the whole display, over the menu bar and the Dock; again to restore. The window's green button (Window ▸ Zoom, a title bar double-click when the system's setting is Zoom) and the expand button at the top row's right do the same: there is no zoom to the visible frame, and no native full screen, which would put ottospace on a Space of its own where apps' windows cannot follow. While ottospace is in front the bars stay away until the pointer reaches their edge. In an app pane the app's menu bar and the Dock come back, over ottospace's top row and bottom edge: nothing moves, and the app's window stops at the Dock's top. Dragging the window ends it. Each window fills its own display: a tab out's green button its window ([Displays](#displays)). On at quit, on at the next start (`layout.json`); only your own key, click, menu or palette turns it on. [Measured](ai/fill-display.md) |
| ⌘⌃A | Debug: outline the focused app pane's accessibility targets (parked panes only) |
| ⌘/ | The keys, by what you are doing, the ones you have not learned first; in list or visual mode that mode's own keys come first. A bar under a key shows how much you use it. Keys you use often fold into `Known · n` at the bottom: a click or ↵ opens it. A key unused for about eight days comes back up. In a small window the list scrolls: J / K or the arrows, space or Page Down / Page Up, Home / End |
| ⌘⇧P | [Command palette](#command-palette): run any command of the menu bar by name. Type to filter, ↑/↓ or ⌃N/⌃P move, ↵ runs, Esc or ⌘⇧P closes |
| ⌘⇧X | [File explorer](#file-explorer) of the checkout on show, beside the focused pane |
| ⌘P | [Go to](#go-to): a file of the checkout on show (↵ opens, ⌘↵ below); ⌫ for a place by name, Tab or `/` for its files; `>` for the commands |
| ⌘, | Open Settings: `settings.json` in your editor, in a tab of its own (see [Settings](#settings)) |
| ⌘⌘ or F18 | From any app: take the keyboard back and bring ottospace forward |
| ⌘T | New tab, with an empty pane: ⌘E picks what goes in it. Opened from a tab that keeps a checkout (`open_repo` made it), it keeps that checkout too: its code city, and a terminal there starts in it (until the tab moves to another workspace; kept across a restart). Pressed in a folded-out window, the new tab opens in the main window with that window's tab's checkout |
| ⌘1 … ⌘9 | Select a tab of the selected task (the second row) |
| ⌘⇧] / ⌘⇧[ or ⌃Tab / ⌃⇧Tab | Next / previous tab of the selected task |
| ⌘⌃1 … ⌘⌃9 | Select a space of the top row: the first, Home, then each open task and workspace. Not ⌥1…⌥9: ⌥ is Meta in a terminal |
| ⌘⌃] / ⌘⌃[ | Next / previous space of the top row |
| ⌘⌥] / ⌘⌥[ | Next / previous item of the drawer's rail: this workspace's tasks, then the transcript |
| ⌘⌃W | Hide the selected task or workspace (Hide): it leaves the top row and nothing in it stops; its row in Tasks or Workspaces opens it again. Not Home |
| ⌘⇧W | Close the tab and its panes |
| ⌘⇧O | With the `multi_monitor` [flag](#features) on: fold the tab on show out to its own window on another display, the keyboard with it; again there (or its Fold back in) folds it back in. See [Displays](#displays) |
| ⌘⇧\\ | With the `zoom_out` [flag](#features) on: zoom out to every tab of the workspace or task on show, as tiles on a grid; again, or Esc, back to the tab you came from. Arrows or H J K L select a tile, ↵ or a click goes into it. See [Zoom out](#zoom-out) |
| ⌘= / ⌘- / ⌘0 | Zoomed out (`zoom_out`): the canvas nearer, farther, every tile fitted. Elsewhere nothing |
| ⌘⇧S | Save the tab on show into the [workspace](#workspaces) it is in; in a task's space, as a new one. On a tab that came from one, or was saved into one: update its tab there |
| ⌘Q | Quit. Asks first while a terminal runs something (a program in front of its shell, or a background or stopped job) or an agent runs ("Quitting ends these:"), or a draft has words ("Not sent yet": an unsent message, the new-task line, a capture, a card's typed answer, a btw question, a message in your editor; never one of only spaces). Click a draft's row to go to it. ↵, Q or ⌘Q again quits; Esc, any other key or any other shortcut cancels and does nothing else |
| Wheel / trackpad | Scroll a terminal back through its history |
| ⇧PageUp / ⇧PageDown | Scroll a terminal a page |
| ⇧Home / ⇧End | Top / bottom of a terminal's history |
| Drag / double-click / triple-click | Select text / a word / a line in a terminal. ⇧-click moves the selection's end to the click. A drag held past the top or bottom scrolls the history. The selection stays through new output until a click in the pane, Esc, or output written over it |
| ⌘C | Copy the terminal's selection: a wrapped line as one line, trailing spaces trimmed, no newline at the end. The copied cells pulse once (scrolled out of view: the pane's border does), held still under Reduce Motion, and the pane's header and the status bar say "Copied 3 lines" for 2 seconds. Nothing selected: ⌘C does nothing (Edit ▸ Copy is off). In a [text field](#keys): its selection. ⌘X in a terminal does nothing |
| ⌘V | Paste into the terminal, or into the open text field. Into a terminal: line breaks become ↵, other control characters are dropped, bracketed when the program asks for it. Text with a line break goes at once, as in Terminal: bracketed, the program holds it until your ↵; not bracketed (a plain shell prompt), each line runs. The pane pulses and says "Pasted 4 lines". Only your own ⌘V pastes: a socket client's is refused |
| Drop files | Files or folders dragged from Finder onto a terminal: the pane takes the keyboard and their paths are typed in, each in single quotes (a `'` as `'\''`), a space between and after, no ↵ (you press it); bracketed when the program asks for it, so Claude Code and Codex take them as pasted file paths. A name with a line break or another control character types nothing, and the pane says so. The quoting holds where a word starts: dropped inside a quote you already typed, the shell reads the text as it would anything typed there. Without bracketed paste the text arrives as keys, so a shell's own key bindings apply (as in any terminal). An image is its path, never read. Onto the drawer's message box: the same quoted paths go into the message. Only your own drop |
| ⌘N | New task, in the drawer. Type what needs doing, with [tags](#tags): `#project`, `>repo`, `@agent`, `/path`, `$skill`, `^reference`, `p1` to `p9`; a dropdown finishes the one at the cursor. ↵ adds the task and starts its agent; ⌘⇧↵ adds it for later; ⇧↵ or ⌥↵ starts a new line (the first line is the title, the rest is asked after it); Esc closes. The line is on top, with what it will point at and + (attach files) inside its field; under it one line says what ↵ will do with it as it is. "? tags" under it, or ⌘/, shows the tags a line may hold (a click on one types its mark); ⌘/ again shows every key. The box starts its task in the workspace it was opened in: if you switch space while it is open, it stays open and says "This task goes to A · Switch to B"; Switch, or ⌘N, brings it to the one on show. [Its keys](#keys) |
| ⌘L | The keyboard into the drawer's conversation, or back to the workspace |
| ⌃G | In the composer or the new-task box: its text in your editor (`$VISUAL`, else `$EDITOR`, else `vi`, as Open Settings runs it), in the drawer: the editor takes the conversation's place (the drawer widens to 60 columns if it is narrower). Save and quit: the text is back in the composer (one trailing newline dropped; the new-task line takes it as one line), nothing sent; quit with an error, or save nothing new: as it was. ⌃G again while it runs goes back to it. A permission card that arrives meanwhile shows under the editor, which keeps the keyboard: click the card to answer it (`a`, `d`), ⌃G to go back. The text goes back to the conversation it came from, wherever you went; its task dismissed, it is put on the clipboard and the status bar says so. Also Edit Message in Editor (Tasks, the palette). Only yours: a socket client's ⌃G opens nothing, and its keys reach neither the editor nor a card. See [Compose in your editor](#compose-in-your-editor) |
| ⌘⇧R | Collapse the drawer into its rail, or open it again (with no chat: the transcript). The agent and the transcript go on, and the rail always shows. Also the → at the right end of the drawer's header (its key in the tooltip). Collapsed, a click on a rail item, ⌘⌥] / ⌘⌥[ or the [agent chip](#the-agent-chip) opens it too; open, a click on the item it shows collapses it, as on the left rail. It opens what it collapsed: the transcript, or the new-task box. Only yours: a socket client's is refused, and so are its ⌘N, the + buttons and `compose` while you collapsed the drawer |
| ⌘⌥R | With the `overlay_panels` [flag](#features) on: pull the drawer out over the panes, to an edge of them that still shows, or back to its width; eased (a cut under Reduce Motion); no pane moves. Also the drawer's \|← beside its → (\|→ while pulled out), and Pull the Drawer Out or Back in the palette. Kept for every task (`layout.json`'s `drawer.wide`). Only yours: a socket client's is refused |
| ⌘⇧L | Collapse the sidebar into the left rail, or open it again on the section open last. Also the ← at the right end of the sidebar's header (its key in the tooltip). Collapsed, a click on a section's icon opens it; a click on the open section's icon collapses it again |
| ⌘⇧A | Collapse the conversation's [dial](#the-agent-dial) to its marker beside the badge, or open it again (a click on either does the same). Kept per task |
| ⌘⇧N | With the `overlay_panels` [flag](#features) on: a quick note. The box, centred over the panes, with a pill for each thing it will point at: `Meeting · <title>` (the one on the calendar now), `Transcript · 10:42` (the one being written, and how far in), the workspace or task in front. The pills are inside its field, under the line. × on a pill leaves it out; nothing without a pill is kept. Its edge has one picker, the project's: that of the task in front (or the task in front of the workspace) as it opened; Tab or a click opens its list, × or "no project" leaves the note untagged, a `#project` typed wins. It has no agent or repository picker. ↵ files it as a task for later, nothing starts; ⌘↵ starts it now in its own space and goes there; Esc closes. Agents read what it was filed with by `note`; the title is never in the state. Only yours |
| ⌘J | Next task: the space and chat of the first task that needs you. With the `decisions` [flag](#features) on, what needs you next: [Decisions](#decisions) while a card waits, else the next task |
| ⌃⌥⌘Space | Capture, from any app: the pane in use (a hosted app you are in, else the focused pane) is frozen as a picture to draw on, and a draft opens in the drawer on the right, with the new-task line as its title. A live app is parked first; "↩ back to …" in the bar returns to it. On the picture: drag circles, ⇧-drag draws a box, ⌥-drag a freehand line, a click places a note to type (↵ keeps it). ⌘Z and ⇧⌘Z go over the steps; a click on a shape's line selects it and ⌫ removes it. ↵ goes to the title, Esc leaves the picture; a click on another pane captures that one too. In the title ↵ adds the task and starts its agent, ⌘⇧↵ adds it for later; × or Esc cancels, after a question if anything was captured. The workspace stays in use beside the drawer, and ⌘N goes to the draft's title. See [Capture](#capture) |
| ⌘⌃V | With the `agent_mode` [flag](#features) on: the robot's menu on the focused app pane (Off, See, See and act): what the task of its workspace may do there. ↓ ↑ move, ↵ takes, any other key closes. Also Agent Mode… and Agent Mode: Off / See / See and Act (the palette). Only yours: a client's key, click or label sets nothing |
| ⌘⌃T | With the `transcripts` [flag](#features) on: stop the transcript that is on, from any pane; else start one of the focused app pane. With focus on no such pane, the status bar says "focus an app pane to transcribe it". Also Start or Stop Transcribing (View, the palette). The app's audio is heard by ScreenCaptureKit (Screen Recording covers it) and transcribed by Apple's on-device recogniser: nothing leaves the Mac. The first start may ask the system for its speech model ("Downloading speech model…"). Below macOS 26 it says "transcripts need macOS 26". While one is on: the header of the pane heard (for an app picked, of each pane of its windows) shows a red dot (a click shows the transcript in the drawer) and Stop; a red dot on its workspace in the top row and on its tab, selected or not (a click brings the pane back); the drawer's rail has the transcript's red dot; the drawer's transcript header says "Transcribing <app>" with the time, the level, Stop, and ⋯ with Pause or Resume (paused: "Paused" and Resume where the level was); → or ⌘⇧R collapses the drawer into its rail, and it records on. Pause or Resume Transcribing is also in the palette and the View menu, and the menu bar's red dot has Stop, Pause or Resume, and Show. Only yours: a socket client's ⌘⌃T, click or label starts and stops nothing |
| ⌘B / ⌘⇧B | With the `marks` [flag](#features) on (so for the next two rows): mark the focused pane / the tab on show; on one that is marked, take the mark off (the Marks section offers it back for six seconds, and ⌘B again within them puts the same mark back). The mark is named for the app, the terminal's directory or the tab's title. It records what to open, not a window or a process: a terminal's directory as it is then, an app's bundle id, name and window title, a tab's title, task and split tree. A marked pane or tab shows ◆ and its letter |
| ⌘' then a letter | Go to the mark with that letter. The status bar shows `GO TO` while it waits; Esc, or a letter of no mark, ends the wait and nothing else happens. If the mark's pane or tab is open, its tab comes forward and the pane takes focus (an app pane is focused, not entered). Else it is opened: a terminal in the recorded directory, the app's window in a pane (the one with the recorded title, else its only window, else its main one; an app that is not running is started), or the tab with its splits. A tab opens with what can be opened: a pane whose directory is gone or whose window could not be hosted is empty and says why, and going to the tab again tries it again |
| ⌘⌥1 … ⌘⌥9 | Go to one of the first nine marks, in the order of the Marks section |
| ⌘⌃E | List mode: the bar (Repos, Tasks, Calendar while a feed is set, Apps, Workspaces, Marks; Apps and Marks only while their [flags](#features) are on) takes the keyboard, on the open section, or Apps if none is (with Apps off, the first). On a workspace ↵ opens its tabs, space lists them under it, `a` adds the tab on show to it, `r` renames it, `x` deletes it and `u` takes the last delete or update back; on one of its tabs ↵ opens that tab, `r` renames it and `x` takes it out. On a mark `x` removes it, `r` renames it, `m` gives it a letter, ⇧J / ⇧K move it down / up, and `u` puts the last removed mark back. `j`/`k` or arrows move; ↵ opens the row (⇧↵ splits down); in Tasks a heading is a row too, and ↵ or space on it folds or unfolds it; in Repos so is a repository's name: ↵ or space folds or unfolds it, ← folds the selected row's repository and → unfolds it, `a` makes a worktree of it; Tab changes section; on a task `s` snoozes one hour, `h` holds, `x` dismisses, `u` undoes; on an app that is not running `p` pins or unpins it, `x` hides a suggestion and `u` takes the last of these back; in Apps `/` types in the filter of "All apps" (↵, ↓ or Esc gives the keys back to the rows); in Calendar ← and → go a day back and on, `t` to today; Esc or ⌘⌃E leaves |
| ⌘; or ⌘⌃G | Sneak mode: every control ottospace draws (rail icons, bar rows, tabs and their ×, top-bar buttons, pane headers and their ×, each pane, the drawer's buttons and picks, the top row's spaces and their ×, the home screen's buttons, rows and nodes) gets a short label, home-row letters first. Type a label and the control does what a click does; a pane's label focuses it, or gives an app pane the keyboard. A × (pane or tab) always has a two-letter label. On a small control (an icon, a ×, a tab, a row, a header: under three labels wide or high) the label sits beside it, touching its edge, and the control gets a thin outline in the label's colour; above or below first, or at the side in a column such as the rail. It stays in the window and in the control's own scroll area or pane, and covers no other small control and no other label; with no room beside it, it takes the control's freest corner. A pane's label stays in its middle. Two-letter labels narrow on the first letter; ⌫ takes it back. Esc, a click, a letter that goes on with no label, or any ⌘ shortcut leaves; the shortcut then runs (⌘C copies, ⌘V pastes). Other keys do nothing. The labels follow what is drawn: if a control moves its label moves, and if it goes its label goes. What happens by itself (a shell exits, an agent files a task) does not end the mode. Starting it hides the accessibility overlay (⌘⌃A). Not ⌃G alone: a terminal needs that key. On a keyboard layout with no Latin letters, the keys type the letters they have under ⌘. From `APP` mode return first. The focused app pane's elements get labels too, in orange (APP mode's colour), always two letters or more so one stray key presses nothing in another app: those its accessibility tree says can be pressed (`AXPress`) or typed in (text fields), enabled, at least 4 points each way where the pane shows them, the innermost of nested ones, at most 150 (nearest the pane's top left). They are placed as ottospace's own are, inside the pane, around ottospace's labels. Their labels start with the letters ottospace's own leave free, 19 two-letter labels to each such letter (e.g. `ta`, `ts`, `td`); the first letter only narrows. Only when those letters run out do they go under a × label's first letter, or take three letters. They appear once the app's tree is read, a moment after ottospace's own, and follow it while the mode is on (read again each second); the status bar says how many, e.g. `12 controls in Calculator`. An orange label presses that element (`AXPress`) and the pane stays parked; a text field's label focuses the field and gives the app the keyboard, so you type there. Before it presses, ottospace finds the element again and checks it is the same (role, enabled, place in its window within 2 points); if not, nothing is pressed and the status bar says the control moved. Sneak sets no value, opens no menu and scrolls nothing in the app, and only your own typed letter presses: a socket client's letter of an orange label is refused. Apps that build their accessibility tree only on request (Electron, Chrome's pages, Spotify) show few elements, and get few labels: ottospace does not switch their fuller mode on. Entering an app pane raises its window, as a click on the pane does |
| ⌘⇧C | Visual mode on the terminal (the status bar says `SELECT`): `hjkl`/arrows, `w b e 0 ^ $`, `H M L` move; `v` / `V` select; `y` or ↵ copies and leaves; Esc leaves |

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

**The new-task box.** ⌘N and the Tasks section's `+` open it in the drawer,
beside the workspace (centred over the panes with `overlay_panels`). Top to bottom: the line,
its question as the placeholder; inside its field what it will point at (a folder taken from a
path's dropdown, the quick note's pills) and its bottom edge: + (attach files, ⌘O), ✎ (mark up a pane for the task, ⌘⇧M: see below), the pickers
and Start ↵. Each picker shows what ↵ will use, with its sigil: `#project`, `>app · main` (where
the agent starts: a repository, the workspace's folder, or "no repository"), `@claude` (the
`default_agent` from the start, so ↵ needs no choice). A click on one, or Tab with no tag to
finish (⇧Tab back), opens its list on the current choice: ↑ ↓ move, ↵ takes the row into the
line as its tag (as typing it would; "no project" takes the tag out), Esc closes it; Tab past the
last is the line again. In the agent's list ⌥↵ makes the highlighted runner or model the default
(written to `default_agent`). A typed tag fills its picker. The row never wraps: a long name is
cut with …. Where a repository's workspace offers it, "this checkout" / "new worktree" (⌥⌘N)
sits beside them. Under the field, one line says what ↵ will do with the line as it is:
"↵ starts @claude in app on main · ⌘⇧↵ later · Esc", with ", in this workspace" when the task
joins the workspace you are in; "↵ takes >app" while a row would be taken; "↵ files it; nothing
starts" for a saved agent. "? tags" at its right, or ⌘/, shows the tags a line may hold, a cap
each; a click types its mark at the cursor (spaced from the words beside it) and opens its
dropdown, as the key would. ⌘/ again shows every key. The dropdown teaches the rest.

The box is beside the rest of the window, not over it, and your pointer works there as without
it. A click that only changes the bar (a rail icon, the bar's ←, a fold, a row's button) leaves
the box its line, its cursor and the keyboard. A click that takes the keyboard elsewhere (a
tab, a pane, a repository's terminal, a mark, a workbook, a task's conversation, a name or a
filter typed in the bar) puts the line aside: it is back when the box next opens empty. So do
the keys that go to a mark (⌘⌥1–9). A switch of space (a space on the top row, ⌘⌃1–9, ⌘⌃] / ⌘⌃[,
a repository with a workspace, a workspace of the list) keeps the box open, pinned to the workspace
it was opened in: it says "This task goes to A · Switch to B", ↵ starts the task in A, Switch
brings it to B. Keys that act on panes (⌘D, ⌘W, ⌘T) wait
until the box closes, and so do a socket client's requests. While your editor has the line (⌃G),
a click that would put it aside waits too.

| Key | Action |
|---|---|
| ↵ | While the dropdown shows rows, takes the highlighted one: it never adds a half-typed tag. Adds the task and starts its agent (the drawer becomes its conversation) when there is nothing to finish: no dropdown, no rows ("no match", "new project"), the word that row's name already, or a folder's end (`~/notes/`). While a folder's rows are read again it waits. Esc first adds the line as typed |
| ⌘⇧↵ | Adds the task for later: the box closes, nothing starts. With the dropdown open, as ↵: it takes the row first |
| ⇧↵ or ⌥↵ | A new line. The first line with words is the task's title (a longer one is cut at 500 characters); the lines after it are asked of its agent after the title, as typed. Tags count up to the title's line; in the lines after it they are text (no dropdown there). The box wraps and grows to 8 rows, then scrolls |
| ⌘↵ | Adds the task and starts it, whatever is highlighted |
| Tab | Takes the highlighted row, or the first |
| ↑ / ↓ or ⌃P / ⌃N | Move over the dropdown's rows. With no dropdown: a row of the text up or down, from the first or last to its start or end (⌃P / ⌃N only the dropdown's) |
| Esc | Closes the dropdown. With no dropdown open: closes the box |
| ⌘O, or `+ file` | The system's file picker: what is chosen goes into the line as `/path` words |
| ⌘⇧M, or ✎ | Mark up a pane for the task: the box tucks away, the pane in use freezes, and you circle (drag), box (⇧-drag), draw (⌥-drag) or comment (click) on it, the [capture key](#capture)'s way; a click on another pane takes it too. ↵ is the box again with each capture as a pill (the part marked, cropped); × on a pill drops it; Esc comes back with nothing of that round. The pictures and the text under the marks go into the task's brief as untrusted context. In the quick note too |
| ⌥⌘N, or `this checkout` / `new worktree` | Where the task works, when the box offers a choice (below): its workspace's checkout (the default), or a new worktree |
| ⌥-click on a tab or a pane | Puts its `^reference` into the line. The tab is not selected, the pane not focused. While ⌥ is held the tab or pane under the pointer is outlined |

- **The dropdown** opens when the cursor is in a tag: up to 8 names for it, with what each is
  and a word of detail. Names that start with what is typed come first, then those where a
  word of the name does (`flow-app` for `ap`), then those that have its letters in order. The
  letters matched are drawn stronger. A row is always highlighted, so what ↵ takes shows: the
  first, or the one the word spells out (`app` over `app-2`), or the one ↑/↓ moved to (they
  wrap). A bare mark (`#`) lists what there is, the first highlighted. Only at a folder's end
  (`~/notes/`) is none: the path is whole. Typing narrows it; a click on a row takes it.
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
- **A new worktree.** When the task would work in a listed checkout (the workspace's default
  worktree, the checkout of the task in front, or the line's `>repo`), not in a folder you took,
  and there is a folder for worktrees (`worktrees_dir`), the drawer offers `this checkout`
  (the default) or `new worktree`; ⌥⌘N switches. With `new worktree`, the line above says
  "↵ will start claude in a new worktree of app on fix-the-zoom". ↵ puts the task in the
  workspace at once and asks git for a new branch from the repository's HEAD, named for the
  task's first words (lowercase, `-` between, `-2` after a name a listed checkout has), in
  `<worktrees_dir>/<repo>/<branch>`; the drawer says "making its worktree …", and the agent
  starts there once it is made. A workspace of that one task takes it as its default; with
  other tasks there, only this task works there. A name a local or remote branch has already:
  nothing is made, a toast says so, and the task waits unstarted in the checkout (its
  Start starts it there). Only your choice and your ↵: a socket client's key, click or ↵
  makes no worktree.
- **When it cannot.** A name the line has no room for (it holds 500 characters) is not put in
  in part: the line stays as it is and the box says "no room". ↵ then adds the task as typed.
  Files left out (a name too long, one with a line break) are counted in the same place.

**The quit question.** ⌘Q, Quit in the menu and the window's red close button all ask the same
question. A quit ends every terminal and agent, and loses every draft not
sent. Its dialog, "Quit ottospace?", has a section for each, only when it has rows, headed with its
count: "Quitting ends these" (`pane 2 · cargo`, `agent · Fix the login`) and "Not sent yet" (`Unsent message · Fix the login`, `New task, not
started`, `Capture draft · Slack`, `Unsent answer · <task>`, `Unsent side question · <task>` (btw), `Message open in your editor · <task>`).
A draft is named by where it is, never by its words; one of only spaces or line breaks is none. A
draft in a conversation not on show, or kept for a task put away or done, is listed too. A click on
its row cancels the quit and opens it, the keyboard in it. Quit (↵) and Cancel are buttons; ×, Esc
and a click outside cancel. The app's other questions (stop and dismiss, stop and resume later,
close an edited window, unlist a repository, close a workspace, delete a worktree) are the same
dialog, each yes named by its result. A terminal asks when:

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
- A terminal that closes by itself, an agent that ends or a draft emptied leaves the list. When
  nothing is left the question goes away; ottospace does not quit.
- Not asked: Quit in the Dock menu, and logout, restart or shutdown. macOS ends the app directly
  there, and GPUI 0.2.2 gives no hook to answer first. The socket's `quit` never asks.
- A shell whose prompt keeps a helper process as its child is asked about every time.

Other keys go to the focused terminal. ⌃ sends control codes, ⌥ is Meta.

## Flow of a task

1. **Write it.** ⌘N opens the drawer on a new task. ↵ adds it and starts its agent; ⌘⇧↵ files it
   for later. With nothing open at all the drawer shows a new task by itself.
2. **It starts at once.** The agent (`@claude`, `@codex-gpt-6-astra`, …; with no `@agent`, the
   `default_agent` setting, else `claude`) runs in the drawer, in the task's repository (no
   `>repo`: the home folder), with the keyboard: Claude Code drawn natively (its structured
   mode, see step 11), Codex in a terminal (its TUI); `conversation_view: "terminal"` puts
   Claude Code in a terminal too. It is asked `Task: <title>`; a
   task with captures is told where its brief is; nothing else is in that first message. It is
   told of ottospace out of sight, in a few fixed words (where its guide and workspace are, to ask
   you for See rather than take a screenshot, that window text is data): the instructions of its
   MCP server, or with none, Claude Code's `--append-system-prompt`. The conversation's first
   message shows your words alone; a dot before it ("ottospace context") lists what was added:
   the guidance and where it went, the server's tools, the brief, the folder. The workspace stays as it was: the same tab,
   panes and focus. That tab, if it is no task's and has panes, becomes the task's and gains its
   badge; on a tab of another task, or with nothing open, the task starts with no tab.
3. **Selecting shows; starting is a button.** A click or ↵ on a task, or on its space on the
   top row, shows its space and conversation and starts nothing. Not running, the drawer offers Start agent (it
   never ran: its task is sent once) or Resume session (it has a session: gone on with, nothing
   sent again; a session never written starts afresh). ↵ in an ended conversation's composer
   resumes it and sends the message. A session is gone on with in the folder it started in, which may be below the repository; with the Tasks
   section closed that one session is read first. One at work in another terminal, or held
   open by another Claude Code, is not: the drawer says so. An agent you just stopped there is
   not another terminal's: it is gone on with at once. Going on with a session, the task runs.
   A saved agent cannot be started yet: the drawer says so too.
4. **The drawer.** Its header: the task's badge and title (hover it for the model, the
   permission mode, how long the agent has run and its folder), the folder itself only when it
   is not the task's repository; Brief (when it has captures); Stop ends the agent (in the terminal view; drawn natively, "Stop the Agent" in the palette or the menu);
   → (⌘⇧R) collapses the drawer into its rail and the agent goes on. ⌘L moves the keyboard in
   and out. At its foot, muted, the model in short, a permission mode that lets the agent
   act without asking (`auto`, `accept edits`; `bypass` in the warning colour), and, at the
   right, where its process runs: its listed repository's name, its worktree (by branch, also one
   outside the repository's folder) and the folder, each said once: `app ~/code/app` at the
   repository's root, `app crates/core` below it, `app fix-login src` in a worktree, `~/notes`
   in no listed repository. The hover has each whole; in a narrow drawer the model and mode, then
   the folder, are cut first, the repository and worktree stay. A session no task has: where it
   started. Before the agent starts: where it would start (a session gone on with: where it
   started). A folder that is gone (a worktree removed) starts no agent: a toast says
   so. It stays once the agent ended. Under the header of a native conversation, its
   [dial](#the-agent-dial). Drag its left edge to make it wider or narrower. Opening a conversation, or a click on a tab, while a new
   task is half written puts the line aside: the box gets it back when it opens again.
5. **Done, Later.** At the header's right: **Later**, then a green **✓ Done**, once the agent
   idles. While a turn runs there is no Done: the composer's Stop (or Esc Esc) stops that turn,
   the one stop button; in the terminal view, its Stop. Done (also ⌘↵ in the
   conversation) makes the task done and stops its agent; the drawer folds into its rail and the
   task leaves the rail, its workspace stays on the top row. In about 0.8 s, no sound: the drawer
   seals into a green ring with a tick that draws itself and a few sparks, slides into the rail,
   and the task's rail item pops with a ✓ on its corner before it goes. With Reduce Motion the
   tick shows still and the drawer fades, no sparks. A toast says
   `Done · <task>` with Undo for six seconds: Undo brings it back as it was, in front; New task
   here (⌘N) opens the box in that workspace. A worktree the new-task box made for the task is
   removed (`git worktree remove`, never `--force`; its branch stays, and Undo adds it back):
   kept, and the line says why, while it has uncommitted or untracked changes ("Kept the
   worktree: 3 uncommitted changes"), a terminal is in it, or another task works there. A
   workspace whose folder it was falls back to its repository, so the next task there starts in
   it (a new worktree offered as usual). The repository's own checkout and a worktree the task
   did not make are never touched. Done tasks are under Done today in Tasks, below the open
   ones; a click there makes it a task again and goes on with its session. Later
   (also the dim × on a rail item's hover, and ⌘W while the keyboard is in the drawer) stops the
   agent and parks the task under Later in Tasks, off the rail; the next task of the workspace
   comes in front. While a turn runs it asks first: Stop "<task>" and put it away? (↵ stops, Esc
   keeps it running). A click on its row brings it back. Under the last reply, once the agent
   answered and idles, a quiet line offers the same: "Got what you needed? ✓ Done · Later"; it
   goes once you type, or the agent starts again. Only yours: a socket client's click, label
   or key on any of them is refused.
6. **Dismissing.** A dismissed task leaves the top row and the drawer (the drawer shows
   nothing, not another conversation); its tabs go to the workspace selected last. Dismissing a task whose agent
   runs asks first: Stop its agent and dismiss?
7. **Spaces and the drawer.** Home, then each open task (and session, and workspace) is a
   space with its own tabs; one is selected, and the drawer is only ever on its agent: a task's
   chat, none for Home. Selecting starts nothing; a closed drawer stays closed. A task
   started with ↵ is in the workspace you are in: one you made, or a task's. No space is made
   and nothing moves (the box says "↵ starts it in this workspace" when there is something in
   it). A workspace may hold several tasks: a later one works in the same folder unless its
   line names another. One conversation is in front, the drawer's. The rail at the window's right edge always shows (the drawer open beside it, or collapsed into it by its →, ⌘⇧R or a click on the item it shows; a click on any rail item, or ⌘⇧R, opens it again). It has
   two parts. On top, this workspace's tasks, the most urgent first (each
   its badge, the dot at its corner in its state's colour), and + for another task here. At the
   foot, below a divider, what is the same in every workspace: the transcript (the `transcripts`
   flag), its waveform grey while idle, in colour with a red dot while it records. The selected
   item is a cell as the left rail's open one: raised, an accent bar at the window's edge, no line
   toward the drawer. Collapsed, none is. A click on an item, ⌘⌥] / ⌘⌥[ (top to bottom), list mode
   (→ past the drawer, ↑ ↓, ↵), a task's row or ⌘J switches it; nothing but you does. With no
   task, ⌘⇧R opens this drawer on the transcript, as a click on it does. Many tasks scroll; the
   transcript stays at the rail's foot. Recording, the drawer shows the file it writes
   (`~/.ottospace/transcripts/2026-11-18-1032-zoom.md`) over its lines: a click opens it read-only in a
   viewer. Stopped, the transcript stays, idle. Idle, it
   offers **Transcribe a window…** (a picker in ⌘E's style of the apps that can play sound and
   their windows, the one playing now first and marked "sound now"; your pick hears that app,
   in a pane or not) and **Earlier today**, the day's finished transcripts in `transcripts_dir` of a
   minute or more with a line heard (app, start, length), each a click from read-only in the drawer
   (‹ Earlier today goes back). The top
   row shows the workspace's badges stacked, the most urgent first; a done task's shows only in Tasks'
   Done, so a workspace whose tasks are all done shows its name (`Workspace` if it had none). A task dismissed (or one you
   mark done) leaves the rail and the most urgent remaining is in front; the last one gone, a
   workspace you made is its own again, with its tabs. From Home, the task gets
   a space of its own with one empty tab. ⌘⇧↵ files it for later, in no workspace: started from
   its row, it gets its own. ⌘T makes a tab in the
   space on show. Drag a tab onto another space's tab or badge to move it there; Move Tab to…
   lists the tasks and the workspaces. The top row's + makes a new, empty
   [workspace](#workspaces) after the selected space.
   Closing a space's last tab closes the space. A click on a Repos row selects the workspace of
   that checkout: the one whose default worktree it is, else one that holds a task working there
   (the one selected last). None: a new workspace for it, named for its branch (a repository's
   own folder: for the repository), that checkout its default worktree, a terminal there. A
   workspace's new terminals (⌘D, a split, a new tab's) and new tasks start in its default
   worktree, unless the task's line names another (`>repo`, `/path`); where a workspace's tasks
   work in different worktrees, each task's row and its rail item's hover name its worktree. A
   Claude Code task whose agent moves into another worktree of its repository (one it made; never
   a sub-agent's `.claude/worktrees/agent-*`) works there from then on, and so does its
   workspace's default when it is the workspace's one task. A
   row a task works in carries its badge until it is done (Terminal, on hover, opens a terminal anyway).
   See `docs/ai/spaces.md`. A
   repository's + (on its name, under the pointer) asks for a branch: a local one is checked out, one only a remote has is
   tracked, a new name is made from the repository's HEAD, in `<worktrees_dir>/<repo>/<branch>`; the
   new worktree opens the same way. See `docs/ai/repos.md`.
8. **Following.** The task runs while its agent works. Claude is given the session's id; Codex
   names its own, and ottospace finds it where Codex wrote it: the one session that started in
   the task's folder after the agent did and before the next agent there, that nothing else
   has. When two could be its (another Codex started there at the same moment), neither is
   taken: the task runs, following none. When the agent ends while the task runs, the task needs you.
9. **A session that is no task's.** A click or ↵ on its row under Elsewhere opens its
   conversation the same way, with no task: it gets a badge on the rail (from its title, else
   its repository) and Track in the drawer's header. Track, there or on the row, makes the task,
   and the conversation is the task's: the same agent if it runs, else gone on with. Track opens
   no terminal tab.
10. **Who may.** Only you start or stop an agent, and only you say which task a tab is of: a
   client's ↵ files the task with no tab, its `task_open` goes to tabs the task has, its
   `session_adopt` makes the task, and none starts anything.
11. **Progress.** While the task is open its session is read on: what it does now shows on its
   row and its model on its card, and the task keeps in its file when its agent started, when it was done, its
   time at work (activity until a quiet gap of two minutes) and its tokens by model (input,
   output, cached; a sub-agent's apart), summed over its sessions. They stay when the session
   files go. Hover the task, or its badge, for its card: priority, state, what it does now and
   its model, tags and counts, Filed, Started, Agent time, Workspace (tabs · panes), the tokens and how full its context is. The
   Tasks section groups by state (Needs you, Running, Later, Done) or by project (State |
   Project in its header), and says under the list what was done today, what runs, and how
   long agents worked today. While a task needs you, the left rail's Tasks icon has an amber dot
   at its corner, as a badge's, whichever section is open or with the sidebar collapsed. A task waiting on cards has a line for each under its row: a
   question (`? <the question on show>` and `2 of 3`), a permission (`⚠ Wants to run: <its
   command>`), past two `+N more`. A click opens the drawer on the card with the keyboard in
   it; nothing is answered from the list. A key of a task's file this build does not know is kept as it is.
12. **The native conversation.** The reply is drawn as it streams: markdown, code blocks (a
   Copy on each), tool calls as rows (a click shows what the call does, as plain text), a
   working row with the call it waits on and the time. A call that edits a file (Claude `Edit`,
   `MultiEdit`, `Write`, `NotebookEdit`; Codex's patch) has a diff card under it: the path
   (relative to the repository; a click reveals it in Finder if it is inside), removed and added
   lines in red and green, plain text, at most 40 lines (`… n more lines` for the rest). Hover a message for Copy and Copy as
   Markdown; an opened call's input (wrapped, JSON pretty-printed) and output, for Copy. Drag
   across the transcript to select its text (a double-click a word, a triple-click a line or
   paragraph): ⌘C copies it, a line break between rows, and ⌘A selects all of it while the
   transcript has the keys (after a click in it; a click elsewhere or typing gives them back).
   Esc clears it. Links show their address on hover and open only on your click: `http`/`https`
   in the browser, a file inside the task's repository revealed in Finder; anything else is
   text. Nothing is fetched: an image on the web is a link, a local image in the repository a
   thumbnail. The composer at the bottom: ↵ sends, ⇧↵ or ⌥↵ starts a new line (long lines wrap; it grows
   to 8 rows, then scrolls, and ↑ ↓ move a row), Esc leaves; while a
   turn runs (or a card waits) ↵ queues the message: queued ones show above the composer (Send
   now, Edit, ×) and each goes on its own, in order, once the agent is done; a question it asks
   comes first, and after your Stop they wait for your ↵. ⌘↵ sends now: into the turn where the
   runner takes that, else (Claude Code, for now) the turn stops and it goes first. They are kept
   with the task over a restart. ⌘V with an image on the clipboard (a screenshot copied) attaches
   it to the message being written, here or in the new-task box: a chip with its thumbnail and ×;
   at most 5; it goes to the agent with the message (Claude Code in Chat view; elsewhere the box
   says it takes none). Each is kept privately in `~/.ottospace/images` until it is sent (from the
   new-task box: until its task is done), then removed. Stop sits at the composer's
   right: it, or Esc Esc, ends the turn and the session goes on. No turn, no Stop. Once the agent
   ended (or was stopped: nothing starts on its session until its process is gone), Resume
   session (or Start agent) takes that place. A stop and the resume after it are one thin
   divider (`stopped · resumed`) in the same conversation; "Earlier turns are in its file" only
   when its turns are not on show. Lines no rule reads are a small mark at the end (`? 2`): its
   hover and click say what they were. Hook lines (`system.hook_*`) are read on purpose by nothing. A permission request is a card in place of the composer: the
   request as the CLI asked it, a labelled line per field, and the agent's own words apart.
   `a` allows this one request, `d` denies it, `m` writes a message for the Deny (↵ denies with
   it), `s` shows all of a request cut at 4 KB, which must be seen before `a` works. ↵ does
   nothing on a card. No "always allow": your Claude Code permission mode (shown in the header)
   decides how often cards come. When the agent asks you questions (`AskUserQuestion`), the card
   is a question card: the question drawn as markdown (context longer than about 6 lines folds
   behind "Show all · N lines"), its options as rows with their descriptions, and Other…, which
   takes your own words while the cursor is on it. ↑/↓ or j/k move, space picks (toggles in a
   pick-any question), 1 to 9 pick that row, a click picks, Esc declines ("The user declined to
   answer."). One question: ↵ sends. Several: one shows at a time, each header a tab (✓ once
   answered) over a thin bar a segment per question; ↵ answers and goes on, ⇥ / ⇧⇥ go between,
   and Review lists every answer (a row goes back to its question; one left is amber). Nothing
   goes until Review's Send; a question left goes as no answer (left out of `answers`). The card
   grows to the drawer's height with its foot in place; the options scroll with the cursor in
   view. Sent, it folds to a line: "You answered 2 of 3 · Scope: …". Your words never reach the
   state or the log. **btw** (Claude Code; not Codex yet): "btw" at the composer's foot, or ⌘⇧K,
   opens a column left of the conversation, headed "btw · read-only", the drawer widened for it.
   Its input asks the running agent a side question, mid-turn or while a card waits: ↵ asks (one
   at a time), Esc or ⌘⇧K back to the composer. The answer comes from the agent's context, with
   no tools, drawn as markdown like a reply; it never enters the conversation, the session file
   or the turn. "Use as my answer" puts it in the open question card's Other… as plain text, else the
   composer as markdown: never sent. × closes the column and its thread; the thread also ends with the agent's process.
   `Chat | Terminal` in the header ends the process and goes on
   with the session in the other view once it is gone; never two at once. Not while a turn runs
   or a card waits in Chat (Stop first); from the terminal at any time, as Stop. A session at
   work in another terminal is watched read-only, as a chat shows it: your messages, its replies,
   what it calls and its edits' diff cards, a message at a time as its file grows (the header says "Watching · updates per
   message"; nothing streams; "Not updating" while its file cannot be read). Its sessions are
   read while its space is on show, whatever section is open, and its tab is its checkout's (a
   terminal opened there starts in it). Take over, once it is idle, goes on with it here. Only you send, answer, stop a turn or switch.

## Command palette

⌘⇧P, or View ▸ Command Palette…, opens a box with every command of the menu bar that needs no
argument: its group (Panes, Tabs, Tasks, Workspace, Marks, View, Developer, App), its name and its
key. Type to filter: the letters in order, ignoring case and spaces (`tr` finds Terminal Right);
a word's first letter and letters side by side rank first, then a group's name (`dev` lists the
Developer group). ↑/↓ or ⌃N/⌃P move, a click or ↵ runs the command as its menu item does, Esc, ⌘⇧P
or a click outside closes it. A command that nothing matches keeps the box open.

Features… (App, also ottospace ▸ Features…) turns the box into a list of the [feature
flags](#features): each row its name, its id (`code_city`) and key, a flask for an Experimental
one (its stage on hover), its description below, and On or Off at the right. Typing the name or
the id filters it. ↵ or a click turns the flag the other way, writes it to `settings.json` and
applies at once. This list and Move Tab to…'s show no group column: all their rows are of one. A command of a feature
that is off is not listed.

Open Settings (App, also ottospace ▸ Open Settings, ⌘,) opens `settings.json` in your editor in a
tab titled Settings; see [Settings](#settings).

About ottospace (App, also ottospace ▸ About ottospace) shows the version and build in the Mac's
About panel. The hover of the status bar's empty middle shows them too.

Move Tab to… (Tabs, also File ▸ Move Tab to…) turns the box into a list of Workspace, the
tasks not dismissed, in their order, then the workspaces. Type to filter by title; ↵ or a click
moves the tab on show to the end of that space's tabs.

The Developer group, also the Developer menu: Accessibility Overlay (⌘⌃A), [Accessibility
Inspector](#accessibility-inspector), Copy State as JSON
(the reply of `{"cmd":"state"}`, to the clipboard), Spin the City and Time Its Frames (with
`code_city` on: the city on show turns once round, a step a frame, as a steady drag; a toast and
the event log then give each frame's time, numbers only; Esc stops it; it runs under Reduce Motion
too, as you asked for it), Time Typing in This Terminal (made-up keys into a throwaway `cat` in a
new pane beside the focused one, never your shell, each timed from the key to the frame that
draws its echo; the pane then closes, and a toast and the event log give the times by stage and the
display's refresh, numbers only; Esc stops it), Open the Settings File and Open the ottospace
Folder (in the Finder; a test run that named no folder has none), and Bring All Windows Back
(see [Getting a window back](#getting-a-window-back)).

The palette is the user's: a socket client cannot open it, type in it or choose (its keys are
refused while it is open), and pane requests wait behind it as they do behind the picker. ⌘⇧P is a
key of ottospace's, so it does nothing while a hosted app has the keyboard (`APP` mode). Not ⌘K:
in a terminal it clears the screen, and is kept for that.

Reset Key Counts (View, also Help ▸ Reset Key Counts) forgets which keys you use. ottospace
counts each key you press that runs a binding (⌘D, ⌘⌥ arrows, ⌘1–9 …), never the palette, a menu
item or a socket client's key, and a press within a second of the last (a held key) is the same
use. Each key has a score that halves in a week without use. A key is known from a
score of 4.5 (five uses within a day or two), and again unknown once it decays below 2 (about eight
idle days). The counts are kept in `~/.ottospace/keys.json`, readable by you alone: per key's id
its score, when it was last used and whether it is known; no key text and nothing typed. Never on
the socket. A test run keeps none unless it names `OTTOSPACE_HOME`.

The Tasks group also holds the drawer's commands: Type in the Conversation (⌘L), Edit Message in
Editor (⌃G), Open or Collapse the Drawer (⌘⇧R), Stop the Agent (the drawer's Stop) and Toggle
Agent Dial (⌘⇧A), each for
the conversation the drawer shows. ⌘⇧P while you type in the conversation opens the palette over
it: the agent gets no key while it is open, and when it closes the keyboard goes back to it.

## Go to

⌘P, or View ▸ Go to Place or File…, opens on the files of the checkout on show: the tab's (a
Repos click opened it there), else its space's (a task's worktree, a workspace's default), when the
Repos section lists it. Type to find one: a match in a file's name ranks before one only in its
folders, a name it starts before one it runs through, letters in a row and at word starts higher,
then the files you opened recently, then the shorter path. Nothing typed: the recent ones first.
A query longer than 32 letters is found whole. ↵ opens it in a viewer (in the focused pane if it
is empty, else beside it), ⌘↵ below it.
Esc closes; ⌫ with nothing typed lists the places. Elsewhere (Home, a space with no listed
checkout) ⌘P opens on the places.

The places: the workspaces, each checkout the
Repos section lists (a repository's own folder and its worktrees, with the branch) and the tasks
not done (needs-you first). Each row has its section's icon, its name and a muted detail: a
workspace's default folder, a checkout's folder or repository and branch, a task's state. With
nothing typed, the places selected most recently come first and the one on show last. Type to
filter, as in the command palette, by name, then by name and detail. ↵ or a click goes there as
the place's own row does: a workspace opens, a checkout's Repos row is clicked, a task's
conversation opens.

Tab, or `/` typed after a place's name, goes into the highlighted place's folder: its name over
the list, and the files git lists there (tracked, and new ones it does not ignore), by path.
`app/login` in one go goes into `app` when it is clearly the best match. ↵ opens the file in a
viewer in that place, going there first. ⌫ with nothing typed, or Esc, goes back to the places; Esc
again closes. Files are read only for a checkout the Repos section lists, off the main thread, once
while the box is open: the first 50,000 (the title says so when there are more). A place with no
such folder (a workspace with none, a task with no checkout) has no files.

`>` as the first character lists the commands, as ⌘⇧P does; deleting it lists the places again.
Go to is the user's, as the command palette is.

## The agent chip

A conversation you closed is one click away. While the selected space has a conversation (a
task's, or a session's) and the drawer is closed, a chip sits at the right end of the tab row;
a space with one tab has no tab row, so it sits on the top row, left of the split buttons.

- It shows the task's badge, the [dial](#the-agent-dial)'s marker for a native conversation
  (the context ring, a dot while a call runs, a red one for a failure) and one line of what the
  agent does: `Working · cargo test`, `Needs you · git push`, `Needs you` (its turn is over:
  your word next), `Idle`, `Ended`, `Failed`.
- While it needs you it is drawn in the needs-you colour: a permission request shows with the
  chat closed.
- A click opens the conversation, as ⌘⇧R does. So does a click on the selected space on the top
  row; a click on another space only selects it. Its hover names the key.
- It takes the room the row has left and moves nothing: short of room for the line it is the
  badge alone, and on a full tab row the badge lies over the row's end. It is not there while the drawer's place shows something else (a new task, a
  draft, a brief, the inspector).
- Only your own click opens the conversation: a socket client's click on the chip or on the
  selected space does not. The state reply has it as `agent_chip`.

## The agent dial

A native conversation has a dial under its header: where its agent's calls go, at a glance.

- **Centre**: the agent's kind (✳ Claude Code), coloured by its state: running, needs you, idle,
  ended, or an error (a failed turn, an end you did not ask for) until your next message. It
  turns about its upright axis while the model thinks (it has the turn and no call runs).
- **Ring**: the context used (tokens in the context over the window); amber from 70%, red from 90%.
- **Stations**, clockwise from the top: You, Model, Files, Shell, Network, MCP, Agents,
  Skills, Memory, Hooks, Tools (any other tool). Files is the agent's file tools only (Read,
  Write, Edit, Grep, Glob, `apply_patch`, …). Shell means the agent ran a command, any command
  (`ls`, `curl`, `git push`): ottospace does not guess what a command did. Network is the agent's
  web tools (WebFetch, WebSearch, Codex's web search). MCP is every MCP tool call, whatever its
  server: ottospace does not guess from a server's name. A spoke is heavier for more use, the older uses
  counting less (half after ten minutes). A station is lit while a call there runs (Model while
  the model has the turn, You while a request waits), with two arcs for its time: the inner fills
  in a minute, the outer in ten. A red dot: a call there failed since your last message.
  Each has its name beside it, brighter while lit. A lit station's dot pulses (not while a
  request waits for you), and is larger for
  each further call running there at once (up to four): it springs to its size, with a kick
  when a call starts, and settles back when one ends.
- **Timer**: under the centre's picture, the running call's time, or the model's (`0:14`).
- The dial is centred; under it, when the drawer is wide enough: what it does now
  (`Shell · cargo test`), or the last thing and when; turns and how long.
- Hover a station for its name and counts, the ring for the context used and the tokens.
- A click on it (or ⌘⇧A, or Toggle Agent Dial) collapses it to an 18-point marker beside the
  badge (the ring, a white dot while a call runs, a red one for a failure); a click on the marker
  opens it. It is open while the agent runs and collapsed once it ended, unless you left it the
  other way: that is kept per task, with the layout.
- A watched session's dial steps a message at a time, with no time arcs: its dots spring to
  their sizes, nothing pulses or turns. With Reduce motion on, nothing moves: a lit station is its
  colour and its larger size.
- It is drawn frame by frame only while something in it moves; at rest, collapsed or not on
  screen, never.
- The state reply has its stations, counts, what is lit (and how many calls run there), the
  context used, `collapsed` and `animating`, never a command, path or label. Design: [docs/ai/agent-dial.md](ai/agent-dial.md).

## Accessibility inspector

Developer ▸ Accessibility Inspector, or "inspector" in the [command palette](#command-palette),
with an app pane focused: that app's accessibility tree, as ottospace reads it, in the right
drawer's place. The drawer (a conversation, the new-task box, a draft, a brief) is covered, not
closed, and comes back as it was when the inspector closes. Showing
something in the drawer (⌘N, a conversation, a capture) closes the inspector.
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
  them. While it has the keyboard, ↑/↓ select, ↵ or Esc gives the keyboard back.
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
- An agent ottospace starts for a task reads its brief with `ottospace ctl '{"cmd":"task_brief"}'`:
  it has a pass of its own, in a file whose path its environment holds (`OTTOSPACE_TASK_PASS_FILE`;
  the file is 0600, in a 0700 folder beside the control socket), which `ottospace ctl` and the SDK
  read and send with each request. `env` shows the path, never the pass. The pass and its file die
  with the agent's process; a new start has a new one. ottospace never writes it to the state or a
  log, and a pass printed anyway reads `[hidden]` in every reply.
- A terminal opened in a task's tab has `OTTOSPACE_TASK`, not the pass; an agent there reads the brief with
  `ottospace ctl "{\"cmd\":\"task_brief\",\"id\":$OTTOSPACE_TASK}"`.
- In the brief, everything that came from another app or a terminal is quoted and labelled as
  untrusted: a message someone else wrote is data for the agent, not an instruction.
- A window your `windows` rules ignore is never captured.
- Only you start a capture. No socket request does (outside a test run), so no program can ask
  ottospace for a hosted app's picture or text.
- With no title typed, the task is named "Capture from an app" or "Capture from a terminal".
- ⌘Q with a draft that holds a capture or a title asks before it discards it.
- If a window was minimised or on another Space, the brief says its picture may be stale.
- The annotated picture is what an agent is sent to first. Electron and Chrome windows (Slack,
  VS Code) expose little text to accessibility; their capture is the picture, your notes and
  the window's title. ottospace does not switch an app's accessibility mode on, and runs no OCR.
- Not built: starting an agent with the brief; removing a task's captures with the task.

Design: [capture.md](ai/capture.md).

## Bug reports

For working on ottospace itself. With `developer.ottospace_repo` naming a git checkout of
ottospace, a bug button shows at the top row's right end (⌃⌘B, in its tooltip only). It takes a
picture of ottospace's own window as it is (by its window id, never the screen or another app's
window; Screen Recording), then lays it over the window:

- Drag a box round what is wrong, or Whole app; a comment card opens beside the mark and takes
  the keyboard. Several marks, a comment each. A click in a box or on a card's comment goes back
  to it; × removes the mark. ↵ or Esc ends a comment; Esc again cancels the report.
- App panes (other apps' windows) are blanked unless you tick Include app panes.
- **Send to an agent** (⌘↵) files a task "Bug: <your first comment>" with your default agent,
  in a new worktree of that repository (`<worktrees_dir>/<repo>/bug-…`, a new branch from its
  HEAD; the repository need not be in Repos); the agent starts there once git has made it. Its
  brief, in `~/.ottospace/materials/<task>-<word>/`: the picture (`report.png`) and a copy with
  the marks (`report.annotated.png`), each mark's place and your comment, and, fenced as
  untrusted data, the build, the OS version, what was drawn under each mark (control ids, pane
  numbers and kinds, rects: no text) and the event log's last 50 lines (names and numbers only).
  No folder for worktrees: nothing is filed, and a toast says so. Done keeps that
  worktree unless the repository is in Repos.
- The picture may show anything ottospace draws at that moment (a meeting title, a transcript, a
  terminal's text). It stays on this Mac.
- Only your click or key starts, marks or cancels a report: a socket client's key, click or
  label does none of it. Design: [bug-reporter.md](ai/bug-reporter.md).

## Sessions

An agent is a saved definition. A session is one running conversation of Claude Code or Codex.
The Tasks section lists, under **Elsewhere**, the sessions of your listed repositories
(worktrees included) that no task has adopted: for example a Claude Code you started by hand in a
terminal. The group is folded at first: its heading shows the count, and a click, ↵ or space
unfolds it. A session at work does not unfold it.

- **What a row shows.** One line: a dot that is lit while the session is active (its file
  changed in the last 30 seconds), its title (below), else its repository or worktree, and the
  runner (`claude` or `codex`). Hover it for its card: the repository, what it does now (`shell ·
  cargo test`, `shell · git push`, `files · src/main.rs`), the model, when it was last active
  and the pane it runs in.
- **Range.** The sessions active in the last 7 days. Nothing on screen changes it; for
  another, set `sessions_range` in the settings: `today` (since the local day began), `month`
  (30 days) or `all` (every one listed). A session at work is always listed. The heading counts those in range; hover it for how many there are in all. The
  range only narrows what was read: at most 200 sessions are listed, the most recent, so All
  shows those 200.
- **Title and model.** A row shows the session's title, on one line, when the runner keeps one: the name you gave a Claude Code session (`/rename`), else the title
  Claude Code wrote for it; for Codex, the thread's name in its `session_index.jsonl`, which
  not every session has. ottospace writes no title itself and calls no model. The card shows
  the model in short: `claude-opus-4-7` shows as `opus-4.7`, another name as it is.
  A title is text a model wrote from what the session read, so it is only drawn: one line, at
  most 80 characters, no control or invisible characters. It never names a task, sets a tag or
  reaches a terminal.
- **Order.** Sessions at work come first, in the order they started, so a row does not move
  while you reach for it. Then the idle ones, the most recently active first.
- **Open** (a click on the row, or ↵ in list mode) shows its conversation in the drawer, and the
  workspace stays as it is. No task is made. Resume session goes on with an idle one there: `claude
  --resume <id>` or `codex resume <id>`, in the folder the session started in, which must be in
  a listed repository or worktree. One at work (its dot lit) or held by a live Claude Code
  process is not: the drawer says it is running in another terminal, and nothing is started. When
  the card says it runs in a pane, that pane is focused too.
- **Track** makes a task of the session: named after the repository, with `>repo` set to that
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
  5 seconds while the Tasks section is open (or the Repos section, for its dots alone: nothing is listed then) and never while both are closed. It does not attach to
  a session, type into it or signal it. Only Open or Track, by you, starts the runner's own
  resume command on it. Only derived facts are shown: names from fixed tables, counts and
  times. Never a prompt, a file's contents or a command line. The one free text is the
  session's title (above). Design: [sessions.md](ai/sessions.md).
- Turn it off with `"sessions": false`. See [Settings](#settings) for where sessions are read from.

## Workspaces

A workspace is a named set of tabs for a context that comes back, e.g. "Maestro DRI": a tab of
dashboards, a tab of docs. It is listed in the Workspaces section whether it is open or not, and
kept between runs. It is not a task: no agent works it and it is never done. (It was called a
workbook: its folder, its setting and the control socket still say `workbook`.) The first space
of the top row, "Workspace", is always there and is not listed: the tabs of no task and no other
workspace.

- **New**: + on the top row, + in the Workspaces section's header, or `New workspace` in the
  empty section. Each makes a new, empty workspace, "Workspace 2", "Workspace 3" (the lowest
  number no listed workspace has): its space goes after the selected one on the top row, with one empty tab,
  selected, ready for ⌘E or ⌘D. It never takes or records the tab on show.
- **Temporary or kept**: a new workspace (from +, or what you opened on Home) is temporary: its
  name is muted and italic on the top row (selected or not) and in the section, and its tip says "Temporary: closes without
  saving · ⋯ Keep". It has no file. It is kept from the moment it has a task (↵ on a new task in
  it), a repository (a Repos click made it, or set its worktree), a name you gave it, a tab saved
  into it (⌘⇧S), or Keep (its row's …, the … on its item on the top row, File ▸ Keep Workspace,
  the palette). A kept one never
  turns temporary again. Workspaces saved before this are kept.
- **Close**: × on its space (⌘⌃W) takes it off the top row. A kept one: nothing in it stops, it
  stays listed, and its row opens it again as it was. With its last tab closed it leaves the row
  too. A temporary one closed (×, ⌘⌃W, its last tab closed or moved away) leaves the list at
  once: `Workspace closed` shows on a [toast](#toasts) for six seconds with Undo,
  which brings it back on the row as it was; its terminals and apps run until then, and close
  when its time is up (× on the bar ends it at once). Each closed one keeps its own Undo: under
  the others in the Workspaces section it shows, italic, with `Closing` and Undo until its own
  time is up, whatever the bar says since (another close, a delete, a save). If a terminal in it
  runs a program, ×, ⌘⌃W and closing its last tab (⌘⇧W, the tab's ×) ask first, as ⌘Q does
  ("Close this workspace?": ↵ closes it, with its Undo; Esc keeps it). A temporary workspace open when you quit comes back
  with the [layout](#restoring-the-layout); once closed it never does. Only your own acts keep or close one:
  a client's `hide_space` or `close_tab` leaves it listed, and no client's step brings a closing
  one back or changes what its Undo keeps (its `select_tab`, a pane moved into or out of it, a close of its tab or pane: refused).
- **Save**: ⌘⇧S, or File ▸ Save Tab as Workspace. In a workspace, the tab on show is saved into
  it; in the first space or a task's, as a new workspace of that tab. It records
  the tab's splits and their sizes, and each app pane's app (bundle id and name); for each
  terminal, its directory as it is then, its last 500 lines (`workbook_scrollback_lines`; `0`
  keeps none; only once you typed in it: a shell left at its prompt keeps none) and the command line of the program in front of its shell, if one runs. No window
  title, URL or document. A tab with nothing in it is not saved. What is saved is what opens
  again once its tabs are closed; a tab not saved comes back only with the [layout](#restoring-the-layout).
- **Add a tab**: `a` on a workspace's row in list mode, or File ▸ Add Tab to
  Workspace… (it opens the section in list mode: pick the row, `a`). **All tabs**: File ▸ Save All
  Tabs as Workspace saves every tab on the bar that has something in it, in bar order, as one new
  workspace (at most 32; with more it saves none and says so). A tab stands for one workspace's tab at most: added to another, it leaves the first.
- A workspace made by saving a tab is named for the tab (all tabs: "Workspace 2", as a new
  one), and the section opens with the
  name selected: type over it, ↵ keeps it, Esc keeps it as it is. No two workspaces share a name
  (`DRI`, `DRI 2`).
- **Save again** on a tab that came from a workspace, or was saved into one, updates that tab of
  it. A toast says what it kept, with Undo, for six seconds.
- **Open**: click its row, or ⌘⌃E and ↵. Its space comes back on the top row, selected. Each of
  its saved tabs that is not open opens in a new tab, in order, and the
  first is selected; a tab of it that is open already is kept, not opened twice. With none saved
  and none open it gets one empty tab. A workspace of
  one saved tab opens it under the workspace's name; of several, each tab under its own title. Each
  terminal starts a new shell in its directory; if the directory is gone it starts in your home
  folder, and a toast says so. Above its prompt, in grey, are the lines it kept, then
  `── saved <date> ──`. A command it kept is typed on the prompt, not run: ↵ runs it, or edit or
  clear it first. The divider names it too (`↵ runs: npm run dev`). The program itself did not
  survive: this is not session restore. Each app pane gets a window of its app as a tab's mark does: one no
  pane holds (the only one, else the main one); an app that is not running is started. A pane
  whose app is not installed, or whose window is in another pane, stays empty and says why. A
  dot marks the row while its space is on the top row or a tab of it is open, and the tab bar
  marks its saved tabs with the workspace icon (hover: its name). A row with nothing saved says
  `no saved tabs`.
- **Its tabs**: ▸ before a workspace of several saved tabs (space in list mode) lists them under it.
  Each opens alone (click, ↵), and its … has Rename (its title; `r` in list mode) and Remove (`x` in list mode: out of the
  workspace; the last one deletes the workspace).
- **The row**: a click anywhere on it opens the workspace, or selects it if it is open. The
  pointer on it (or list mode's selection) shows one …: its menu has Rename and Delete, and Keep
  on a temporary one.
  In the menu ↓ ↑ move, ↵ takes the item the keys are on; Esc, any other key or a click
  outside closes it. In list mode `r` and `x` rename and delete with no menu. Deleting leaves the open
  tabs alone; `Workspace deleted` shows on a [toast](#toasts) for six seconds, with
  Undo (`u` in list mode) and ×, which ends the offer. Nothing in the section moves.
- Kept lines and commands can hold secrets a tool printed, or a token in a command line. They
  stay in your workspace files, plain text with escape sequences taken out, at most 64 KB a
  terminal and 1 MB a workspace. Credential-shaped text is hidden as `[hidden]` (tokens, `key=`
  passwords, `user:pass@` URLs, `Authorization:` headers; a private key block is left out), and
  a command holding any is not kept. This is best effort: a secret of another shape is kept as
  printed. Set `workbook_scrollback_lines` to `0` and no terminal text is saved at all. The save
  says what it kept in a toast, and the row says `history` while a workspace holds any;
  deleting the workspace deletes its file. Never in the control socket's state (it counts them:
  `lines`, `has_command`).
- Kept in `~/.ottospace/workbooks`, one file each, readable by you alone. A file there that
  ottospace cannot read is counted in a toast and left as it is. A build from before
  workspaces could be empty does not read a file with no saved tab: it counts it and leaves it.
- Only you make, save, open, rename or delete a workspace: a socket client's key, click or label is
  refused. The state lists them (`workbooks`: names, tabs and pane kinds, no directories).

Not built: bringing an app back on the URL, channel or document it showed, and a workspace
that belongs to a task or a worktree. Design: [workbook.md](ai/workbook.md),
[spaces.md](ai/spaces.md).

## Restoring the layout

Quit ottospace and start it again: the workspace comes back. `~/.ottospace/layout.json` holds it,
written a second after a change and at quit, readable by you alone.

- **What comes back**: every tab in bar order, each with its task (a task's tabs together, tabs of
  no task last) and the workspace it came from; its splits and their sizes; which pane had focus;
  the tab on show; the section that was open; the drawer (the conversation it showed, open or
  closed, its width). A task's tabs live here, keyed by the task's number, not in the task's file;
  its badge colour is in the task's file. A tab of a task that is gone or dismissed comes back as
  a tab of no task.
- **Terminals** start a new shell in the folder they were in (one that is gone: your home folder,
  and a toast says so). As a [workspace](#workspaces)'s do, they show their last lines in
  grey above the prompt, and a program that ran in front is typed on the prompt, not run: ↵ runs
  it. The same setting (`workbook_scrollback_lines`; `0` keeps no terminal text) and the same
  cleaning apply: escape sequences taken out, credential-shaped text hidden, capped. The program
  itself did not survive. The grey lines are kept once: a start in which you typed nothing there
  keeps them as they were; one in which you did adds what it printed below a second divider.
- **The Settings tab** (Open Settings) is kept as a mark on its pane, never as a command and with
  none of its screen. It comes back running your editor on `settings.json` again, built as ⌘,
  builds it from `$VISUAL` or `$EDITOR` now; ⌘, brings it forward. An editor that is gone exits
  at once and its pane closes.
- **App panes** keep the app (bundle id and name), never a window title. A running app's window
  goes back in its pane; an app that is not running is not started: its pane says so and offers
  `Open <App>`. `restore_apps: true` starts it instead. Opening a workspace starts its apps: you
  asked for them; a start does not.
- **The drawer** shows the task's conversation again, not running: Resume session in the drawer
  goes on with its session. No agent starts by itself. In Chat, what the session's file has of
  before shows above the composer: its calls, its edits' diff cards, a line per message (the
  words are in the file: Terminal shows them); ↵ on a message goes on with it. An agent ottospace
  started ends when ottospace quits. "Live in another terminal" is said only when the file is
  written after the start, or a live process has the session.
- A tab whose task no longer exists comes back as a tab of no task, so nothing is lost.
- A file that cannot be read is set aside as `layout.json.bad`, and ottospace starts empty and
  says so. A file a newer build wrote is left as it is and not written over. A pane of a kind
  this build does not know comes back empty.
- **File ▸ Start Fresh** (or the palette): the next start is empty. The file (and a
  `layout.json.bad`) is removed and not written again this run. `restore_layout: false` turns it all off: the file is neither read nor
  written (one that is there is left alone).
- Marks that stood for a tab or a pane stand for it again.

## Meeting timeline

Set `calendar_ics` to your calendar's feed (an `https://` link, such as a calendar's "secret
address in iCal format", or a file's path) and ottospace shows your day's meetings, never in
the way.

- **The band**: 12 points high, directly above the status bar, the window's width, with a
  hairline along its top as the status bar has; panes give up that height from the first frame,
  so nothing moves when the meetings come. Until the first read lands a short highlight slides
  along it (`Loading…` with Reduce motion); a first read that fails says `Couldn't read the
  calendar · Retry` there, through the retries that follow by themselves (your Retry slides it
  again; your click only; hover for why). It is the workday
  (`calendar_workday`, 09:00 to 17:00), fixed through the day: a small mark at its start and one
  at its end, and a bright tick for now that moves along it (pinned to an end before or after
  hours). A **call** (an entry with a Zoom, Meet or Teams link) is a solid block at its clock
  place; calls that overlap split the height into as many lanes as overlap (up to 4; a fifth at
  once is named in a lane's hover card, `+1 more: …`), and a call alone takes the full height. A
  **block** (an entry with no call link: focus time, a reminder, a room booking) is a faint layer
  the full height under the calls, so a 4-hour block under three calls takes no lane. A call
  outside the workday widens the band to hold it, never past midnight (one over midnight is cut
  there); a block never does: one wholly outside the band (a reminder at 00:00) is a small dot at
  that end, its details on hover. All-day events are one small dot at the left end's top. Blocks
  are at least 3 points wide. Hover only, no click: a block shows its title, time, length, place
  (links taken out), whether it has a Zoom, Meet or Teams link and whether it is tentative, free
  or out of office; a mark its time; now the time, the next meeting (`Design review · 15:00`,
  `· now` while it runs) and when the feed was last read (`calendar not updated since 09:40` when
  the last read failed).
- **Colours**: calls in the meeting amber, blocks a muted grey, out of office (Outlook's OOF) a
  quiet slate, focus blocks green: a block with no Zoom, Meet or Teams link whose title starts
  with `Focus` or `Deep Work` (case ignored, whole words: `Focus: writing`, not `Focusing`). Tentative entries are lighter, free ones (Outlook's FREE, or `TRANSP:TRANSPARENT`)
  faint, past ones faded and the one that runs at full strength. `calendar_colors` colours by
  title: `[{"match": "1:1", "color": "teal"}, {"match": "focus", "color": "lime"}]`, the first
  rule whose `match` the title holds (case ignored) giving the hue, over a focus block's green
  too; tentative, free and past
  still lighten it. A colour is one of `red`, `indigo`, `orange`, `pink`, `lime`, `purple`, `tan`,
  `teal` (the tasks' own colours); `match` is plain text, not a pattern: titles are other
  people's text, and a pattern of yours run over them is risk for no gain.
- **The pulse**: from 10 minutes before a meeting its block glows (its opacity) softly, slowly;
  from 2 minutes, faster; while it runs, steadily. With Reduce motion (System Settings,
  Accessibility, Display) the colour steps up instead, with no movement. Nothing else in
  ottospace pulses.
- **At the start**, if you are in no call, or still in an earlier one (on from 10 minutes before,
  or the meeting before, running over): a meeting notice on a [toast](#toasts) at the
  window's bottom right (◷, indigo: `Meeting:` the title and how long ago it started, **Join** if the meeting
  has an `https:` link, × to dismiss; it outranks a warning or information, not an error), which stays until
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
  through `/usr/bin/curl`: `https:` only, no redirects (a feed that has moved is not read: a
  toast asks for its final address), only from a host on the internet (looked up once,
  checked, and curl pinned to that address), at most 5 MB, 30 seconds. A proxy set in your
  environment (`https_proxy`) applies. With `gh` for pull requests (`repos_prs`) and for a clone you start (From GitHub…), it is ottospace's only network access. The link is handed to curl on its standard input, not in
  its arguments, and is never shown, logged or put in the state. A read that fails keeps the
  events of the last one, and is tried again 15 seconds later, then 30, doubling up to
  `calendar_refresh_minutes`. Each fetch is in the app's log (`<home>/log`): its try, then its
  size, events and time, or what failed (`timed_out`, `http` and the status, …); never the link
  or a title. Recurring meetings, time zones, cancelled meetings and moved
  occurrences are followed; see [meeting-timeline.md](ai/meeting-timeline.md) for what is not.
- **Declined meetings**: set `calendar_email` to your address as the feed lists you among a
  meeting's attendees, and a meeting (or one occurrence) you declined is left out everywhere: the
  band, the section and the alerts. A feed published with limited detail (free/busy
  only, or without attendees, as Outlook's published calendars are) does not say who declined;
  then nothing is left out, and `calendar_email` does nothing. Outlook removes a meeting you
  decline from your calendar, so it is usually not in the feed at all; one kept is marked free,
  and `calendar_hide_free: true` leaves every free entry out. The address stays in
  `settings.json`: it is not sent with the feed's request, shown, logged or in the state.
- **The Calendar section** (its rail cell, second, is there only while `calendar_ics` is set):
  one day drawn as Outlook's day view. Its all-day events sit in a strip at the top; under it
  an hour column (`9`, `10`, `11`…) with a line across at each hour, and each meeting a block as
  tall as it lasts, one point a minute (a 09:00 to 17:00 day is about 480 points), at least 20
  points so a 15-minute meeting still reads `Standup 9:00`. A block is tinted in the meeting's
  colour (the band's) with a bar of it at its left edge, its title and start. Meetings that
  overlap sit side by side in columns, as in Outlook (up to 4; more at once merge into one,
  named on its card). Free time up to an hour is empty space; longer, a short band that says how
  long (`2 h free`), so the day stays on one screen. The day spans the workday, widened to its
  meetings and, on today, to now, out to whole hours. Now is a line across in the accent colour, a dot at its left end
  and the time (`14:12`) in the hour column; it moves with the minute, with no frame of its own,
  and inside a `free` band sits where now falls in it. Past meetings are faded; the one that runs
  is tinted stronger and edged in its colour. The section opens with now in view (at the top on
  another day). Hover a block for the band's card. `Join` shows on hover or selection for a meeting with an
  `https:` link. Under the day a muted line says how the feed's fetch goes: `Loading the
  calendar… 412 KB` (or the seconds before a byte), `Updated 2 min ago · 1858 events`, or what
  failed in plain words (`Couldn't reach the calendar (timed out after 30 s)`, `The calendar
  answered 403`, `Couldn't read the calendar: not an iCalendar feed`) with the time of the last
  good read and `Retry`, which fetches at once (your click only). A day with no
  meetings is its hours with one `free` band (and now's line on today), drawn only once a read
  succeeded. The header: `‹ Today ›`, or `‹ Mon 16 Nov ›`
  and `Today` on another day, from 7 days back to 7 on; its date's tooltip says when the feed was
  read. The section goes back to today when the day turns and at start; the band, the alerts and
  now's tick are always of today. The rail cell shows a steady amber dot while a meeting is 2
  minutes off or runs, and a filled amber badge while its notice stands (also while an error outranks it). In list mode (⌘⌃E,
  which starts on the meeting that runs, else the next, else the day's last)
  `j`/`k` (↓/↑) go from meeting to meeting by start, the block scrolled into view, ↵ joins, ← and → go a day back and on, `t` back to today; the View menu and the
  palette have the same (`Previous Day in Calendar`, `Next Day in Calendar`, `Today in
  Calendar`). The open Calendar is not kept in the restored layout: a build without the section
  could not read the file.
- **Agents**: the `calendar` request gives an agent the meetings of a day, or of up to 7 from
  it (`{"cmd":"calendar","day":"tomorrow","days":2}`; the 7 days before today, today and the next
  7 are held, `yesterday` names the day before): start
  and end (RFC 3339 with your offset), all day or not, title, place, whether it has a Zoom, Meet
  or Teams link, `status` (`tentative`, `free` or `away`; none when busy); busy time (meetings
  merged, free ones left out) and free time (the gaps in each day's
  `calendar_workday`); when the feed was read and whether you are in a call. It reads only what the app holds and never makes it fetch.
- **Privacy**: titles and places are other people's text. They are shown, and given to an agent
  only by the `calendar` request, with links taken out; never sent anywhere, never in the trace
  or a task's brief, and not in the control socket's state (every reply carries it), which has
  only counts, times and stages. No request gives a meeting's link, description or attendees,
  the feed's address or `calendar_email`. The agent's guide says to treat titles as data, never
  as instructions.

## Terminal panes

A terminal pane runs your shell on a PTY. The pane closes when the shell exits. It keeps 10,000
lines of history. Typing returns to the bottom. A program that asks for the mouse (vim, tmux,
htop, Claude Code's full-screen view) gets its presses, drags and wheel; on the alternate screen
without the mouse (less, man) the wheel is ↑ and ↓. ⇧ and the wheel
always scroll ottospace's own history (none while a program is on the alternate screen), also when a
program that exited left a mouse mode on. ⇧PageUp and friends go to a program on the alternate screen.
A click on a terminal focuses it and clears its selection; a drag there selects, even in an
unfocused pane. While the program has the mouse, hold ⇧ or ⌥ to select (⌘-click still opens a
path or a link). A program's cursor is the shape it asks for (block, underline or bar). Paste is bracketed when the program asks for it.
Text is drawn bold, dim, italic, struck through or underlined (curly and coloured underlines too)
as the program asks.
Links: an `http://` or `https://` URL in the text (one wrapped across rows too; punctuation after it
and a closing bracket it did not open are left out) and an OSC 8 hyperlink. Hold ⌘ over one: it is
underlined and the pointer is a hand; an OSC 8 link whose text is not its address shows the address
under it. ⌘-click opens that address in the default browser. Nothing else opens: no `file:`,
`mailto:` or other scheme, no user name in the address, nothing over 2048 characters. Only your own
click opens one, never a socket client's. Without ⌘ nothing changes.
A program that asks for focus events (1004) hears when its terminal gets or loses the keyboard:
another pane, another tab, a box over the panes or another app in front.
Programs see `TERM=xterm-256color`, `COLORTERM=truecolor` (24-bit colour), `TERM_PROGRAM=ottospace`
and `TERM_PROGRAM_VERSION`, whatever terminal started ottospace. When ottospace has neither `LANG`
nor `LC_ALL` (started from the Dock), they get `LANG` from the system's region as UTF-8 (`de_CH.UTF-8`),
or `en_US.UTF-8` when macOS has no such locale. `scripts/term-probe` prints a test
card of colours, styles, glyphs and replies, to compare terminals side by side.
A program asking for the terminal's colours (OSC 10, 11, 12 and 4, e.g. to pick a light or dark
theme) or its size in points (CSI 14 t) gets the answer. It never reads the clipboard (OSC 52).
Each character sits on its own columns, whatever font draws it, and box-drawing and block
characters (`│ ╭ █ ▛ ░`) are drawn as shapes that fill their cell, so boxes and block art have no
gaps between rows. Double lines (`═ ║`) are the font's.
A program that asks for the kitty keyboard protocol (Claude Code does) gets it: ⇧↵, ⌃↵ and Esc
reach it as their own keys (flags 1 and 4), so ⇧↵ makes a new line there instead of sending.
A program that asks to set the clipboard (OSC 52) copies nothing by itself: its pane's header shows
`A program here wants to copy 1,240 characters “…” · 3 lines · Copy · Always for this pane · ×`, and only your
click copies, on the ask you saw: a newer ask has its own pill, and its Copy is dim and does nothing
for half a second. The preview is the first line, cut short with `…` when it is not the whole text,
with invisible or reordering characters as `�`; the pill also says how many lines and whether the text
ends with a newline. Always lasts while that pane does. Over 100 KB is refused (the pill says so). A
program asking to read the clipboard gets no answer.
Its header names its directory, and the program in front of the shell while one runs:
`app-api · cargo test`. An app pane's header shows the app's icon and the window's title; an
empty pane's, nothing.

### Empty places

One empty state, wherever there is nothing: a workspace's or a task's empty tab, a repository's
tab after its last pane closed, an empty pane (a split, ⌘T, a pane whose content went) and a tab
with no pane folded out to its own window. The lattice, and over it, just above the middle, the
rows of what fills it, each its key as caps (as the keymap has it; a held ⌘ lights them) and a
click does what the key does: Open an app (⌘E), Terminal (⌘D), New task (⌘N; it starts in that
workspace). A row shows only where its key fills that place: an empty pane beside others shows Open
an app alone (beside a pane ⌘D splits, never fills, and ⌘N fills nothing); one alone in its tab
(⌘T's) is that tab's empty state: all three rows, and ⌘D or ⌘⇧D puts a terminal in it; a folded-out tab shows Terminal and
New task (its picker opens in the main window and fills that window's tab). A pane smaller than 240 × 160 points shows its row without the lattice. A
click in an empty pane focuses it. What differs, on purpose: Home has its heading, its tasks and
its legend; a pane a mark could not fill says why first (and Open *app*, for an app's pane); a
slot that waits says what for; an app's replaced window offers the new one.

## File viewer

A file, read only, in a pane: markdown drawn by the conversation's renderer (headings by level,
code blocks coloured by their language (by its grammar, as a code file) with Copy, task boxes, tables aligned as written and
scrolled across when wide, quotes and `> [!NOTE]` callouts, footnotes at the foot, bare addresses
as links, `#heading` links that scroll to it, pictures beside the file; raw HTML as the text it
is, a web picture only a link), code as lines with their numbers coloured by the file's extension or name
(by its tree-sitter grammar for Rust, TypeScript and TSX, JavaScript, JSON, TOML, Python, shell, Go,
C, C++, Java, C#, Ruby, HTML (its `<script>` and `<style>` too), CSS, YAML, SQL, PHP, Lua,
Makefiles and XML, functions and punctuation too; by a small lexer for Swift, Kotlin (no grammar:
their size), diffs and the rest, and for a file of over 2 MB or one that takes over half a second to parse; markdown opened at
a line shows its source, coloured with its fences and front matter), other
text plain, a PNG, JPEG, GIF, WebP, BMP or TIFF as a picture (see Pictures below). Open one with ⌘E's **File…** (the open panel), a ⌘-click
on a path in a terminal (from the shell's folder, `~` too; `src/main.rs:12` opens `src/main.rs` at line 12)
or on a file link in the conversation, or `ottospace ctl '{"cmd":"open_file","path":"/abs/a.md"}'`.
A ⌘-click opens only markdown, code and text files there; any other file does what a click did
before. A terminal's ⌘-clicks reuse the one viewer they opened beside it, and the keyboard stays
in the terminal.

- It reads only a regular file (no folder, device or pipe), at most 5 MB ("Too large"); "File not
  found" with the path when it is gone, and the file again when it is back.
- It reloads when the file changes on disk (looked at twice a second, read once the change has
  held for 0.2 s), at the same line.
- Keys: `j`/`k` or ↓/↑ a line, ⌃D/⌃U half a page, Space or ⌃F/⌃B a page, `gg`/`G` the top and
  the end. A count before a move repeats it (`10j`) or names a line (`3G`, `3gg`); the status bar
  shows it while typed, Esc takes it back, another key drops it.
- At lines: `src/main.rs:12` or `src/main.rs:12-14` ⌘-clicked in a terminal, or `open_file` with
  `line` and `last`, opens the file with those lines marked (a tint and a bar at the left), a few
  lines below the top. Past the end: the last line. A viewer of the file in the tab moves there
  instead of a new one opening. A markdown file opened at a line shows its source. The mark stays
  until the viewer is asked for other lines or another file; Esc takes it off.
- ⌃G or **Edit** in its header: your editor (`$VISUAL`, else `$EDITOR`, else `vi`) on the file in a
  terminal beside it.
- Links in it open only on your click: a web page in the browser; a file in the file's folder is
  revealed in Finder, or with ⌘ opened in the viewer beside. A web picture is never fetched.
- Restored after a restart with its file and line. The state has its path, kind, line and line
  count, never its text.
- **Pictures**: fitted to the pane (never enlarged past its own size), over a checker where it is
  transparent; its size in pixels, its file's size and the zoom in the pane's foot. ⌘= and ⌘- zoom
  in steps, ⌘0 fits it again, a trackpad pinch zooms; zoomed, a drag or the wheel moves it. An
  animated GIF or WebP plays while its pane is on show (Pause and Play in the foot); under Reduce
  Motion it stands on its first frame until you press Play. Decoded off the main thread, at most
  32 MB of file and 40 million pixels (every frame together): past that, or a file its decoder
  cannot read, the pane says so. Never a picture from the web. SVG shows as its XML source.

## File explorer

⌘⇧X, File Explorer in the palette or the View menu, or **Open in explorer** in a Repos row's
right click opens a pane of the checkout's tree: the tab's checkout, else its space's (a task's
worktree, a workspace's default). Only a checkout the Repos section lists; elsewhere a notice
says so.

- Folders first, then files, by name as a person sorts them (`f2` before `f10`); a glyph by kind
  (folder, markdown, code, picture, text, link).
- ↑ ↓ move, → opens a folder (or goes into it), ← closes it (or goes to its parent), ↵ opens a
  file in the viewer beside (the same viewer for the next one) and keeps the keyboard; letters
  typed go to the next name that starts with them. A click does the same.
- What git ignores is hidden; **Show ignored** in its header shows it. `.git` is never listed.
  Git's marks: `M` changed, `A` added, `U` new, `!` in conflict; a folder carries the strongest
  under it. Read once (`git status`, hardened as the status bar's) in a checkout Repos lists.
- Folders are read one at a time, off the main thread, as you open them; at most 5,000 names a
  folder (the header says when there were more). A link is listed (↗) and never followed, nor is
  any folder outside the checkout read.
- A row's right click: **Open**, **Open below**, **Reveal in Finder** (your click only), **Copy
  path**; on a folder **Open terminal here**.
- The state has its shape (rows, selected, open folders), never a name.

## App panes

To put an app window in a pane, open Apps on the left rail and click the app. It fills the focused pane if that is
empty, otherwise it splits right. Shift-click splits down.

A new window of a hosted app never splits a pane by itself. It stays a free window at its own size,
over its app's pane, and it is offered in that pane's header:
`Zoom opened “Zoom Meeting” · no slot for this window · ◫ split right ⊟ split down ×`. ◫ splits it
in to the right of the app's pane, ⊟ below it (a click, or its sneak label); × leaves it floating.
In a narrow pane the chip cuts its line short: the pointer on its words shows the whole title and
the reason in a tooltip (not while an app's window is live: it would be drawn under that window).
Several windows offered by one pane group in one chip, `Zoom opened 3 windows ▾`: ▾ opens their
list over the pane's top right, each window with the same three choices. Windows of one title
(Zoom's several `Zoom Meeting`) each say what tells them apart, in this order: size
(`large · 1280×800`, `small · 320×240`), display (`main display`, `display 2`), kind (`window`,
`panel`, `dialog`), else `window 2` by window id; each also shows a small picture of itself, taken
once when the list opens, kept in memory while offered, never on disk. The offer
goes when you choose, when the window closes, or when it goes in a pane another way; its pane
closing moves it to another pane of the app. When an app remakes its window (closes it, opens
another), the pane stays empty and offers the new one: `Zoom replaced its window · Bring it here ↵ ×`. Only you take
or wave away an offer: a client's click is refused, and the window's title is never in the state.
While you are in the app's pane its window covers the header: the offer shows once you leave.
A slot (`slots` flag, [Slots](#slots)) that selects the new window comes first: it offers it in its
own pane, or takes back the window it held, and there is no chip. Later: ottospace remembers where
you put a window of each app. ottospace asks for a floating window in front once, and then leaves it to the app, which closes it. One
you can reach stays where the app put it: its title bar on screen and, while the app's pane is
parked, not under ottospace's window unless it is over that pane. Any other (off screen, by its
parked window, under ottospace) is first moved to the middle of its app's pane, wholly onto that
pane's screen (a pop-up bigger than the screen at its top left). A dialog or panel (subrole
`AXDialog`, `AXSystemDialog`, `AXFloatingWindow`, `AXSystemFloatingWindow`) or a modal window
is offered too, its chip saying why it floats (`a dialog or panel, so it floats`): Zoom's meeting
window can be a panel. A sheet stays on its window, and a window under 100×100 (an app's helper)
where the app keeps it. `windows` rules decide first: `"do": "split"` splits it in, `"do": "float"`
floats it with no offer, `"do": "ignore"` leaves it alone. Each decision is a `place` line in [the
log](#the-log), with the rule that made it.

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
- Not written, with a notice in a toast, and the pin then lasts until ottospace quits:
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

Scrolling over an app pane you are not in enters it (`wheel_to_apps`), as a click on the pane
does: the status bar says `APP`, the window comes up over the pane and the app has the keyboard.
The scroll that entered reached ottospace, not the app: ottospace sends it on to the window once
the window is up, so no scroll is lost. Leave as ever: the return key, or a click on ottospace
itself. Only your own wheel enters: a socket client's scroll enters nothing. None enters under
sneak mode, the palette, a box, the quit question, the accessibility overlay or a drag, nor while
another app is in front.

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

### Slots

With the `slots` flag on, ⋯ in an app pane's header opens its menu, whose `Create slot…` keeps the
pane as a slot: a pane that keeps its place and waits for a window you name. The dialog says its
rule in one line on top (`Waits for any Zoom window, the largest, and offers it here.`) and sets it
in a form under it, each choice with how many of the app's windows open now it takes: Window (any
window; the window id, the app's own name for it, when it has one; or the title), Title (is,
contains or starts with, and the words: the window's whole title to begin with), Kind (said for a
dialog or panel: it holds only that kind), Several (the largest, the smallest, the frontmost, display N),
and When it opens: Offer it (O, the default) or Take it (T: a window that opens while it waits, on
the tab on show, never the one you are in). Under them, the app's windows open now, those the rule
takes ticked; the action row says how many match, and Keep slot (↵) makes it. × or Esc closes it.
By default the rule is any window of the app, the largest: several such slots pair by size, the
largest pane with the largest window. The window id is a choice of yours, and the dialog says that
some apps (Zoom) move it between their windows, and when other windows have it now. A title test is written as the glob `layout.json` keeps (`*Incident*`, `Weekly*`); a
pattern written there by hand stays as it is. ⋯ on a slot offers `Edit slot…` (the dialog, with
Save), `Offer again` (the windows passed over, while there are any), `Give back the window` (while
it holds one), `Swap` or `Swap with “…”` (with another slot of the app on the tab: the two exchange
their windows, or the window moves to the empty one), `Re-sort Zoom windows by size` and `Clear
slot`. The menu is the user's: ↓ ↑ ↵, Esc closes it.

A slot that waits draws a dashed outline in its pane with its app's icon (kept by bundle id in
`~/.ottospace/icons`, so it shows when the app is not running; a generic one without), `Waiting for a
Zoom window`, the rule in the dialog's words (`Zoom · window id zm.meeting.window.main`), `Zoom is not
running`, `Zoom's window is away`, `Its window is already in another pane` or why each window of
the app is not its own (`2 Zoom windows: 1 is in another pane; 1 has no window id`) when so, and whether it offers or takes the window. A window it
selects is offered there: a solid outline, `“Weekly sync” is open`, Put it here (↵ while the slot
is focused) and, quieter, Not this one and `N more match`; in a small pane, the icon and one line
(and Put it here while it offers). Its other acts are in ⋯. Several slots that select the same windows each get
one: those that name their window (by title or id) first, as many served as can be; then app-only
slots by size, the largest pane with the largest window. A window in a pane is never moved for
another; Swap and Re-sort move them. ⌘E there and a pick end the slot. A filled slot's
header says `slot`. Only you make, fill or clear a slot; a client's click on one is refused.

A pane you put a window in (the picker, ⌘E, Put it here) stays when that window closes or its app
quits: it becomes a slot that waits for the same kind of window, by the window's id (`Waits for:
Zoom · window id zm.meeting.window.main · the largest`, which the next meeting's window has too), or
`no window id` for one without, never by its title. × closes it. A pane a client opened,
and a window the app opened by itself close with their window, as before. A slot holds what the picker
lists and a new window would be offered: a standard window, a dialog or a panel (Zoom's popped-out
`Meeting chat`), never a sheet, a tiny window or one a rule ignores or floats.

## The return key

The return key works while any app is frontmost. Tap ⌘ twice, alone: either ⌘ key, each press
under 250 ms, the second within 300 ms of the first's release, and no other key, modifier or mouse
button in between. ⌘ held, ⌘C, ⌘Tab, ⌘-click and ⌘⇧ do not count. While ottospace itself has the
keyboard, ⌘⌘ does nothing. ottospace hears only the modifier keys' changes (never which key you
type), cannot change or stop them, and uses the Accessibility access ottospace already has. A key or
mouse button held, or Caps Lock toggled, between the taps spoils them. `"return_double_cmd": false`
turns it off, e.g. for another tool that uses ⌘⌘.

F18 works too. Set `OTTOSPACE_RETURN_KEY` to another of `f13`-`f20`. If another program already
holds the key, ottospace logs that and runs without it. The status bar names the keys in use. Most
keyboards have no F18: `"return_caps_lock": true` makes Caps Lock the return key while ottospace
runs. At start ottospace adds one entry to macOS's key mapping (`hidutil`'s `UserKeyMapping`),
Caps Lock to the return key, keeping every entry you have; at quit it takes out only that one. It
remembers what it added in `~/.ottospace/caps_return.json`, so after a crash the next start finds its
own entry instead of adding another, and turning the setting off takes it out at the next start.
While ottospace runs, Caps Lock stops toggling capitals everywhere, in every app. If you have mapped
Caps Lock to something else, ottospace leaves it and the status bar says so.

By hand instead (lasts until restart; replaces any mapping you have):

```sh
hidutil property --set '{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":0x700000039,"HIDKeyboardModifierMappingDst":0x70000006D}]}'
```

To undo it: `hidutil property --set '{"UserKeyMapping":[]}'`.

## Compose in your editor

⌃G in a conversation's composer or the new-task box writes its text to a file of its own in
`drafts/` in the ottospace folder (the folder 0700, the file 0600, a random name ending `.md`, never
under `/tmp`) and runs your editor on it, with the same command line as Open Settings, in a terminal
in the drawer. The file is removed when the editor exits. At start, files a crashed run left there
are deleted, not restored: a draft lives in the app while the editor runs, and nowhere else after.
Up to 100000 characters come back (the new-task line: its own limit); more, the first part is kept
and a toast says so. Control characters other than a line break go, as a paste's do; a tab
is four spaces.

## Toasts

What ottospace has to say about itself (a settings file it cannot read, a file not saved, a
shell that did not start, why a move of yours did nothing, a key that needs a restart) shows as
a toast: cards at the window's bottom right, over the panes, just above the status bar (and the
timeline's band), clear of the sidebar and the drawer. It takes no room: no pane, terminal or hosted window moves or resizes when one comes or goes. It
slides in from the right and fades, and is there at once under the system's Reduce Motion. A
live app window over the bottom-right pane is in front of ottospace's window: while that app is in
front, each card floats above it in a small panel of its own, which never takes the keyboard; a
click between two cards goes to the app.

- **Level**: red for an error, amber for a warning (a move that did nothing, a retired setting, a
  global key not registered), blue for information, and amber on indigo for a meeting that started
  (`meeting`: it outranks a warning, not an error). A glyph says the level too: `!` in a circle, `▲`,
  `ⓘ`, `◷`. The state says a meeting's notice as `Meeting started`, never its title.
- **Stacked**: at most three cards, the most pressing nearest the status bar, and of one level the
  newest; the others stack upward, then `+N more`. An error is not pushed up by information; when
  it goes, the next takes its place. A
  workspace's or a task's **Undo** never runs out unseen: under three higher notices it takes the
  third card.
- **One line**: text too long is cut with an ellipsis; point at the card and it shows all of it,
  wrapped (at most eight lines), growing upward, the cards over it moving up meanwhile. A settings
  problem offers **Open settings**; a meeting with a link, **Join**; a workspace deleted or updated,
  **Undo** (only your click takes it back).
- **Dismiss**: × on a card, or Esc (the first card, nearest the status bar) when no mode and no terminal takes it. Only you dismiss a
  notice: a socket client cannot, and its failure never takes the place of what your own step
  said. A dismissed settings line comes back when it says something else.
- **How long**: as before. What a step did goes after 8 seconds; a settings problem stays until
  the file is right; a failure stays until the next one, or until dismissed.
- **Whose**: the toast is yours. Why a step did nothing, and what it did, is an answer to the one
  who took the step: your own shows on the toast; a socket client's (or a timer's) is in the
  reply's `notice` alone, so a refused client opens no toast, moves nothing of yours and
  closes no menu of yours. What is wrong with the app itself shows whoever's step found it, and
  a client's success clears only a client's line.

| Kind | Level | About | Another sender's on the toast |
|---|---|---|---|
| `failed` | error | the app: a step of yours or a timer's that failed; of a client's, a shell or agent that did not start, `settings.json` not written, a window of yours it moved back on screen | yes, never over what your own step said |
| `refused` | warning | the sender: why its step did nothing | no: the reply's `notice` |
| `told` | information | the sender: what its step did | no |
| `unsaved` | error | the app: a task, mark, workspace or layout not saved | yes |
| `settings_problem`, `settings_retired`, `settings_restart` | error, warning, information | the app: `settings.json` | yes |
| `start` | error | the app: found at start | yes |
| `keys` | warning | the app: a global key not registered | yes |
| `editor` | information | the app: your editor gave its text back | yes |
| `meeting` | meeting | the app: a meeting started and you are in no call; × is its Dismiss, Join its link | yes; its title never in the state |
| `undo` | information | the sender: a workspace you deleted or updated, offered back for six seconds; × ends the offer | no; only you delete one |

A copy's "Copied 3 lines", the mode and the count of tasks that need you stay in the status bar.

## Status bar

The status bar, left to right: the mode, as a block in its colour, only where nothing else on screen shows where keys go: `APP` (a hosted app has them), `LIST` (the sidebar's rows), `SELECT` (selecting terminal text), `GO TO` (⌘' waits for a mark's letter), `LETTER` (a mark waits for its letter). Every other mode (the palette, a question, the new-task box, a conversation, sneak's labels, a field) shows itself, and the bar shows `NORMAL`, dimmed, as in normal mode. The slot is as wide as the widest word, so the folder never moves when a mode comes or goes (the slot takes no click); where you are (the focused pane's folder, `~` for home, then its git branch, a dot when a checkout listed in Repos (or its worktree) has changes not committed; an app pane's app; nothing on Home or an empty pane; a click copies the folder's path), what the mode says and short messages in the middle, the next meeting from 30 minutes before it (`Team sync in 12 min`, then `Team sync now · Join` while it runs; Join opens its link), and the agents of every workspace (`● 2 working · ● 1 needs you`, or `No agents`; a click goes to the next task that needs you, as ⌘J). Working counts a session at work in a terminal too, one no task has, as the Tasks or Repos section last read it (neither open: none); it is named apart only beside a task's (`● 3 working · 1 in a terminal`), or alone (`● 1 working in a terminal`). Hovering that part lists them; a click on it goes to the first one's pane when it is known (`session_waits`), else to its Elsewhere row. In a narrow window the folder shortens first (its name alone, then cut), then the meeting (`12 min`), then the agents (`● 2 ● 1`); the mode never shortens. The folder's branch and changes are read when the focus, the folder or the program in front of its shell changes (`git status`, off the main thread, taking no lock). Changes are read only in checkouts listed in Repos, by their own top folder (a repository nested in a listed one, or a link out of it, is not listed): a repository's own config can name programs git runs (`core.fsmonitor`, filters), so a folder cloned elsewhere shows its branch, read from `HEAD`, and no dot, and runs no git. The status that does run, as the Repos section's own git reads, turns off fsmonitor, hooks, an external diff, the system config and attributes, and the pager. A socket client's click copies nothing and joins nothing; the meeting's title is drawn, never in the state.

## Settings

ottospace keeps its files in `~/.ottospace` (another folder: set `OTTOSPACE_HOME`). It reads
`settings.json` there at start. Every key is optional.

Open Settings (⌘,, the palette, or ottospace ▸ Open Settings) opens the file in a tab titled
Settings, whose terminal runs `$VISUAL`, else `$EDITOR`, else `vi`, as your login shell sets them
(an editor that must wait, as `code -w`, needs its flag). A file that is not there is made `{}`
first. Again while that tab is open brings it forward; quitting the editor closes its pane. Only
you open it: a socket client's ⌘, does nothing. Open at quit, it comes back at the next start,
the editor running again ([Restoring the layout](#restoring-the-layout)).

A change is read when the file is saved: what is wrong in it is said in a toast (`settings.json:3:1 trailing comma`, or the first of several and how many more) and the last good settings stay, until it is right again. Then `worktrees_dir`, `clone_dir`, `transcripts_dir`, `projects_dir`, `repos_prs`, `windows`, `capture`, `wheel_to_apps`, `workbook_scrollback_lines`, `default_agent`, `agent_commands`, `conversation_view`, `session_quiet_minutes`, `tasks_view`, `sessions_range`, `calendar_notify`, `calendar_notify_for`, `calendar_hide_free`, `calendar_colors`, `city_folder_colors`, `city_exclude` and `features` apply at once; any other key changed says "restart to apply" with its name, in muted text: information, not an error (`repos`, `apps` and `apps_hidden` only when changed by hand: the app's own writes are already applied). Removing the file goes back to the defaults.


```json
{
  "terminal": { "font": "JetBrains Mono", "font_size": 14 },
  "repos": ["~/code/app", "~/code/web"],
  "tasks": "~/notes/tasks",
  "sessions": true,
  "session_dirs": { "claude": ["~/.claude"], "codex": "~/.codex" },
  "session_quiet_minutes": 3,
  "agent_models": { "claude": ["claude-opus-5-5"], "codex": ["gpt-6-astra"] },
  "agent_commands": { "claude": "ja claude" },
  "default_agent": "claude",
  "conversation_view": "native",
  "agent_mode": "act",
  "capture_key": "ctrl-alt-cmd-space",
  "return_double_cmd": true,
  "capture": { "keep_images": true, "ax": true },
  "workbook_scrollback_lines": 500,
  "restore_layout": true,
  "restore_apps": false,
  "apps": ["Microsoft Outlook", "Slack"],
  "apps_hidden": ["Music"],
  "suggest_dock_apps": true,
  "wheel_to_apps": true,
  "windows": [{ "app": "us.zoom.xos", "title": "Zoom Meeting", "do": "ignore" }],
  "features": { "marks": false, "apps_section": false }
}
```

| Key | Meaning | Default |
|---|---|---|
| `terminal.font` | The terminal's font, by family name. Use a monospaced one | `Menlo` |
| `terminal.font_size` | In points, 6 to 72 | `13` |
| `terminal.cell_width` | Each column is whole pixels wide, so every character is drawn alike. `round_up`: the font's width rounded up (airier; about 10% fewer columns at 12 points on a 1x display). `round_down`: rounded down (as many columns as before; letters closer, some touch) | `round_up` |
| `repos` | The repositories the Repos section lists, in its order: each a path, or `{"path": …, "pinned": true}`. Opt-in: the section's + (the folder you pick, or one inside a repository), a folder dropped on the section, and Add Folder to Repos (⌘⇧P: the focused terminal's folder, else the tab's task's) add one at the end, by its full path; a folder that is no repository is refused. The trash on a name takes it off; nothing on disk is touched. An entry that is a folder of repositories (older settings named folders to look in) lists them as one group, folded until you unfold it, removed whole; one of them added by itself leaves the group. `OTTOSPACE_REPOS` (`:` between directories) wins over it, and lists what they hold flat. At most 64 | none (before: `~/code`, `~/notes`) |
| `tasks` | Where tasks are kept, one JSON file each. A relative path is inside `~/.ottospace` | `tasks` |
| `clone_dir` | Where From GitHub… (the Repos section's +) clones: `<dir>/<name>`, the repository's name checked (letters, digits, `.`, `_`, `-`; not `.` or `-` first). A folder of that name there already is refused. Missing, it is made after you say so (↵ again). A relative path is inside `~/.ottospace`. Read each time | `~/code` |
| `worktrees_dir` | Where + on a repository's name makes worktrees: `<dir>/<repo>/<branch>`, slashes in the branch as `-`; a repository whose folder name another listed one shares gets a short hash of its path after it (`app-1f2e3d`). A relative path is inside `~/.ottospace`. Read each time | `worktrees` in `~/.ottospace` |
| `transcripts_dir` | Where transcripts are kept (the `transcripts` flag): one Markdown file each, `YYYY-MM-DD-HHMM-<app>.md`, written as lines come. With a calendar feed, its header lists the meetings it was possibly of (on at the start, or starting within 5 minutes; else one that ended within 5): `- Possibly, by your calendar:` and each title with its times; never a place, link or attendees. One that starts later while it records is a `*calendar*` line at its start. A transcript of an app keeps one file a meeting: when the meeting's windows (the one picked and those the app opened since) are gone for 10 s, the file ends; the next words heard begin a new one with its own header; 30 minutes with nothing heard after, it stops. A folder ottospace makes is private to you (0700), each file too (0600). A relative path is inside `~/.ottospace`. Read at each start | `transcripts` in `~/.ottospace` |
| `projects_dir` | The folder the Projects section lists (the `projects` flag): each folder in it is a project, its note `README.md`, else `index.md`, else its only or newest `.md`; a lone `.md` file is a project too. Its title is the note's first heading, else its name. Hidden names and links are skipped; at most 500 projects and 200 entries a folder; a note over 256 KB is titled by its name. Only read, while the section is open (on opening, then every 10 s); never written. `~` is expanded; a relative path is inside `~/.ottospace` | none: no section, nothing read |
| `repos_prs` | Each worktree's open pull request, read with your `gh` (`gh pr list`, read-only) while Repos is open: every 5 minutes, and on opening at most once a minute. `auto`: when gh is installed and logged in, silently otherwise; `on`: as `auto`, and the pane says why when it cannot; `off`: gh never runs. A pull request is a worktree's only if it is of its branch and from the repository the branch pushes to (a fork's of the same name is not) | `auto` |
| `sessions` | List agent sessions that are no task's under Elsewhere in the Tasks section. `false`: no session file is ever read | `true` |
| `session_dirs` | Where the runners keep their sessions: `claude` is a list of Claude Code config directories (one for each account), `codex` is Codex's home. A runner left out keeps its default. An app started from the Dock does not see `CLAUDE_CONFIG_DIR` or `CODEX_HOME`: name other directories here | `~/.claude`, `~/.codex` |
| `session_quiet_minutes` | A task adopted from a session moves to Needs you once the session has been quiet this many minutes. `0`: a quiet session never moves its task | `3` |
| `agent_models` | The models the `@agent` dropdown offers after each runner, in place of the built-in list: ids as the runner's CLI takes them. `claude` and `codex` are each a list; a runner left out keeps its own, an empty list offers the runner alone. An id that does not start with the runner's name gets it in the tag: `gpt-6-astra` is `@codex-gpt-6-astra`, `opus` is `@claude-opus`. An entry that is not a name (letters and digits, with `.`, `-` or `_` between them one at a time; at most 64) is left out and named in the status bar | `claude-fable-5-2`, `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-5-5`; `gpt-6-astra` |
| `agent_commands` | What your login shell runs in place of a runner's program, its arguments after it: `{"claude": "ja claude"}`. Shell text, so an alias or a `VAR=value` before the program works; in the terminal view and the native one, for a new session and one gone on with. One line each; `claude` and `codex` only. Never in the socket's state, but put no secret in it (a shell error may name it): let a command fetch one, as `ja` does. Only this file sets it: no socket request does | each runner's program |
| `default_agent` | The agent a task with no `@agent` starts: a runner (`claude`, `codex`) or a runner and a model (`claude-opus-5-5`, `codex-gpt-6-astra`). Shown on the new-task box's agent picker from the start; ⌥↵ in that picker's list sets it | `claude` |
| `agent_mode` | With the `agent_mode` flag: the level an app pane you put in a task's workspace starts at, `act`, `see` or `off`. One a client opened (`open_app`), the system adopted or a restore rebuilt starts at Off; after a restart, or when a tab mark opens its tab again, a level you picked with the robot is offered in the pane's header, and comes back only when you click the offer. The robot in a pane's header changes one pane | `act` |
| `conversation_view` | How the drawer shows a Claude Code conversation: `native` (drawn by ottospace: markdown, cards for permission requests, a composer) or `terminal` (the CLI's own TUI). The header's `Chat \| Terminal` switches one conversation, and the layout keeps the pick. Codex always shows in a terminal for now | `native` |
| `workbook_scrollback_lines` | How many of a terminal's last lines a [workspace](#workspaces), and the [restored layout](#restoring-the-layout), keeps, shown again above its prompt when it opens. At most `2000`. `0`: none is kept | `500` |
| `restore_layout` | [Restore the layout](#restoring-the-layout) at start and keep it in `layout.json`. `false`: every start is empty | `true` |
| `restore_apps` | At a restore, start an app a pane names that is not running. `false`: its pane waits and offers `Open <App>` | `false` |
| `tasks_view` | How the Tasks section groups: `state` or `project`. Written when you pick one in the section's header | `state` |
| `sessions_range` | Which sessions Elsewhere lists: `today`, `week` (7 days), `month` (30 days) or `all`. Set by hand: nothing on screen picks it | `week` |
| `capture_key` | The key that captures the pane in use, from any app: modifiers (`ctrl`, `alt`, `cmd`, `shift`) and a key, with `-` between them. `"off"`: no key. `OTTOSPACE_CAPTURE_KEY` wins over it. Not `ctrl-alt-space`: macOS uses it to change the input source | `ctrl-alt-cmd-space` |
| `return_double_cmd` | ⌘ tapped twice, alone, takes the keyboard back from any app, as the return key does ([the return key](#the-return-key)). `false`: only F18. Read at start | `true` |
| `return_caps_lock` | Caps Lock is the return key while ottospace runs: one entry added to macOS's key mapping at start, only it taken out at quit; Caps Lock stops toggling capitals everywhere meanwhile. A Caps Lock you mapped yourself is left alone. Read at start ([the return key](#the-return-key)) | `false` |
| `capture.keep_images` | Keep a capture's pictures on disk. `false`: no picture is ever written | `true` |
| `developer.ottospace_repo` | A git checkout of ottospace (`~` is your home). While it is one, the [bug button](#bug-reports) shows. Applies at once | none |
| `capture.ax` | Read an app pane's accessibility tree with its picture. `false`: it is never read for a capture | `true` |
| `windows` | Rules for windows ottospace must leave alone. Each rule selects windows by `app` (the app's name or bundle id), `title`, `identifier` (the app's own name for the window, its `AXIdentifier`: Zoom's main meeting window is `zm.meeting.window.main`; a window without one has empty text: `""` or `*` matches it) and `subrole`; every field given must match, `*` stands for any text, and case does not matter. `"do": "ignore"`: the window never becomes a pane, when its app is hosted or when it opens later, and is never moved. `"do": "manage"` undoes a built-in rule. A new window of a hosted app: `"do": "split"` puts it in a pane of its own even if it is a pop-up (a dialog, a small window; a sheet stays on its window); `"do": "float"` keeps it a free window over its app's pane, as a pop-up, and it never becomes a pane. Without either, a new window floats and is offered in its app's pane ([App panes](#app-panes)): none splits by itself. Your rules come first, but a `split` or `float` rule never moves a window a built-in rule leaves alone (Zoom's share toolbar): only `manage` does. Name an app by its bundle id to be sure. A window is judged once, when it is first seen, on the title it has then; a changed rule applies to windows seen after it | Built in: only standard windows are ever put in a pane, and Zoom's sharing controls, annotation layer and pop-ups are ignored |
| `apps` | The pinned apps: listed first under "Not running" in Apps, and in the picker before anything is typed; choosing one starts it in the background and puts its window in a pane. Each is the name of the app as Finder shows it (`Visual Studio Code` for `Visual Studio Code.app`), looked for in `/Applications`, `~/Applications` and `/System/Applications` (and their `Utilities`), or the path of its bundle (`~/Tools/Lumen.app`). One that is not found is left out of the list and kept in the file. Pin and Unpin in the Apps section write this list: the name if it means that same bundle when read back (the first of that name in those folders, in that order), else its path. A name that is found only once ottospace runs (the app was installed since) is listed from the next scan. (Once you type in the picker it offers every installed app in those folders, these first) | none |
| `apps_hidden` | Apps of the Dock not to suggest in Apps, written as `apps` is. Hide in the Apps section writes this list | none |
| `wheel_to_apps` | The wheel or trackpad over an app pane you are not in enters it, and the scroll reaches the app. `false`: the wheel there does nothing | `true` |
| `calendar_ics` | The [meeting timeline](#meeting-timeline)'s calendar feed: an `https://` link (to a host on the internet, not an address or `localhost`) or a file's path (`/…` or `~/…`). Never shown: a private feed's link holds a secret | none: no timeline |
| `calendar_workday` | The workday the timeline's band spans: `"HH:MM-HH:MM"`, local 24-hour time, the start at least an hour before the end (`"06:00-22:00"`; `24:00` ends the day). `calendar_window_hours`, from before, is retired: the status bar asks to remove it, and the rest applies | `"09:00-17:00"` |
| `calendar_refresh_minutes` | How often the feed is read again, at least 1 | `5` |
| `calendar_notify` | A meeting that starts while you are in no call, or in another call, shows a meeting notice on a toast, a notification and a sound | `true` |
| `calendar_notify_for` | Which meetings alert: `all`, or only those with a Zoom, Meet or Teams `links` | `all` |
| `calendar_email` | Your address among a meeting's attendees in the feed: a meeting you declined is left out. Kept on this machine, never shown. A feed with no attendees (Outlook's published calendars) cannot say who declined: use `calendar_hide_free` | none: nothing is left out |
| `calendar_hide_free` | Leave out entries that keep the time free (Outlook's FREE, `TRANSP:TRANSPARENT`): the band, the section, the alerts and the `calendar` request | `false` |
| `calendar_colors` | Colours by title, first match wins: `[{"match": "1:1", "color": "teal"}]`. `match`: plain text the title holds, case ignored (not a pattern); `color`: `red`, `indigo`, `orange`, `pink`, `lime`, `purple`, `tan` or `teal`. At most 64. A rule wins over a focus block's green | none: colours by kind |
| `city_exclude` | Folders the code city leaves out, by checkout: `{"*": ["archive"], "~/notes": ["old/drafts"]}`. `*`: every checkout; any other key is a checkout's path (`~` for your home). An entry is a folder's name, left out wherever it is, or a path from the checkout's root (`notes/archive`), left out with all under it; no patterns, nothing outside the checkout (no `/` first, no `..`). `!target` keeps a built-in one. At most 256 | `.git`, `node_modules`, `bower_components`, `jspm_packages`, `target`, `dist`, `.next`, `.nuxt`, `.turbo`, `.gradle`, `.venv`, `venv`, `__pycache__`, `.tox`, `Pods` |
| `city_folder_colors` | The code city's colours by folder name, over the built-in ones: `{"notes": "teal", "docs": "red"}`. A name is one folder's (case ignored), a colour `red`, `orange`, `yellow`, `green`, `teal`, `cyan`, `blue`, `indigo`, `violet`, `pink`, `brown` or `slate`. At most 64 | `docs`/`doc` green, `tests`/`test`/`__tests__`/`spec` violet, `e2e` pink, `src`/`lib` blue, `crates`/`packages`/`pkg` teal, `apps` cyan, `scripts`/`tools` slate, `assets` brown, `examples` yellow |
| `suggest_dock_apps` | List the Dock's apps that are not running under "Not running". `false`: only pinned apps are listed there, and the Dock is not read | `true` |
| `features` | [Feature flags](#features), by name: `true` or `false`. A name that is no flag is a mistake, named in a toast with the flags there are; the others still apply | each flag's default |
| `paste_confirm` | Retired: ⌘V never asks. Still read, so a file that has it applies; a toast asks to remove it | |

A mistake in the file (a key that does not exist, a wrong value, a font that is not installed) is
shown in a toast. That key takes its default and the others still apply. If the file is
not a JSON object, every default applies.

## Features

Parts that are not ready to be on for everyone are behind a flag, in `features` in the settings:
`{ "features": { "marks": true } }`. Off, a feature is absent, not greyed out: no rail cell, no
section, its keys do nothing, its commands and menu items are not listed, and its socket requests
answer `feature off: <name>`. What it keeps on disk stays: turned on again, it is all there.
Palette ▸ Features… lists each flag with its name, id, stage, description and whether it is on; choosing one turns it the
other way, writes it to `settings.json` and applies at once.

A flag is temporary and about readiness: it goes when its feature is stable, and a setting of it
then does nothing (a toast says so). A preference is a normal setting.

| Feature | Stage | Default | What it is |
| --- | --- | --- | --- |
| `marks` | Experimental | off | Marks: Name a pane or a tab, and go back to it with a letter |
| `apps_section` | Experimental | off | Apps section: The Dock's apps, pins, Not running and All apps |
| `transcripts` | Experimental | off | Transcripts (⌘⌃T): An app pane's audio, transcribed on this Mac (macOS 26), as lines in the drawer and a Markdown file |
| `slots` | Experimental | off | Slots: A pane that keeps its place and waits for a window you name (⋯ on an app pane's header) |
| `agent_mode` | Experimental | off | Agent mode (⌘⌃V): Let a workspace's task see an app pane, from the robot in its header |
| `multi_monitor` | Experimental | off | Multi-monitor (⌘⇧O): Fold a tab out to its own window on another display |
| `overlay_panels` | Experimental | off | Overlay panels: The sidebar and the drawer float over the panes, moving nothing, and close after use |
| `decisions` | Experimental | off | Decisions (⌘J): A workspace that shows what waits on you, one question at a time |
| `code_city` | Experimental | off | Code city: An empty tab or pane draws its folder (a task's checkout, a workspace's default, a repository's tab) as a 3D city, its files where they were last time |
| `session_waits` | Experimental | off | Waiting sessions: An agent CLI you started in a terminal pane needs you when its turn ends, at once when it asks |
| `projects` | Experimental | off | Projects: The projects in your projects_dir, on the left rail: read, never written |
| `terminal_tasks` | Experimental | off | Terminal sessions as tasks: A live Claude Code or Codex you started in a terminal is listed with the tasks, read-only, until it ends |
| `zoom_out` | Experimental | off | Zoom out (⌘⇧\\): Every tab of the workspace on show at once, as tiles on a grid |
| `decision_sources` | Experimental | off | Decisions from any source: Agents and routines file decisions over the socket (Slack, Jira, a session): they wait to be sorted |
| `city_live_layer` | Experimental | off | Code city on a live layer: The code city drawn straight to a layer of its own under the window's content, a new picture each display refresh (with code_city) |

- `marks` off: no Marks cell or section, ⌘B, ⌘⇧B, ⌘' and ⌘⌥1…9 do nothing, no Mark commands or
  menu items, no ◆ on a marked pane, and `marks`, `mark_add`, `mark_go`, `mark_remove`,
  `mark_rename` and `section` `marks` answer `feature off: marks`. `marks.json` is still read and
  never written; the state's `marks` is `[]`.
- `apps_section` off: no Apps cell or section, and `section` `apps` answers
  `feature off: apps_section`. ⌘⌃E drives the rail's first section. Hosting apps in panes, the
  picker (⌘E, ⌘⇧E) and its installed apps, sneak labels in apps, the wheel's entry, restore and rescue all
  still work. The state's `apps_bar` is empty.
- `transcripts` off: no `Transcribe` in a pane's header, no transcript on the drawer's rail, ⌘⌃T does
  nothing, no Transcribe or Pause command or menu item, and the state's `listen` is null. Turning it off
  ends a transcript that is on. On, an app pane's header offers `Transcribe` on macOS 26 (design:
  [transcripts.md](ai/transcripts.md)).
- `slots` off: no ⋯ in an app pane's header, and a slot, restored or turned off mid-run, is a plain
  app pane of its app that keeps the slot on its record for the flag's return. On, see
  [Slots](#slots) (design: [slots.md](ai/slots.md)).
- `multi_monitor` off: ⌘⇧O does nothing, no Fold Tab item, no tab is out; one out folds back in
  (design: [multi-monitor.md](ai/multi-monitor.md)).
- `overlay_panels` (an experiment, off): the sidebar and the drawer take width from the panes, as
  described elsewhere in this page. On, both float over the panes from their rails instead, with a
  short slide in (none under Reduce Motion) and a shadow: the panes, and every hosted window, lie as
  with both collapsed, so opening either moves nothing. Each is for a quick look or a jump:
  - The sidebar closes on ⌘⇧L or its rail cell, Esc in its list, a click outside it (not on a rail),
    the keyboard leaving it for a pane, or a row that goes somewhere (a task, a repository, a tab,
    a workspace, a mark). Opening it gives its list the keyboard; Esc or ⌘⇧L gives it back to the
    conversation it came from, else to the pane.
  - The drawer folds into its rail on ⌘⇧R, →| or its rail item, a click outside it, or the keyboard
    leaving it for a pane. Esc in a conversation is still the agent's. A capture's draft keeps it open.
  - Both may be open at once, each over its own side; one click outside both closes both.
  - ⌘N's box (and +'s) floats centred over the dimmed panes, not in the drawer (the canvas
    "NewTaskFly"). ↵ starts the task and the box grows into the drawer as its conversation, which
    takes keys at once; ⇧↵, ⌘↵ and Esc fade it; a click outside folds it into the drawer's rail,
    and ⌘⇧R brings it back in the drawer. Under Reduce Motion it goes at once. With nothing open
    at all, the box stays the drawer's; a client's box is the drawer's too. The state's
    `compose_centred` says it.
  - ⌘⇧N's quick note is that box with its pills (see ⌘⇧N in [Keys](#keys)). The state's
    `quick_pills` lists them by kind.
  - Only you open or dismiss one: a client's `section`, ⌘⇧L, ⌘⌃E, ⌘N, a new task or a needs-you
    change opens neither (what it puts in the drawer waits behind the rail), and its dismissal is
    refused ("only the user opens or dismisses a side overlay").
  - An app you are in: its window is above ottospace and may cover part of an open overlay. The
    part that shows takes clicks; opening the sidebar takes the keyboard back and parks the app.
  - Each slides out of its rail and, closed, back into it (none under Reduce Motion). Sliding back
    it takes no click: one there goes to the pane under it.
  - The state's `overlays` gives each one's rect (`sidebar`, `drawer`), also while it slides back;
    null once closed.
- `decisions` off: ⌘J is the next task only, no Decisions command or menu item, and one open
  closes. On, see [Decisions](#decisions) (design: [attention/decisions.md](ai/attention/decisions.md)).
- `code_city` off: every empty place draws the lattice, and no file list is read. On, see
  [Code city](#code-city) (design: [code-city/plan.md](ai/code-city/plan.md)).
- `session_waits` off: a session needs you only through a task adopted from it, once quiet. On,
  every sessions round (5 s with Tasks open, 20 s closed) finds each terminal pane with an agent
  CLI in front of its shell (its program's name) and that CLI's session. The same for every
  runner: it needs you once its file has been quiet `session_quiet_minutes` ("needs you · your
  turn"); a runner that says why it waits needs you at once, with the reason ("needs you ·
  permission", "question", "input"; today Claude Code, by its process record). A task adopted
  from it needs you with it; one no task has counts in the status bar's `needs you`, its Elsewhere
  row's dot is the needs-you colour, and ⌘J, with no task needing you, focuses its pane. Nothing is
  installed and nothing is typed into it (design: [sessions.md](ai/sessions.md#waiting-on-the-user)).
- `projects` off: no Projects cell or section, and `section` `projects` answers
  `feature off: projects`. `projects_dir` is not read; the state's `projects_bar` is off and empty.
  On, see [Projects](#projects) (design: [projects.md](ai/projects.md)).
- `terminal_tasks` off: terminal sessions are Elsewhere's rows only. On (it reads the sessions in
  panes, as `session_waits` does), each live `claude` or `codex` you started in a terminal (in one of ottospace's
  panes, anywhere; elsewhere, in a listed repository while Tasks is open) is a card under Needs you
  or Running, after the tasks, and counts there (`needs you`, ⌘J, the status bar). Live: its pane
  runs it, or its runner's process record holds it (Claude Code), or its file changed in the last
  30 s, or its record says busy, or its sub-agents' files were written lately (Claude Code: then
  never "your turn", however quiet its own file). A click (its card, or its agent in a code city)
  opens it read-only in the drawer where you are: no tab of its own, the space on show stays, and
  it goes when another space is on show. Take over (once idle) makes it a task and goes on with it. Hide takes it out until it asks again in a new turn (answered or at
  work since, then a wait), never within 5 minutes; activity or the wait it had never ends a hide
  (kept in `sessions-hidden.json`). "N hidden · Show" lists the hidden ones, dim, each with Unhide;
  "N hidden · Hide" folds them away again. Ended, it stays dim 5 minutes,
  counted nowhere, with Take over when it can be gone on with here. The Project view shows no
  cards yet: there they stay Elsewhere's rows.
  Design note: pull request #423.

### Decisions

With `decisions` on, ⌘J (Tasks ▸ Decisions, the palette) opens Decisions (with nothing waiting, it
goes to the next task, as ⌘J does with the flag off): a workspace of one pane, a card, for each question a task's agent waits on
(a question card, `AskUserQuestion`). The card on show stays first; then P1 first, then the one
waiting longest. Its place on the top row reads "Decisions · 2 of 5" over a thin bar. On the card: 1–9 pick, ↑↓ move, space picks, ↵ sends (the agent
resumes), S skips it (shown next time), L puts its task off for an
hour (snoozed), Esc closes. Other… is typed in the drawer. ⌘L talks the card over in the drawer,
on its task; the right rail shows it. Panes you add close when you answer, skip or put off the
card. A card answered elsewhere leaves; after the last, Decisions closes and the tab you came from is back. Never kept.
Only you open, answer, skip or close it.

With `decision_sources` on too, what agents and routines file (`decision_file`) waits in triage:
after the agents' questions, one card, "Sort 3 new items" (the status bar says `· 3 to sort`
beside needs you; nothing pings). ↵ sorts them, S leaves them for another run. Each item, the
first filed first, shows where it came from, what came in, what the filer suggests and claims
(someone waits, a deadline: not counted until you press U). One key each: ↵ keeps it (under the
project it suggests), T takes it now (for today), L puts it off a day, N not mine, D dismisses;
C flips core or ancillary, Z takes the last back, P lets its scope skip triage from now: the
Slack channel, Jira project, repository or routine its key names, shown on the card before you
press it (never a whole source; kept in `~/.ottospace/decisions/trust.json`; Z takes it back, its
items back in triage). A sorted item whose filer then changes what it says (not only its time)
comes back to triage, says what changed, and keeps nothing you accepted. The look is
provisional. Only you sort.

The week's focus (`decision_sources`): Tasks ▸ This Week's Focus… (the palette too) lists the
Projects entries; tick up to four (↑↓ move, space ticks, ↵ sets, Esc closes, setting nothing).
Each is an area of the week, named by the project's title (an area set before keeps its own name
and links); what is of it (a task's `#project` or `>repo`) comes first in Decisions, in Needs you
and for ⌘J. Each Decisions card says why it is where it is ("Search speed · an agent waits · P2 ·
waiting 5 min"; its hover gives the sum). Kept in `~/.ottospace/focus.json` by ISO week (the last twelve); a new week starts with
none. Only you set it; agents read it with `week_focus`. An agent may propose it (`focus_set`,
from your weekly note, say): Decisions shows it as one card after the agents' questions, exactly as
it would be set, what changes and where it was read; ↵ sets it, E opens the editor with it ticked,
N keeps the focus as it is (the same proposal is not shown again this week). A new week asks once:
"Keep last week's focus?" (↵ keep, E edit, N none). A card that changes while on show (a newer
proposal, a filer's update, another card in its place) says so, and its next key only says you saw it.

### Projects

With `projects` on and `projects_dir` set, the left rail has a Projects cell under Repos. Its list
is the folder's projects: each folder in it (its note is `README.md`, else `index.md`, else its
only or newest `.md`) and each lone `.md` file. A row shows the title (the note's first heading,
else the name) and how long ago it was last used (changed, or opened here). A click (or ↵) opens
its note read-only in a file viewer beside the focused pane, ⇧ below; a project with no note says
"no note" and opens nothing. Only you open one: a client's click, ↵ or label opens nothing. Drag a row up to place it: placed projects come first, in your order,
above a thin line; the rest follow, most recently used first. Esc puts a drag back. The order and
the open times are kept in `~/.ottospace/projects.json`, never in your notes. ottospace only reads
the folder, and only while the
section is open (on opening, then every 10 s): no link is followed, hidden names are skipped, at
most 500 projects, 200 entries a folder, and 256 KB of a note. Nothing in it is written. The
state's `projects_bar` gives each row's kind and times, never a title or path; an opened note's
viewer pane has its path, as any viewer's (design:
[projects.md](ai/projects.md)).

### Code city

With `code_city` on, an empty place (see [Empty places](#empty-places)) draws the city of its
folder, rendered on the GPU, where the lattice was. Its folder is the nearest of: the tab's own (a
repository's tab, opened from Repos or `open_repo`, keeps its checkout after its last pane
closes), its space's (a task's checkout, a workspace's default worktree). Only a checkout listed
in Repos; Home has none and keeps its lattice, and so does a pane under 320 × 220 points. Places
of one folder and size share one picture, and a folder's files are read once for all of them.
In the city a folder is a district, a plinth standing on its parent's, its top edge lit,
a street apart from its neighbours and a shade lighter per tier (a folder with one
child is none of its own: `src/main/java` is one district), a file a building on it, of glass (its faces see-through, its edges whole), in its folder's
colour, its ground sized by its size class (1 up to 2 KB, doubling to 128 KB) and its
height by its length. A file is where it was the last time: the same files give
the same plan, an edit that keeps a file's size class moves nothing, and a new file most times
moves only its own folder
(design and numbers: [code-city/plan.md](ai/code-city/plan.md)). A building's colour is its
nearest folder's with a named colour (`city_folder_colors`, over built-in ones: `docs` green,
`tests` violet, `src` blue, `crates` teal and more), else its top-level folder's own, from its
name alone; a file in the root is grey. So `crates/a/tests/x.rs` is a test's colour, and adding
a file or a folder changes no other building's colour. A lit building keeps its task's colour.

- The file an agent reads or edits now is tinted in its task's colour (in a task's space, that
  task's agent; elsewhere every task working in that checkout, as the Repos dots count them): while
  the call runs, and after it while the agent still works and has run no other call. A shell
  command lights none. The call names only the last two parts of a path, so two files that share
  them are both lit.
- Each agent at work in the checkout floats over its city as an icon: a task's badge (its letters
  in its colour), or a session's runner glyph for a `claude` or `codex` you started in a terminal.
  It shows only while a city is on show (an empty tab or pane of a checkout listed in Repos, with
  `code_city` on), and only while the agent works: a task's turn runs, or its session is active
  (its file changed lately). In a task's own space, only that task's agent; elsewhere every task
  whose checkout it is, and every session at work there that no task has. A task whose agent
  runs in a terminal (Codex, or `conversation_view: "terminal"`) shows by its session, read from
  its file (so `sessions` must be on and `session_dirs` must hold its config folder). The icon
  stands over the file the agent reads or edits now, a cone of its colour on it (in a chat, the
  last file it read or edited until its next call); at a shell line, or when the file is unknown
  or its name matches two buildings, hovering over the city's middle as high as a beam (above
  the hints if they cover it, else under). A shell line that names a path in the checkout (after
  its `cd` there: `cd <checkout> && rg loop src/ui`) stands it over that file or folder, read as
  words only, never run. While a call of it runs it pulses: the pool on its building, or a halo
  round the icon (none under Reduce Motion). A terminal session's line is not read: it hovers.
  What it reads and searches plays once over the picture: a read (`Read`, or `cat`, `head`, `sed`
  … in a shell line) flashes its building; a search (`Grep`, `Glob`, or `rg`, `grep`, `find` …)
  sweeps a band of light across the folders it names (the whole city when it names none), and the
  files its result names stay lit a moment. Calls close together play as one; none under Reduce
  Motion. Codex's calls carry no result, so its searches light nothing after the sweep. The
  sub-agents a chat's agent runs (its `Agent` calls not ended) are small squares under its icon,
  eight at most, then `+n`; a terminal session's are not shown yet.
  A `claude` or `codex` the user runs in one of ottospace's terminals, working in the checkout,
  shows by its process alone (no setting, no session file read): its runner's glyph, hovering,
  until its program ends or its pane closes. With `sessions` on, its session shows instead. An icon with no room clear of the hints is not drawn.
- The files are what `git ls-files` lists there (tracked, and new ones not ignored), less the
  folders `city_exclude` leaves out and built-in ones (`node_modules`, `target`, `dist`, …):
  dropped before the layout, they take no room, move nothing and count toward no limit. Read off
  the main thread with the status bar's hardened git, only in a checkout listed in Repos, when
  the tab comes on show and when the lit files change. No timer. Over 600,000 files, 64 MB of
  names or 60 seconds: no city, the lattice stays. A read of a tab that closes stops.
- Level of detail: a folder whose files would be under about 2 px on screen stands as one block
  (as tall as its files on average, lit if an agent is at one of them); its files grow out of it
  as the camera nears (past about 3 px), at once under Reduce Motion. Turning the camera changes
  none; nothing moves on the plan.
- It takes no click and no key, and nothing in it moves: a still picture, made again when the
  files, the lit files or the tab's size change. Districts and files are named as a map is:
  each name centred on its plinth (a folder's, in capitals, once the plinth on screen is wider
  than the name) or on a file's top; a thing from 40 points to 45% of the view on screen, the
  largest first, at most 50, none over another or behind the tab's hints. Coming nearer names
  deeper folders, then files. No path or name is in the socket's state or the logs.
- Pointing at a building shows a beam: a thin line straight up from its top to a dot, with the
  file's path beside the dot (cut in the middle if long); at a district, its folder's. A lit
  building has a steady, thinner beam in its task's colour. Over the hints, the hints win, and
  no beam is drawn over them. Hovering draws, and renders nothing again.
- Its camera is yours to move, your pointer's only: drag with the middle button held (the
  wheel, pressed) to pan: across, the ground under the pointer follows it; up and down, the
  city slides straight up or down in the place as far (half the place's height at most); the mouse wheel zooms about the point under the pointer; a left drag
  (past a click: under 4 points is still a click, which focuses the pane) turns it round the
  point it looks at. On a trackpad, two-finger scroll pans, ⌥ with it turns, ⌘ with it zooms, and a pinch
  zooms about the pointer (ottospace hears it itself: GPUI 0.2.2 delivers none; not yet in a tab
  folded out to another display, nor under the key reference, a box or a side overlay). Esc during a drag puts it back where the drag began; a
  moved camera shows Reset view at the place's corner, which brings back the fitted view (from
  above a corner, 38° down, the city's middle 0.6 of the way down the place, the room over it for
  beams, names and agents), eased (at once under Reduce Motion). Pitch stays between 12° and 80°, zoom between a building filling the tab and
  the whole city, the point looked at over the city. Each place that shows a city (a tab, a tab
  folded out, an empty pane) has its own camera: turning one turns no other. A new place starts
  from the checkout's camera last used, and that one is kept across closing the tab and a restart
  (in `layout.json`); a changed file list keeps it. Over the hints,
  the hints win. A moved camera cuts no building down for the lit ones (those stay raised and
  coloured).
- No GPU, or a render that fails: the lattice.

### Displays

With `multi_monitor` on, ⌘⇧O (View ▸ Fold Tab Out or In, the palette) folds the tab on show out
to its own window on another display: the next one after the main window's, left to right (the
one it was on before, if free); with two displays a second fold swaps (the tab coming back in shows in
the main window if it is of the same workspace; else the main window stays on its workspace). The keyboard goes with
it. The main window (the one ottospace started with: Home, the top row, the left sidebar) shows
the tab's neighbour; the tab keeps its place in its workspace's row, marked out, with Fold back in
where its × is. A click on it goes to its window. The only tab of its workspace leaves an empty tab
of that workspace in its place (its empty state); folding back in removes that one if it is still
empty and not renamed, and the main window shows the tab again.

- The window out has a "From" strip (its workspace, the tab, Fold back in) and its workspace's
  rail on its outer edge: its tasks (a click shows one in the main window, whose drawer holds the
  conversation), + for a new task in that workspace, the transcript. One drawer per conversation:
  the main window's.
- ⌘T, ⌘1–9 and ⌘} ⌘{ typed there act on the main window's row; ⌘⇧W closes the tab out. Its red
  button folds it back in; only the main window's quits.
- Its green button fills its own display, as the main window's fills the main one's; ⌃⌘F fills the
  window with the keyboard. Each window's is its own; folding back in ends it.
- Closing a temporary workspace folds its tab out back in, and it cannot fold out while closing; its Undo brings it back as a tab (an empty stand-in tab goes).
- Unplugging the display folds the tab back in; plugging the same display in again (by its UUID)
  takes it back, unless the main window shows it then. Folded in by you, it stays in.
- Not yet: a tab holding an app pane stays in the main window (a notice says so), as does a
  capture of a pane out; the focus arrows stop at a window's edge; a tab out folds in at the next
  start.
- Only you fold a tab out or in: a client's ⌘⇧O is refused. The state's `displays`, `windows` and
  `out` say what is where.
- `zoom_out` off: ⌘⇧\\ does nothing, no Zoom Out or In item, and an open overview closes. On, see
  [Zoom out](#zoom-out) (design: [zoom-out.md](ai/zoom-out.md)).
- `decision_sources` off: `decision_file`, `decision_withdraw` and `decision` answer `feature off:
  decision_sources`, the state's `filed` is null; decisions kept from before are still read, and
  expire and go on time. Design: [decision-sources.md](ai/attention/decision-sources.md).
- `agent_mode` off: no robot in an app pane's header, ⌘⌃V does nothing, no Agent Mode commands,
  the grants are kept as they are (turning it on again raises none), and every read of an app pane by a client (`app_read`,
  `app_look`, `ax_tree`) answers `feature off: agent_mode` (design: [agent-mode.md](ai/agent-mode.md)).
  On: an app pane's header has a robot with its level (`Off`, `See`, `Act`, a caret), dimmed where
  no task works (a level picked there waits, and the first task you start there is offered it in
  the header: only your click gives it; a client's task gets nothing; `state.agent_mode.ahead`); See in the app's blue with the task's tip, See and act in amber; the pane's
  header line in the level's colour. Only the task the pane is granted to (its agent's pass) may `app_read` its tree or
  `app_look` at its picture (a PNG at most 1600 px, 0600, in `materials/agent-<pid>-<task>-<word>/`,
  the last 10 kept, gone when the level goes below See and at the next start; none while the pane is live), at See or more, one read and one look a second; `ax_tree` is
  `app_read`'s older name. Every use and refusal is a row in that task's conversation and a spoke
  of the dial's Apps station. A pane that moves to another task's workspace, or whose task ends,
  is Off; grants are not kept over a restart. See and act presses nothing yet (stage (c)).

### Zoom out

With `zoom_out` on, ⌘⇧\\ (View ▸ Zoom Out or In, the palette) shows every tab of the workspace or
task on show as a tile on a grid over the panes area, in tab order (⌘1…⌘9's), with room between: one
tile, 2 × 2 for four, three columns for seven. Each tile is the tab as it lies: its panes in their
places, a terminal's screen live and small, an app pane its window's last picture (the one its
parked pane already shows: nothing new is captured), any other pane its name. Above each, its
number and name; the tab you came from says "you were here", the selected tile has a ring.

- It is a canvas. A two-finger scroll pans it (it follows the fingers), as does a drag of its empty
  ground; ⌘ with a two-finger scroll, or a mouse wheel, zooms about the pointer (the point under it
  stays); ⌘= and ⌘- zoom about the middle; ⌘0 fits every tile again. From one tab filling the area
  out to a tab a twelfth of it (a 4 × 4 workspace is then a third of the area wide); it never pans
  every tile out of view. A trackpad pinch zooms about the pointer too. The camera is kept per
  workspace or task, also across a restart (`layout.json`): ⌘⇧\\ opens where you left it, fitted
  the first time and again when its tabs are not the ones of the last overview (one added, closed
  or moved since).
- Drag a tile to place it: it lands where you drop it, its corner on a quarter-tile grid, and
  stays there (also across a restart). The others keep their own grid cells (one a placed tile
  covers takes the next free cell); a new tab takes its own. The arrows go to the nearest tile that
  way. Tidy (T, or the button at the canvas's top right, shown while a tile is placed) puts
  every tile back on the grid. A click on a tile still goes in. Only your own drag places.
- A tile shows its terminals' text from 360 points wide, their rows as bars down to 120, and under
  that the tab's name alone.
- Arrows or H J K L move the selection, and the canvas follows it into view; ↵ or a click goes into that tab; Esc or ⌘⇧\\ again goes
  back to the tab you came from. ⌘; labels the tiles: a label's letters go in. No other key reaches a
  terminal under it (⌃C does nothing); ⌘ shortcuts run as ever. In an app pane the keys are the
  app's: take the keyboard back first (F18), then ⌘⇧\\.
- The zoom plays both ways: the tab shrinks into its tile as the others fade in, and the chosen tile
  grows back to fill the area. It never holds a key up: the view has changed before it plays, and on
  the way back in it takes no click. Under Reduce Motion it is a cut.
- An app you are in is parked as it opens (its picture is in its tile, no window floats over the
  tiles), and is live again when you go back to the tab you came from. ↵ into another tab, as ⌘1:
  no app is live there until you click one.
- A tab out on another display (`multi_monitor`) is a tile marked out; ↵ goes to its window.
- Any other key or click of yours (⌘T, ⌘1, a rail) ends it and does what it does. On Home there is
  nothing to zoom out to: a notice says so.
- Only you zoom: a client's ⌘⇧\\, arrows, ↵ or click is refused ("only the user zooms out or in"), and
  while you are zoomed out a client's request that would take the keyboard is refused too. A
  client's other requests run underneath (a split redraws its tile).
- The state's `state.zoom` (`from`, `selected`, tab ids, and `camera`: `scale` and `center`) and `zoom_tiles` (each tile as drawn, also
  while zooming back in); `counts.zoom_frames`: the frames the zoom asked for, none at rest.

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
ottospace ctl '{"cmd":"decision_file","source":"jira","key":"jira:ABC-1","question":"Take ABC-1 this week?"}'  # decision_sources
ottospace ctl '{"cmd":"sneak"}'                      # labels on ottospace's controls: `sneak.targets`
ottospace ctl '{"cmd":"key","keys":"a"}'             # type a label: that control acts as if clicked
ottospace ctl '{"cmd":"state"}'
ottospace ctl '{"cmd":"trace"}'                      # the last 2,000 events: names, times and ids, never text
```

With the `decision_sources` [flag](#features) on, an agent or a routine files a decision for you
(`decision_file`): a question, its options, why, evidence, and its own suggestions. One key per
thread (`slack:C0MADEUP:1.0`): filing it again updates it. It waits for you; its words are kept in
`~/.ottospace/decisions/<id>.json` (0600), deleted two weeks after it closes, and given to a client
only by `decision`, labelled untrusted. At most 20 open per source, 100 in all, 30 filings an hour
per source; a decision expires after a week, or when the task it was asked for is done. See the
guide (`ottospace skill`) and [decision-sources.md](ai/attention/decision-sources.md).

`OTTOSPACE_TRACE=1` also writes the trace to `~/.ottospace/trace.jsonl` when the app panics or gets
`kill -USR1`.

### The log

`<home>/log` (`~/.ottospace/log`, or in `OTTOSPACE_HOME`) keeps what happened to the app, to read after a problem: one JSON line an
event, in local time, e.g. `{"at":"2026-11-16T17:51:03-08:00","ev":"displays","count":1,"sizes":["1728x1117"]}`.
Only you can read it (0600). At 1 MB it becomes `log.1` and the one before goes: 2 MB at most.

| Event | Fields |
|---|---|
| `start` | `pid`, `version`, `commit`, `hosting`, `displays` (`count`, `sizes`) |
| `quit` | `hosted`: windows given back |
| `displays` | The displays changed (one plugged in or taken away): `count`, `sizes` in points |
| `window_display` | The window is on another display: its number and size |
| `stall` | The window had something to draw for `stalled_ms` and drew nothing; ottospace has it draw again. `n`: stalls in a row (a minute with none starts over), written the 1st, 2nd, 4th, 8th.. |
| `redrawn` | Frames came again after a written stall: `kicks`, `lasted_ms`, its `n` |
| `host_failed` | A window could not be hosted: its number |
| `give_back_failed` | A hosted window could not be put back on a screen: its number |
| `slot_miss` | A waiting slot offers nothing, and why changed: `pane`, `bundle`, `why` (each reason's count: `rules`, `no_id`, `other_id`, `other_title`, `other_kind`, `other_display`, `in_pane`, `passed_over`, `bigger_pane`, `other_slot`). Never a title or id |
| `place` | A new window of a hosted app was judged: `bundle`, `subrole`, `modal`, `sheet`, `size`, `pane` (its app pane's size), `min_width` (the least width that pane's window kept), `place` (`split`, `offer`, `float`, `leave`), `why` (`no_slot`, `dialog`, `modal`, `tiny`, `sheet`, `kind`, `rule`) and `rule` (`windows[2]: manage`, `built-in rule 3: ignore`, or null). Never the title |
| `pick_list` | The picker listed an app's windows (⌘E, then the app): `bundle`, `listed` (how many) and `skipped`, each window it left out with `why` (`minimised`, `sheet`, `tiny`, `no_id`, or the rule: `built-in rule 0: ignore`) and `subrole` (an `AX` word, else `other`). Never a title |
| `gpui` | An error or warning of GPUI's macOS layer: `level`, `src` (`gpui-0.2.2/src/platform/mac/window.rs:489`), `said` only when fixed words and a code (`could not create display link, code: -6661`), and `n`: one from the same place is written its 1st, 2nd, 4th, 8th.. time |
| `panic` | `src` (`crate/src/file.rs:line`) and `thread`; never the message |
| `listen_audio` | A transcript's sound started: the app's `bundle`, `pids` the stream hears (the app and the helpers it started that ScreenCaptureKit offers, of its own bundle id or one under it), whether it or a helper was `playing`, and `unoffered`: helpers playing that SCK does not offer, whose sound is not heard |
| `listen_audio_filter` | The app's helpers changed while transcribing (looked at every 5 s): `bundle`, `pids` heard now |
| `listen_counts` | Every 10 s while transcribing: `bundle`, `buffers` of sound the stream gave, `loud` (of those, with sound), `lines` the recogniser finished |
| `listen_quiet` | No sound 10 s after a start, from an app that was playing: `bundle`, `pids`, `buffers` (none: SCK sent nothing) |

Never in it: terminal or transcript text, titles, meetings, passes, paths or file contents. Each event
keeps only the fields listed here.
The same lines go to the unified log, subsystem `dev.ottospace` (in zsh, `log` is a builtin, so
name `/usr/bin/log`):

```sh
/usr/bin/log show --predicate 'subsystem == "dev.ottospace"' --last 1d
```

A test run writes neither: the file only in a folder it names with `OTTOSPACE_HOME`.

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

### MCP server

`ottospace mcp` is a stdio MCP server over the same socket: read-only tools `workspace`,
`app_read`, `app_look`, `capture`, `wait`, `task_brief` and `guide`, each one socket request on the
socket's terms (a task's agent's pass, agent mode's grants; with a task's pass, `capture` and
`wait` read only that task's workspace's terminals). Its instructions are that guidance,
with its tools. ottospace gives it to the agent of every task it starts, for that run
only and never in your CLI's config files: Claude Code by `--mcp-config` with its tools in
`--allowedTools` (no prompt for them; the server is named `ottospace_task` there), Codex by `-c mcp_servers.ottospace_task.*` with
`default_tools_approval_mode = "writes"` (its read-only tools unasked), both in either view.
To use it from a CLI you start yourself, add it by hand, e.g.
`claude mcp add ottospace -- ottospace mcp`; with no task's pass it reads no app pane. Design:
[agent-awareness.md](ai/agent-awareness.md), section 5.

## How app windows are hosted

macOS cannot reparent another process's window, so ottospace fakes embedding with the Accessibility
(AX) API and ScreenCaptureKit (SCK). An inactive app window is parked off-screen with one pixel on
a display, and its pane shows an SCK stream of it. Clicking the pane moves the real window over it
and replays the click, so input is native. The wheel over a parked pane enters it the same way,
and the scroll is sent on to the window. A pane shows one title bar: the stream leaves out the
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
main screen, and a toast says so. If an app replaced its window while hosted, its windows
left in the parking spot come back instead. ottospace only moves and sizes windows: it never
minimises or hides them, nor moves them to another Space.

To bring back any window stranded off screen, whatever left it there: `ottospace rescue` in a
terminal (no app need run), or Developer ▸ Bring All Windows Back (also in the ⌘⇧P palette). Each
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
- [code-city/plan.md](ai/code-city/plan.md): the atlas, a map where a thing keeps its place, and the code city on it. Built: the layout, and the city behind a task's empty tab, behind `code_city` (see [Code city](#code-city)).
- [file-viewer.md](ai/file-viewer.md): the read-only file viewer pane, and what comes after it (the markdown pass, the explorer, comments).
- [bug-reporter.md](ai/bug-reporter.md): marking a bug in ottospace's own window and handing it to an agent.
- [fill-display.md](ai/fill-display.md): the window over the menu bar and the Dock (⌃⌘F, the green button): what macOS does, measured, and what is left to check by hand.
- [sneak-apps.md](ai/sneak-apps.md): sneak labels that press controls inside the focused app pane, and why agents still cannot.
- [multi-monitor.md](ai/1-initial-brainstorm/multi-monitor.md): options for more than one display, and a staged recommendation. Nothing measured yet.
- `background/`: mockup images.

[capture.md](ai/capture.md): the capture key, annotate mode, what a capture stores, and how a brief quotes it.
[marks.md](ai/marks.md): marks, what one records, and how they sit beside session restore, pins and tasks.
[projects.md](ai/projects.md): the Projects section, what a project is on disk, the order rule, the slices, and a proposal for progress and next actions.
[workbook.md](ai/workbook.md): workspaces (there still "workbooks") beside tasks, marks and session restore; stages 1 and 2 as built, and what is later.
[session-restore.md](ai/session-restore.md): the workspace back after quitting, in layers; layer 1, the layout file, as built.
[terminal-persistence.md](ai/terminal-persistence.md): terminals and agents that outlive a quit or an update, held outside the app; with session restore, tasks come back as they were.
[task-conversation.md](ai/task-conversation.md): a task's agent in a right drawer, the three zones, how an agent is started; stage 1 as built (the agent's terminal in the drawer), and what is later.
[sessions-as-tasks.md](ai/sessions-as-tasks.md): a `claude` or `codex` started in a terminal as a read-only card among tasks: live, aged out, hidden until it needs you. Design note, not built.

[routines.md](ai/routines.md): routines and agents from ottobot, in stages, toward ottospace subsuming it.

[e2e-reliability.md](ai/e2e-reliability.md): replies that wait for the frame that draws them, failure folders, the trace, and repeated runs to find flaky tests.

[agent-dial.md](ai/agent-dial.md): the agent dial over a native conversation: its model, the station table.

[agent-runners.md](ai/agent-runners.md): a native (non-terminal) agent conversation: driving Claude Code and Codex in their structured modes, permission prompts, trust rules. Not wired in yet.

[agent-mode.md](ai/agent-mode.md): a per-pane switch that lets a task See, or See and act on, a hosted app's window; who is asking, what each level grants, what Off closes. Not built.

[agent-awareness.md](ai/agent-awareness.md): how a task's agent learns ottospace is there (it does not, today), sees the workspace through it instead of screenshots, See before a task exists, what agent mode cannot stop, and `ottospace mcp`, a shim over the socket for the agents ottospace starts. Proposed.

[meeting-timeline.md](ai/meeting-timeline.md): the meeting timeline as built, how a call is detected, the network decision, and what the calendar reader does not follow.

[transcripts.md](ai/transcripts.md): an app's audio transcribed on the Mac, live, started and stopped by the user alone. Slice 1 is in (behind `transcripts`): the model, ⌘⌃T, the indicator and the drawer's lines, fed only by a test fixture. Then the file in `transcripts_dir`, heard lines handed over on a fixed tick, and the level meter with its label (Hearing speech, Listening, No sound for 30 s), drawn only. Then the app's real audio (ScreenCaptureKit) and Apple's on-device recogniser, the model's download, the menu bar's dot and the start reminder.

[slots.md](ai/slots.md): slots, panes that wait for a window (one window selector for slots, `windows` rules and transcripts' `meeting`), offer it, and empty when it closes. In: the selector (`windows` rules take `identifier`) and, behind `slots`, the model (its states, who may, the record in `layout.json`) and slice 3 on screen: ⋯ and its dialog, a slot's pane, new windows a slot selects. Next: the Zoom measurements, Launch and Close the pane.

[zoom-out.md](ai/zoom-out.md): every tab of a workspace at once (⌘⇧\\), in three slices: the overview on a grid, free placement, the canvas. Not built.

[multi-monitor.md](ai/multi-monitor.md): a tab folded out to another display (⌘⇧O), focus arrows across displays, unplug folds it back in; behind `multi_monitor`. Built: the model, ⌘⇧O and the window out (see [Displays](#displays)); its own drawer and the arrows across, not yet.

[dock-strip.md](ai/dock-strip.md): a strip at the workspace's top or bottom edge for an app's small accessory window (Zoom's share control bar), sized by the window, not a ratio; options weighed, slices. Not built.

[repos.md](ai/repos.md): the Repos section: the opt-in list and its order, groups from older settings, folding, the default folder's dot, and how + makes a worktree (the git it runs, what it refuses).

[pick-and-app-profiles.md](ai/pick-and-app-profiles.md): Pick on screen (task context picked in panes), the apps' extended accessibility mode on demand with its header icon, and app profiles as data; the spike's measurements and the slices. Not built.

[attention/plan.md](ai/attention/plan.md): human attention as the scarcest resource: a Decisions session with visual aids, agents and roles, a secretary agent, a machine-wide stray-sessions inbox, Slack ping to draft reply, a quick note, focus mode; what exists on main for each. Not built.

[attention/decision-sources.md](ai/attention/decision-sources.md): decisions from any source (agents, terminal sessions, Slack, Jira, PR review, calendar, routines) in one queue: the record, filing over the socket, triage as a decision (sorted in one key each), priority from the week's four focus areas and progress against them, the acts an answer takes, trust. Design note, not built.

[`docs/ai/backlog.md`](ai/backlog.md) is an older backlog. `docs/ideas/` holds the owner's own
notes.
