# Spaces: two levels of tabs

The hierarchy is **space → tabs → panes**. Replaces "a tab may belong to a task", the right
rail's cells and the drawer's go word (`docs/ai/ux-simplification.md` §1.7, §3).

## The model (core: `model/spaces.rs`)

- A **space** is one of: **Workspace** (always first, never hidden: the tabs of no task or
  workbook), an open **task**, a **session** the user opened from Other sessions that is no task,
  an open **workbook**. `Model::spaces` is the row in order, hidden ones too.
- Every tab is of exactly one space (`Tab::space`); a space's tabs sit side by side in row order.
  A space has at least one tab: closing its last tab closes the space (it stays in Tasks or
  Workbook); Workspace gets an empty tab instead.
- **One selection.** The selected space is the tab on show's. Selecting a space shows the tab it
  showed last. The drawer is only ever on the selected space's conversation: a task's or a
  session's agent; none for Workspace or a workbook. ⌘⇧L opens and closes the drawer; switching
  spaces keeps it open or closed. Selecting starts nothing: only `TalkStart` (and the user's ↵ on
  a new task) starts or resumes an agent.
- **Hide** (`SpaceHide`, × on the space) takes a space off the row. Nothing ends: agent, tabs,
  panes, terminals stay; hosted windows stay parked as for any tab not on show. Its row in Tasks
  (`TaskTalk`), `TaskGo`, its Workbook row (`WorkbookOpen`) or `SpaceOpen` bring it back where it
  was. Selecting one of its tabs (`SelectTab`, a pane focused, a mark) shows it too.
- **New task** (⌘N, ↵ in the box): the task's space is made at the row's end with one empty tab
  and selected. One exception, in sight: started while a Workspace tab is on show that holds a
  terminal in the task's folder (a tab opened from the Repos section), that tab moves into the
  new space as its first (`Model::bind_tab`). Nothing else moves.
- **Repos**: a click on a repository or worktree opens a terminal there in a Workspace tab of its
  own (the empty one on show, else a new one) and selects it.
- A tab changes space only by: the user's move (drag onto a space, Move Tab to Task…), a task
  started from it (above), its space gone (task dismissed or deleted, workbook deleted:
  Workspace's), the undo of a dismissal, or a session adopted as a task (its space becomes the
  task's). The property test checks exactly this.

## Who may do what

Socket clients may select spaces and tabs, hide and open task spaces (`select_space`,
`hide_space`, `open_space`), as they may select tabs. No one but the user moves what the user
types to: while the keys are in the chat, a client's selection of another space, a request that
takes the keyboard, opens the list or closes the drawer is refused (`CHAT_TYPING`), and any other
step of theirs that would show another space (a new tab, a closed one, a Repos row) leaves the
tab on show where it was and the keys in the chat (`Model::keep_shown`). The new task the user is
writing is not closed or filed by a client (`BOX_TYPING`). The property test checks both for
every action of a client or the system. Moving a tab between spaces
is the user's alone (`TABS_USERS`), so is opening a workbook's space (it starts their apps). A
client's new tab is Workspace's.

## Kept between runs (`layout.json`, version 1)

`spaces`: each `kind` (`workspace`, `task`, `workbook` by uid), `hidden`, `tab` (its last tab,
by place); each tab's `space` (place in `spaces`), and still its `task`, so an older build reads
it. A file with no `spaces` (an older build's) puts a tab with a live task in that task's space,
the rest in Workspace. An unknown `kind`, a task gone or a workbook not kept: Workspace's. A
session's space is not kept: its tabs come back in Workspace.

## Workbooks

Opening a workbook builds its closed tabs into its space and selects it; tabs of it already open
elsewhere stay where they are and the first is selected. Saving a tab (⌘⇧S) or all tabs leaves
the tabs where they are: a workbook is a file; its space is what opening it makes. Save All Tabs
saves the selected space's tabs as one workbook.
