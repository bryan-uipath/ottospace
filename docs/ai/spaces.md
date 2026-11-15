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
  and records no tab: its file has no tabs until one is saved into it. Twice makes two. The
  user's alone, as saving is: it writes a file.
- **New task** is ⌘N and the Tasks section's +. The top row's + no longer opens the box.
- **× keeps it listed.** Hiding a workspace takes it off the row with its tabs and panes as they
  are; its row in the section brings it back. Its last tab closed: off the row, still listed;
  opened again it shows its saved tabs, or one empty tab.
- **Saving** stays, and is never reached from +: ⌘⇧S saves the tab on show into the workspace
  whose space it is in (a tab of it already: updated in place), else as a new workspace of that
  tab; `Add tab` on a row; Save All Tabs as Workspace. A tab made in a workspace is kept by
  session restore (`layout.json`) while its space is open or hidden, and in the workspace's own
  file only once saved.
- **The first space, "Workspace"**, stays as it was: always first, never hidden, in no file and
  not in the section. It is the home of tabs that are of nothing (below). Folding it into the
  list did not fall out naturally: it has no file and cannot be deleted.

## The model (core: `model/spaces.rs`)

- A **space** is one of: **Workspace** (the first), an open **task**, a **session** the user
  opened from Other sessions that is no task, an open **workspace** (`Owner::Workbook`).
  `Model::spaces` is the row in order, hidden ones too.
- Every tab is of exactly one space (`Tab::space`); a space's tabs sit side by side in row order.
  A space has at least one tab: closing its last tab closes the space (it stays in Tasks or
  Workspaces); the first space gets an empty tab instead.
- **One selection.** The selected space is the tab on show's. Selecting a space shows the tab it
  showed last. The drawer is only ever on the selected space's conversation: a task's or a
  session's agent; none for a workspace. ⌘⇧L opens and closes the drawer; switching spaces keeps
  it open or closed. Selecting starts nothing: only `TalkStart` (and the user's ↵ on a new task)
  starts or resumes an agent.
- **Hide** (`SpaceHide`, × on the space) takes a space off the row. Nothing ends: agent, tabs,
  panes, terminals stay; hosted windows stay parked as for any tab not on show. Its row in Tasks
  (`TaskTalk`), `TaskGo`, its Workspaces row (`WorkbookOpen`) or `SpaceOpen` bring it back where
  it was. Selecting one of its tabs (`SelectTab`, a pane focused, a mark) shows it too.
- **New task** (⌘N, ↵ in the box): the task's space is made at the row's end with one empty tab
  and selected. One exception, in sight: started while a tab of the first space is on show that
  holds a terminal in the task's folder (a tab opened from the Repos section), that tab moves
  into the new space as its first (`Model::bind_tab`). Nothing else moves. Stage 3 replaces it.
- **Repos**: a click on a repository or worktree opens a terminal there in a tab of the first
  space of its own (the empty one on show, else a new one) and selects it. A row a live task
  works in carries that task's badge, and its click (`RepoClick`) selects the task's space
  instead (the latest started, if several); Terminal, on hover, opens the terminal anyway
  (`OpenRepo`). Stage 2 changes the click.
- A tab changes space only by: the user's move (drag onto a space, Move Tab to…), a task
  started from it (above), its space gone (task dismissed or deleted, workspace deleted: the
  first space's), the undo of a dismissal, a session adopted as a task (its space becomes the
  task's), or its workspace opened (a tab of it in the first space joins the workspace's space).
  + moves none. The property test checks exactly this.

## Next: a workspace's two links (not built)

The owner: "a workspace can be associated with: a single task; a single worktree. a task can be
associated with a single workspace. a repository can be associated with a single task. so its
essentially a 1:1:1 relationship - it's just you can have a workspace with no task or worktree, a
workspace with only a task or worktree, and a workspace with both".

- **Stage 2: one kind of space, two optional links.** A workspace has `task` and `worktree` (a
  repository's own folder counts: its default checkout), each held by one workspace at most. A
  task's space is then a workspace with its task set, not another kind: binding is setting a
  field. A click on a Repos row selects the workspace linked to that worktree, else makes one
  (a terminal there, named for the worktree while it has no task); it no longer opens a tab in
  the first space. Sessions stay a kind of their own.
- **Stage 3: a task started inside a workspace binds it.** The user's ↵ in the new-task box
  while a workspace with no task is selected: that workspace becomes the task's, in place, with
  all its tabs (replaces `bind_tab`). A worktree's workspace gives the task its repository; a
  `>repo` on the line that names another wins, and the task gets a new workspace. ⇧↵ (for
  later), a client's `task_new`, and a task started from a task's or session's space bind
  nothing. While binding would take a workspace that holds a pane, the box's line says so
  ("↵ makes this workspace the task's").
- Rules for the property test then: one workspace a task, one a worktree; binding moves no
  pane and ends nothing; nothing but the user's act sets or clears a link on the space the user
  is in.

## Open questions

1. A second task for a worktree whose workspace already has a task. Not decided. Until it is:
   the second task's workspace has the task link only; its agent still runs in that folder. One
   function, one test, cheap to change.
2. Should a tab made in a workspace be saved into its file without ⌘⇧S ("a workspace is what is
   open in it")? Today the file changes only when the user saves; session restore keeps the rest.
3. The first space: keep it apart, or make it an ordinary first workspace once links exist
   (strays then go to the workspace selected last)?
4. The row in the section: body opens, one … for Rename and Delete (asked for; its own change).

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
apps): no request was added for +, and a test clicks it as the user. A client's new tab is the
first space's.

## Kept between runs (`layout.json`, version 1)

`spaces`: each `kind` (`workspace`: the first space; `task`; `workbook` by uid), `hidden`, `tab`
(its last tab, by place); each tab's `space` (place in `spaces`), and still its `task`, so an
older build reads it. A file with no `spaces` (an older build's) puts a tab with a live task in
that task's space, the rest in the first space. An unknown `kind`, a task gone or a workspace not
kept: the first space's. A session's space is not kept: its tabs come back in the first space. A
workspace made with + is a file with no tabs (`workbooks/<id>.json`): a build before this one
counts such a file as unread and leaves it alone.

## Opening and saving a workspace

Opening one builds its closed tabs into its space and selects it; its tabs open in the first
space (one saved from there) come into its space too, and one of a task's stays the task's. One
with no saved tab opens its space as it is, else with one empty tab. Saving a tab (⌘⇧S) or all
tabs leaves the tabs where they are. Save All Tabs saves the selected space's tabs as one new
workspace.
