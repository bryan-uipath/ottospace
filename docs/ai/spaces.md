# Spaces: two levels of tabs

The hierarchy is **space → tabs → panes**. Replaces "a tab may belong to a task", the right
rail's cells and the drawer's go word (`docs/ai/ux-simplification.md` §1.7, §3).

## Workspace is the one word

The owner: "the workbook/workspaces are very confusing. can we just change 'workbook' to
'workspace' and let it create workspaces that aren't associated with any task or worktree (to
start)". A workbook was already "a saved, named workspace" (`workbook.md`). So there is one
thing, a **workspace**: named, kept between runs, on the top row while open, listed in the
Workspaces section whether open or not.

Built (stage 1):

- Everything the user reads says workspace. In the code, the requests, the state, the control
  ids, the setting and the folder it is still `workbook` (`Owner::Workbook`, `workbooks`,
  `rail:workbook`, `~/.ottospace/workbooks`): files kept so far load unchanged. The rename
  inside is a follow-up.
- **+ makes a new one** (`WorkbookNew`): the top row's + and the section's + (and the empty
  section's "New workspace"). Named "Workspace 2", "Workspace 3": the lowest number no
  workspace has. Its space goes after the selected one with one empty tab, selected. It takes
  and records no tab: its file has no tabs until one is saved into it. Twice makes two. While
  a delete can still be taken back, + takes neither that workspace's name nor its file name:
  the undo brings it back as it was. The
  user's alone, as saving is. It is temporary (below, "Temporary workspaces"): no file yet.
- **New task** is ⌘N and the Tasks section's +. The top row's + no longer opens the box.
- **× keeps it listed.** Hiding a workspace takes it off the row with its tabs and panes as they
  are; its row in the section brings it back. Its last tab closed: off the row, still listed;
  opened again it shows its saved tabs, or one empty tab.
- **Saving** stays, and is never reached from +: ⌘⇧S saves the tab on show into the workspace
  whose space it is in (a tab of it already: updated in place), else as a new workspace of that
  tab; `a` on a row in list mode or File ▸ Add Tab to Workspace…; Save All Tabs as Workspace. A tab made in a workspace is kept by
  session restore (`layout.json`) while its space is open or hidden, and in the workspace's own
  file only once saved.
- **Home** replaced the first space, "Workspace" (below, "Home").

## Temporary workspaces (built)

The owner: "making workspaces ephemeral if they don't have a task or a repo associated with them.
ie, i opened a workspace and it's 'Workspace 8' because i had opened a bunch of workspaces in a
previous session. if we close that kind of workspace - no task or repo associated - it doesn't
need to persist by default. maybe we need a way to differentiate persisted vs not workspaces".

- **Kept or temporary** (`Workbook::kept`). A workspace is kept when it has a task bound, a
  default worktree, the user renamed it, saved a tab into it (⌘⇧S, Add Tab, Save All), or chose
  Keep (`WorkbookKeep`: its row's …, its item's … on the top row (`MenuOf::Space`, Keep alone), File ▸
  Keep Workspace, the palette). Else it is temporary:
  one made with +, or for what the user opened on Home, or for a client's tab with nowhere to go.
  It becomes kept by itself the moment it gains a task, a worktree or a name; a kept one never
  turns temporary (`bind`, `set_worktree` and a rename set the flag, nothing clears it).
- **No file.** The shell writes only kept workspaces (`Workbooks::stored`). A temporary one open
  or hidden at quit is in `layout.json` (its space's `name`, beside its `workbook` id), and
  comes back temporary (`temporary_workspace`). A build before this one reads such a space as a
  workspace not kept: its tabs go to the workspace selected last.
- **Closing discards.** The user's step that takes a temporary workspace off the row (×, ⌘⌃W,
  its last tab closed or moved away) closes it (`settle_closing`, after `act`): it leaves the list
  at once, its space hidden with its tabs, and the notice bar offers `Workspace closed` with Undo
  for `NOTICE_MS` (#208's offer, `Change::Closed`). The user's Undo brings it back on the row,
  selected, as it was. Its time up (a `Tick`; × on the bar sets it up now): its tabs close, their
  terminals and windows with them, and it is gone. Only time passing ends one, so no other step
  ends a pane by it. A tab of it selected meanwhile (a mark, a client's `select_tab`) puts it back
  on the row, temporary still, and the offer goes. A second close while one is offered takes the
  bar; the first still goes at its own time.
- **Who.** Only the user's step closes one or keeps one: a client's `hide_space` or `close_tab`
  leaves it listed (it goes at quit, having no file, unless it is on the row then), and its
  `WorkbookKeep` is refused (`USERS_ALONE`).
- **Numbering.** + names a new one with the lowest free number among the listed workspaces and
  one whose Undo is offered (`free_name`): once closed ones go, "Workspace 2" comes again.
- **Look.** Its name is muted on the top row and in the section, its tip "Temporary: closes
  without saving · ⋯ Keep". The state says `spaces[].kept` (false only for a temporary
  workspace) and `workbook_offer` `closed`.
- **Migration.** Every file read is kept: a workspace saved before this rule was one the user
  meant to keep, whatever it holds (deleting stray "Workspace 8"s is one click each). The upgrade
  of an older layout's first space ("Workspace") is kept too.
- Property rules: no step but the user's keeps or closes a temporary workspace or takes one off
  the list (the files read again aside); no temporary workspace is written; a closing one is not
  listed, not on the row, not kept and not in `layout.json`, nor are its tabs.

## Home (built)

The owner: Home is the first item of the top row, a house with no word and no ×. It never holds a
pane: what you open there gets its own workspace.

- **Home** (`Owner::Home`) is always first and never hidden, in no file and not in the section.
  ⌘⌃1, "Go to Home" in the palette and the View menu select it. It holds one fixed empty tab of
  its own (`HOME_TAB`), its screen: the home screen (tasks that need you, the nodes, the key
  caps). The state leaves that tab out of `tabs` and says `active: null` there.
- **The user's act on Home makes a workspace.** A pane put there, ⌘T, a Repos click: a NEW
  workspace, selected, named as + names one ("Workspace 2"), or for the folder a Repos click
  opened. Listed like one made with +: temporary, but for a Repos click's (its worktree keeps it).
- **⌘N on Home**: the task gets a space of its own, as before. In a workspace it joins.
- **Anything else goes to the workspace selected last**: a client's pane or tab, a stray tab (its
  task dismissed, its workspace deleted, a session's after a restart), a tab of no space read
  from `layout.json`. Else the first workspace on the row; else the last selected that is still
  kept, though closed (it comes back on the row); else one is made. So a client makes a
  workspace file only while none is kept. A stray tab that holds nothing is closed.
- Under the user's typing (chat or new-task box) a client's pane on Home leaves Home on show.
- **How it is modelled**: `Model::settle_home` (`model/spaces.rs`), after every step, moves
  what is in Home to a workspace by the rules above; `last_spaces` is the selection history.
  The undo of a dismissal finds its tabs where they went (`unlinked` follows them).
- **Upgrade**: an older `layout.json` whose first space held a pane: its tabs become a kept
  workspace named "Workspace", where the first space was on the row. One of empty tabs only is
  just Home.
- Property rules: Home holds no pane and only its own tab, after any step from any origin.

### Home's screens (built)

The owner, on the design canvas:

- **Home's heading** is the house beside the word "Home"; no tagline.
- **An empty tab** (a workspace's, a task's, a session's) is the lattice and three key caps
  only: ⌘E Open an app, ⌘D Terminal, ⌘N New task. Home draws the same list (`fill_list`). A
  row's click does what its key does, as the user: in a workspace ⌘N's ↵ starts the task there.
- **The top row's glyph** is the core's (`Model::glyph`, in the state as `spaces[].glyph`): Home's
  house; a task's space its badges; a workspace whose tabs hold one pane, an app's, that app's
  icon (the pane header's); else the name (a workspace keeps its glyph beside it).

## The row in the Workspaces section

The owner: "clicking the body should open it, and we should have an ellipsis on hover that lets
us rename if we desire or delete". Built:

- A click anywhere on a row opens the workspace (selects it if open). No words on hover.
- The pointer on a row, or list mode's selection, shows one …. Its menu (`row_menu.rs`):
  Rename, Delete; a tab's row: Rename, Remove. ↓ ↑ move, ↵ takes the item the keys are on (none
  when a click opened it, the first when a key or a label did), Esc, any other key or a click
  outside closes. No item has a letter: a key meant for what is behind a short-lived menu must
  never delete (list mode keeps `r` and `x`). Delete keeps its Undo. Sneak labels reach the …
  and the items. The user's alone; any other move of the user's closes it.
- It holds the keyboard while open: nothing of a client's takes the keyboard under it
  (`MENU_OPEN`), a client's keys and clicks go where they went before, and a key held past the
  menu's end is not for what was under it.
- It hangs under its … (over it at the window's foot). Its … drawn elsewhere (a row above came
  or went, the window changed) closes it: it never hangs under another row.
- `Add tab` left the row: `a` in list mode and File ▸ Add Tab to Workspace… do it.
- The menu is one list of items per kind of row (`row_items`), drawn by one function: another
  row gets a menu by naming its items. No more than that.

**What a workspace keeps of what is open in it.** Made with +, a terminal added, closed with ×
and opened from the list: the same terminal, still running (× hides, it ends nothing). Closed,
then a restart, then opened from the list: its tabs, splits and each terminal's folder come
back from `layout.json`, which keeps hidden spaces too; a new shell, and its last lines as the
`workbook_scrollback_lines` setting says, as for any restored tab. Both are an e2e test. Lost
only where the user said so: `restore_layout: false` or Start Fresh (no layout is restored: a
tab never saved with ⌘⇧S is gone), and a tab closed with ⌘⇧W.

## The model (core: `model/spaces.rs`)

- A **space** is one of: **Home** (the first), an open **task**, a **session** the user
  opened from Other sessions that is no task, an open **workspace** (`Owner::Workbook`).
  `Model::spaces` is the row in order, hidden ones too.
- Every tab is of exactly one space (`Tab::space`); a space's tabs sit side by side in row order.
  A space has at least one tab: closing its last tab closes the space (it stays in Tasks or
  Workspaces). Home keeps its one empty tab.
- **One selection.** The selected space is the tab on show's. Selecting a space shows the tab it
  showed last. The drawer is only ever on the selected space's conversation: a task's (the one in front, with several) or a
  session's agent; none for a workspace. ⌘⇧L opens and closes the drawer; switching spaces keeps
  it open or closed. Closed on a conversation, with nothing else in the drawer's place, the
  **agent chip** (`Model::agent_chip`) is at the end of the tab row (the top row, for a space of
  one tab): the badge, the dial's marker and what the agent does. The user's click on it, or on
  the selected space (`SpaceClick`), opens the conversation as ⌘⇧L does; a client's click only
  selects. Selecting starts nothing: only `TalkStart` (and the user's ↵ on a new task) starts or
  resumes an agent.
- **Hide** (`SpaceHide`, × on the space) takes a space off the row. Nothing ends: agent, tabs,
  panes, terminals stay; hosted windows stay parked as for any tab not on show. Its row in Tasks
  (`TaskTalk`), `TaskGo`, its Workspaces row (`WorkbookOpen`) or `SpaceOpen` bring it back where
  it was. Selecting one of its tabs (`SelectTab`, a pane focused, a mark) shows it too.
- **New task** (⌘N, ↵ in the box): started in a kept workspace or a task's space, the task is
  in that workspace (below, "A task started in a workspace", "Several tasks in one
  workspace"). Else (from Home or a session's) its space is made at the row's end
  with one empty tab and selected.
- **A task gone** gives its tabs to the workspace it was bound to, else to the workspace
  selected last (Home's rule, below).
- **Repos**: a click on a repository or worktree (`RepoClick`) selects the workspace of that
  checkout, else the user's makes one (below, "Worktrees"). A client's, with none: a terminal
  there in the workspace selected last (Home's rule below). A row a live task works in carries
  that task's badge; Terminal, on hover, opens a terminal there anyway (`OpenRepo`).
- A tab changes space only by: the user's move (drag onto a space, Move Tab to…), a task
  started from it (above), its space gone (task dismissed or deleted, workspace deleted: the
  workspace selected last), the undo of a dismissal, a session adopted as a task (its space
  becomes the task's), or Home's rule (below). + moves none. The property test checks exactly this.

## Worktrees (built)

The owner: "the model is workspace has tasks - and the tasks for a workspace can have different
worktrees"; "the workspace _can_ carry a worktree (doesn't have to)"; "a terminal in the
workspace starts in the default worktree makes sense". A repository's own folder counts as its
main checkout.

- **A task's worktree** (`Model::task_worktree`) is the listed checkout its folder is in (the
  longest): the one place that says where its agent works. Its folder is its `/path` taken in
  the box, else its `>repo`, else what it took from its workspace (below).
- **A workspace's default worktree** (`Workbook::worktree`, in its file): optional. Only a kept
  workspace has one. It counts while it is listed (`Model::space_worktree`): a worktree gone
  from the list is no default, and comes back with it. A bound workspace keeps its own.
- **Set by the user's Repos click.** A click on a checkout's row (`repo_click`): the workspace
  whose default it is, else one that holds a live task working there (its task in front); of
  several, the one selected last, then the first on the row. A kept workspace that is closed
  opens, as its row would. None: a NEW kept workspace after the selected one (from Home: at the row's end), named for the
  worktree's branch, else its folder (a repository's own folder: the repository's name), that
  checkout its default, one tab with a terminal there, selected. From Home too; and the
  Terminal word from Home (`OpenRepo`, Home's rule) gives the workspace it makes the default.
  A worktree the user's + made (`WorktreeAdded`) opens as their click on its row would.
- **What starts there.** A terminal opened in the workspace with no folder of its own (⌘D, a
  split, a new tab's, the box's Terminal) starts in the default (`start_in_default`). A task the
  user starts there (↵) works in the default unless its line names another (`>repo`, `/path`);
  so does each task that joins later, not the folder of the one in front. A workspace with no
  default: terminals start where they did, and a joining task shares the front one's folder.
- **An agent that makes a worktree** (`agent_cwd`). The owner: "an agent could create a worktree
  when starting a task - in that case, we'd want the workspace associated with that task to
  connect to that worktree". The task's session says where its agent works: Claude Code writes a
  `cwd` on each record (`What::Cwd`, the main loop's only: a sub-agent's file and a
  `isSidechain` record say nothing of it); the shell passes the last one with the session's work
  (`SessionWork { cwd }`, the system's alone: a client's is refused). A runner whose records do
  not move with its agent is never followed (`Disk::cwd_moves`: Codex's `turn_context` `cwd` is
  the client's per turn, and ottospace sets none). When the `cwd` is in another listed checkout
  of the task's repository, the task works there (`Task::worktree`, kept in its file): its row,
  the drawer's foot, a terminal opened in its workspace and an agent started afresh say and use
  it. Its workspace's default follows from the task's old worktree only when the task is the
  workspace's one task (the main checkout to the new worktree: "some cases the agent will use
  the main checkout, and some cases it will create a worktree"); with others there it stays
  ("like the subagent case"), and the task's row and strip tab name its own.
- **A new worktree from the box** (`ComposeWorktree`, `box_worktree`, `Making`). The owner's
  canvas: "⌘N here joins this workspace and shares its checkout; the new-task box offers a new
  worktree as the other choice". Offered (`compose_worktree`) while the box is the drawer's, the
  task would start an agent in a listed checkout (the line's `>repo`, else the workspace's default
  or its front task's repository), not in a folder taken in the box, and `worktrees_dir` is set.
  Picks `this checkout` (default) / `new worktree`, ⌥⌘N switches; reset when the box closes. The
  user's ↵ (`ComposeSubmit { open }` by the user alone) files the task and joins it as any ↵ does,
  then asks for `Effect::AddWorktree { fresh: true }` on `repos::branch_for` the title (first four
  plain words, lowercase, `-`; `-2`… past a listed branch or folder) and remembers it (`making`):
  meanwhile `agent_start` is `None` and the drawer says "making its worktree …".
  `WorktreeAdded` with that `asked` path: the task's folder is the worktree, the default of a
  workspace of that one task follows from its old checkout (`default_follows`, #186's rule), and
  its agent starts (`launch`, the system's, for the user's ↵). `WorktreeFailed`: the task waits,
  unstarted, in its checkout; the shell's notice says git's or its own why. The shell makes a
  `fresh` one only as a new branch from HEAD: a name `refs/heads/` or any `refs/remotes/*/` has
  is refused before anything is made, so no one's code is checked out for an agent. A client's
  choice is refused; a client's ↵ makes none.
- **Never a sub-agent's.** A `cwd` in `<repo>/.claude/worktrees/agent-*` (Claude Code's
  isolated sub-agents; `subagent_path`) moves nothing, nor does one in another repository.
- **Made a moment ago.** A `cwd` the list does not hold yet: the list is read again at once
  (`Effect::ReadRepos`), then every `MOVE_RETRY_MS` (2 s) for `MOVE_WAIT_MS` (10 s), and the task
  follows when it is there. A `cd` inside the checkout asks the same few times: the list is files
  git wrote, read in a moment.
- **Gone on with** (`--resume`): where the session started, as before, since Claude Code finds a
  session by the folder it started in.
- **Shown.** The state's `spaces[].worktree`. Where a workspace's tasks work in more than one
  worktree, each task's row says its worktree's name and its strip tab's hover too
  (`Model::worktree_label`, `tasks_bar` rows' `worktree`); else nothing is said.
- **Who.** Only the user's act sets a default (a Repos click, their + made a worktree), or the
  task's own agent moving (above), or the files read again: a client's click on a row with no workspace opens a terminal as `OpenRepo`
  does and sets none, and no request sets one.
- Property rules: a terminal opened by a step that names no folder starts in its space's default;
  a task that joins a workspace with a default works there unless its line names another; no
  step but the user's (`WorktreeAdded` counts, and `RestoreWorkbooks`), or one that follows an
  agent's move (`SessionWork` not a client's, the list read again for it), changes a workspace's
  default or a task's worktree; a sub-agent's worktree is never a task's.

## Earlier: a workspace's two links (replaced)

The owner: "a workspace can be associated with: a single task; a single worktree. a task can be
associated with a single workspace. a repository can be associated with a single task. so its
essentially a 1:1:1 relationship - it's just you can have a workspace with no task or worktree, a
workspace with only a task or worktree, and a workspace with both".

- **Stage 2: one kind of space, two optional links.** A workspace has `task` and `worktree` (a
  repository's own folder counts: its default checkout), each held by one workspace at most. A
  task's space is then a workspace with its task set, not another kind: binding is setting a
  field. A click on a Repos row selects the workspace linked to that worktree, else makes one
  (a terminal there, named for the worktree while it has no task); it no longer opens a tab in
  a new workspace. Sessions stay a kind of their own.
- With the worktree link: a worktree's workspace gives a task started in it its repository,
  unless the line's `>repo` names another.
- **Replaced by the owner** ("Several tasks in one workspace", below): a task belongs to one
  workspace, a workspace holds none or more. A second task in a workspace joins it and shares
  the checkout; "a new worktree for this task" becomes a pick in the new-task box, not the
  default (built: "Worktrees", "A new worktree from the box").

## A task started in a workspace (built)

The owner: "if i have a workspace w/o an active task, and i'm focused in it, and I press
command+n for a new task and specify the task - the active workspace should now be attached to
the task (right now - a new task/workspace gets created for the task)".

- **The user's ↵ in the new-task box** (the one that adds the task and starts its agent), while
  the selected space is a workspace of no task: that workspace is the task's space from then
  on (`Model::bind_space`). In place on the top row, with all its tabs and panes: no pane
  moves, nothing closes, no space is made. The drawer shows the task's conversation there.
  This replaces the one-tab exception (`bind_tab`).
- **A kept workspace** (made with +, or saved) stays what it is: listed in Workspaces, its file
  kept, now with `task` (`Workbook::task`, in its file). On the top row it shows as the task
  (badge and title); its row in the section carries the task's badge and opens the task's
  space (a saved tab of it that is closed is built there); ⌘⇧S there saves into it; a tab moved "to the workspace" goes to the task's space.
  Deleted while bound, the task keeps the space and its tabs; the delete's Undo binds it again.
  The task's space gone while bound (its last tab closed, or no layout restored): its row
  opens it as its own again, with its saved tabs, unbound.
- **Home** is not taken: a task started there gets a space of its own. The one-tab exception
  is gone with `bind_tab`.
- **Unbound when the task goes.** The task dismissed, or deleted: the workspace is its own
  again, where it was on the row, with its tabs. The user's undo of the dismissal binds it
  again. A task that is done but not dismissed keeps its space, as every task does.
- **Since**: a workspace that has a task takes more (below, "Several tasks in one workspace").
- **Nothing else binds.** ⇧↵ (for later; no agent), a line whose agent would not start, a
  client's `task_new` (it never takes the user's workspace), a task started from the first
  space or a session's: a space of its own at the row's end.
- **The box says so.** While ↵ would start it in a workspace that holds a pane or a task: "↵
  starts it in this workspace" (after the plan, when there is one: "↵ will start codex in app
  on main, in this workspace"). An empty workspace is bound with nothing said.
- `>repo` on the line is as before: the agent runs where the line says. Binding changes no folder.
- **How it is modelled.** The smallest thing that gives the behaviour: the space changes kind
  in place (`Owner::Workbook` → `Owner::Task`), and the kept workspace records the link
  (`task`). So every rule about a task's space (drawer, hide,
  restore) holds as it was. The end state is still one kind of space with two optional links
  (above): then binding is only setting the field.
- Property rules: one workspace a task, a live one; a bound workspace has no space of its own;
  only the user's ↵ on a new task binds (or their undo of its dismissal, or files read again);
  binding moves no pane and closes none.

## Several tasks in one workspace (built)

The owner: "i'm also wondering if we actually want the ability to have multiple tasks in a
workspace (a task is always associated with a workspace, but a workspace may have multiple
tasks)"; "lets join the workspace and share the checkout"; "switching between them is enough
right now"; "i'd expect < 5 so the ui should be suitable for that".

- **↵ joins.** The user's ↵ in the new-task box, in a workspace made with + or in a task's
  space, with or without tasks: the task is in that workspace. No space is made, no pane
  moves, nothing closes; its conversation is in front. It works in the folder of the task
  that was in front, unless its line names a repository or a folder. The box says "↵ starts
  it in this workspace" (after the plan when there is one) while the workspace holds a pane or
  a task. Not from Home (a space of its own) nor a session's.
- **↵ on a row of the dropdown** takes the row: no task, nothing joined, and the box says
  "↵ completes", not "starts it in this workspace". A click on a tag's cap joins nothing.
- **⇧↵ (for later) joins nothing**: it has no agent and no conversation to put in front, so it
  is listed in Tasks with no workspace; started later from its row, it gets its own. A
  client's `task_new` joins nothing either: it never puts a task in the user's workspace.
- **One conversation in front** a workspace (`Model::front`): the drawer's subject, the
  dial's, the agent chip's, `tab_task`. Everything that spoke of "the selected space's
  conversation" asks `Model::space_subject`; where a task lives is `Model::task_space`.
- **Who changes the front**: the user alone. A new task they start (it), a tab of the strip
  (`TalkFront`), ⌘⌥] / ⌘⌥[ (`TalkCycle`, around the strip), a task's row and ⌘⇧N (which
  switches within and across workspaces). A client's `TalkFront`/`TalkCycle` is refused
  (`FRONT_USERS`), and its `task_open` selects the workspace and leaves the front one.
- **The front one gone**: dismissed or deleted by anyone, the most urgent remaining task is
  in front (needs you, running, later, on hold, done; then priority, then the older). Marked
  done by the user (in any workspace): the same, among those not done. Marked done by its agent's report: it
  stays in front, since that is no act of the user's and a conversation is not taken from
  under them; it leaves the strip once another is in front.
- **The drawer's strip** (`Model::strip`, `talk_strip`): over the conversation, a tab a task
  in the order they came, done ones left out. Each is the task's badge, whose corner dot is
  its state's colour, so one that needs you is seen without opening it. The front tab is wide
  and says its title, cut with …; the others are badges, the title on hover. + at its end
  opens the new-task box here. Hidden with fewer than two tasks. Made for one to five: no
  scrolling, no overflow menu; past that the others stay badges and the front one keeps 96
  points for its title.
- **The top row**: a workspace's item shows its tasks' badges stacked, the most urgent first
  and on top (five at most), then its name: the kept workspace's name with several tasks,
  the task's title with one, as before. Its summary state is its most urgent task's.
- **How it is modelled.** A space of a task (`Owner::Task`, which a bound workspace's is)
  lists the tasks that joined it (`Space::joined`) and the one in front (`Space::front`). A
  task is in one space at most: joined to one, or its own. After every step (`settle_joined`):
  a joined task is live, joined once and has no tab of its own; a space whose own task is gone
  while others remain becomes the most urgent one's (the one in front, if it is one of them),
  in place, with its tabs and its kept workspace's link; the last task gone, the workspace is
  its own again with its tabs (as before). The user's undo of a dismissal puts the task back
  in the workspace it left; not in front.
- **Kept between runs**: `layout.json`, each space's `tasks` (joined, in order) and `front`.
  A task of the file that is gone, listed twice or with a space of its own is left out.
- Property rules: a task is in one space at most; the front task is one of the space's; no
  step but the user's takes a task from the front while it is still there; only the user's ↵
  on a new task (or their undo) joins a task to a workspace.

## Open questions

1. ~~A second task for a worktree whose workspace already has a task~~: decided, it joins
   (above, "Several tasks in one workspace"), and may work in another worktree ("Worktrees").
2. Should a tab made in a workspace be saved into its file without ⌘⇧S ("a workspace is what is
   open in it")? Today the file changes only when the user saves; session restore keeps the rest.
3. ~~The first space~~: decided, Home (above).

## Who may do what

Socket clients may select spaces and tabs, hide and open task spaces (`select_space`,
`hide_space`, `open_space`), as they may select tabs. No one but the user moves what the user
types to: while the keys are in the chat, a client's selection of another space, a request that
takes the keyboard, opens the list or closes the drawer is refused (`CHAT_TYPING`), and any other
step of theirs that would show another space (a new tab, a closed one, a Repos row) leaves the
tab on show where it was and the keys in the chat (`Model::keep_shown`). The new task the user is
writing is not closed or filed by a client (`BOX_TYPING`). The property test checks both for
every action of a client or the system. Moving a tab between spaces is the user's alone
(`TABS_USERS`), so is making a workspace (it writes their file) and opening one (it starts their
apps): no request was added for +, and a test clicks it as the user. A client's new tab, or its
pane on Home, goes to the workspace selected last (Home, above).

## Kept between runs (`layout.json`, version 1)

`spaces`: each `kind` (`workspace`: Home, which keeps no tab; `task`; `workbook` by uid),
`hidden`, `tab` (its last tab, by place); each tab's `space` (place in `spaces`), and still its
`task`, so an older build reads it; `home`, if Home was on show. A file with no `spaces` (an
older build's) puts a tab with a live task in that task's space, the rest in the first space,
which becomes "Workspace" (Home, above). An unknown `kind`, a task gone or a workspace not kept,
a session's space (not kept): its tabs go to the workspace selected last. A
workspace made with + is a file with no tabs (`workbooks/<id>.json`): a build before this one
counts such a file as unread and leaves it alone.

## Opening and saving a workspace

Opening one builds its closed tabs into its space and selects it; a tab of it open in another
space stays there. One
with no saved tab opens its space as it is, else with one empty tab. Saving a tab (⌘⇧S) or all
tabs leaves the tabs where they are. Save All Tabs saves the selected space's tabs as one new
workspace.
