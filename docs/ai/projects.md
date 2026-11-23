# Projects

Status: slice 1 in (behind `projects`). The owner's ask: a Projects entry on the left rail listing
all projects; a setting for where they live; repositories linked to projects and back; a project's
progress and help with its next actions. Design: the canvas's board C ("Projects and the weekly
note") and its "Decided" note.

## Decided (owner)

- A Projects section on the left rail, like Repos. Nothing is read until `projects_dir` is set.
- Order: projects the user dragged come first, in that order; the rest by most recently used. No
  switch between the two.
- ottospace may read the notes. The user's own typed words may later be written into them;
  agent-written text never: it lives in ottospace's own folder and links to the note.
- No MCP server: agents reach projects through the control socket, read-only.

## Assumptions (made here, not by the owner)

- **A project on disk.** `projects_dir` names one folder. Each entry in it is a project if it is
  a folder, or a lone `.md` file. Hidden names (`.x`) and links are skipped.
- **Its note.** In a folder: `README.md`, else `index.md` (any case), else its only `.md`, else the
  most recently changed `.md`. A lone `.md` is its own note. A folder with none has no note.
- **Its title.** The note's first ATX heading (`#` to `######`), past a front matter block and
  outside fenced code; else the name, without `.md`. Control characters dropped, 120 characters at
  most. No front matter key is read yet.
- **Recently used.** Until ottospace opens projects (slice 2), "used" is the last change on disk:
  the folder's own time (an entry added, removed or renamed; most editors save by renaming) or its
  note's, whichever is later. Slice 2 adds the last time ottospace opened it.
- **Limits.** At most 500 projects, of the first 2000 entries; 200 entries looked at in a folder;
  a note over 256 KB is titled by its name. Read with `O_NOFOLLOW | O_NONBLOCK` (no link, no pipe).
- **When read.** Only while the section is open: on opening, then every 10 s, on a thread of its
  own. The setting is read live: setting or removing it shows or hides the section at once.
- **What the socket sees.** Every reply carries the state, so titles and paths are not in it, as
  meeting titles are not: `projects_bar` has each row's kind (`file`, `note`), `placed`, `modified`
  and its drawn rect. A later read-only request may give titles, as `calendar` does (below).

## The model

`ottospace-core::projects`: `Project { path, title, note, modified }` and `Projects { on, listed,
placed }`. `ordered()` is the rule: each placed path that is listed, in `placed` order; then the
rest by `modified` (newest first), title, path. `title_of`, `note_of` and `due` are pure and
tested; the shell's `projects.rs` only lists the folder and reads notes.

`Action::SetProjects { on, listed }` (the shell's): the setting's state and a read. With `on`
false the section closes, as the Calendar's does without a feed. `Section::Projects` sits under
Repos; its `ListRow::ProjectEntry(path)`. Neither `layout.json` nor the state keeps it open across
a build without it.

## Slices

1. **The list** (this): flag, setting, model and order rule, the rail cell and the list (title,
   last change), the folder read in the shell. e2e on a made-up folder; a shots scenario.
2. **Drag and open.** Drag a row to place it (the reorder the Workspaces and Repos rows use; Esc
   puts it back); placed paths kept in `~/.ottospace/projects.json`, never in the notes. A thin
   line between placed and the rest. Opening a project (click, ↵): its note in a file viewer pane,
   read-only, as ⌘-click on a path opens one; a project with no note opens nothing and says so on
   its row. Opening counts as use for the order.
3. **Repositories.** Link a listed repository to a project, from either side (a project's ⋯ ›
   Link repository…; a Repos row's menu › Link to project…). Kept in `projects.json` with the
   order, keyed by the project's path; shown on both rows (the repo's name under the project, the
   project's title on the repository's heading). A renamed folder loses its links; ottospace may
   offer to re-link.

## Proposal: progress and next actions (not built)

From the canvas (C3), what can be shown with no bookkeeping:

- **On the row:** activity, 8 weekly bars of days with any change (folder files, commits in linked
  worktrees, its tasks' events); motion, never called progress. And its tasks by `#project` tag:
  "5 done · 2 open", the bar the Tasks section's Project view draws today.
- **In its hover card:** the note's checkbox count (`- [x]` of all) when it has any, and the linked
  worktrees' pull requests, as Repos reads them.
- **Next action, read:** the first unchecked item under a heading named "Next", else the first
  unchecked item in the note. Shown on the row; ticking it is the user's own act (their words, so
  it may be written).
- **Next action, proposed:** only inside a weekly review view (⌘P › Review Projects): one row per
  project, quiet ones first, each asking keep / choose next action / someday / done. An agent may
  propose next actions there, with the note and recent tasks as context; what the user picks is
  kept in ottospace's folder, linked to the note, never written into it.

## Open questions

- **"Back to recent"** in a row's menu (unplace a dragged project): left out until decided.
  Recommendation: add it in slice 2 as the menu's only item; dragging a row below the line does
  the same.
- **A read-only `projects` request** giving titles and note paths to agents (as `calendar` gives
  meeting titles), labelled untrusted data. Recommendation: yes in slice 3, once links exist.
- **Front matter** (`status`, `repos`): read it when present, as C2.2 suggests? Recommendation: not
  until a slice needs a key; titles ignore it today.
- **Archive and delete** (moving a folder): both write to the notes folder. Recommendation: out of
  scope while the folder is read-only.
- **A project with no note:** open a terminal in its folder instead? Recommendation: no (slice 2
  opens notes only); revisit with workspaces that span a project and a repository.
