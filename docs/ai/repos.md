# Repos: folding, the default folder, new worktrees

The Repos section lists each repository's name over its checkouts. Clicks on a row are as
`docs/ai/spaces.md` says (a task's row selects the task; else a terminal in a Workspace tab).

## The model (core: `repos.rs`, `model.rs`)

- **A repository's name is a list row** (`ListRow::RepoHead`): ↵ or space on it folds or
  unfolds its rows; ← folds the selected row's repository, → unfolds it. List mode starts on the
  first checkout, as before.
- **Folded** (`repos_folded`, by path, kept in `layout.json`): its rows are not drawn and are not
  list rows; the name shows a count of its checkouts. A folded repository is shown unfolded
  while a live task in one of its checkouts runs or needs the user, and folded again when that
  ends. Folded by the user while one does, it stays folded (`repos_held`, not kept) until that
  ends. Anyone may fold (`RepoFold`): it changes what is drawn, not what runs.
- **The default folder**: the repository's own directory, the first row. A small dot in the
  accent colour in the row's gutter marks it, in place of the "Default" line; its tooltip says so.
  Nothing sets a default: there is no "default repository for new tasks".
- **Rows are one line**: the branch, cut short before what is drawn at the row's end (the task's
  badge), never under it. The path is the row's tooltip.

## A new worktree (`WorktreeEdit`, `MarkSubmit`, `Effect::AddWorktree`, `WorktreeAdded`)

1. The user's + on a name (or `a` in list mode) opens a one-line field under it, as a workbook's
   name is typed (`MarkInput::Worktree`). Only the user: the core refuses `WorktreeEdit`, and
   any edit, ↵ or Esc of the field, from any other origin. There is no socket request.
2. ↵ checks the branch as `git check-ref-format --branch` would, and stricter (`check_branch`):
   not empty, at most 200 bytes, no leading `-`, no space, control character, `~ ^ : ? * [ \`,
   no `..`, `@{`, `@` or `HEAD`, no part that is empty, starts with `.` or ends with `.lock`, no
   trailing `.`. A refused name is said in the status bar and the field stays.
3. The folder: `<worktrees_dir>/<repo>/<branch>` (`worktree_dir`). `worktrees_dir` is the
   setting (a relative path is inside the ottospace folder), else `worktrees` in the ottospace folder (`OTTOSPACE_HOME` honoured; a test run
   with no folder of its own has none, and the + says so). `<repo>` is the repository's folder
   name, with `-<6 hex digits>` (FNV-1a of its path) when another listed repository has the same
   folder name. Slashes in the branch become `-`.
4. The shell (`worktree.rs`), off the main thread: the folder must be two plain names under
   `worktrees_dir` (resolved); `<repo>` a real folder, not a link (made if missing); the
   worktree's folder must not exist (not even as a link); `<repo>` must not be a checkout itself
   (`worktrees_dir` inside the repositories' folder). Then, with `-C <repo>`, no prompt
   (`GIT_TERMINAL_PROMPT=0`), no `GIT_DIR`-like variables, nothing on standard input, a 120 s
   timeout:
   - `git rev-parse --verify --quiet refs/heads/<branch>`
   - there: `git worktree add -- <folder> <branch>`
   - only a remote has it (exactly one remote; `git for-each-ref --format=%(refname) refs/remotes/*/<branch>`):
     `git worktree add --track -b <branch> -- <folder> <remote>/<branch>`
   - neither: `git worktree add -b <branch> -- <folder> HEAD`
5. Made: the list is read again and the worktree opens as its row's click would
   (`WorktreeAdded`, `Origin::System`). Not made: git's message, cleaned and cut to 200
   characters, in the status bar; an empty `<repo>` folder made in step 4 goes; anything git
   left is left, and said.

## Never done

- No worktree is removed: that deletes a folder, and was not asked for.
- The trash on a name stops listing the repository (`RepoRemove`): it changes `settings.json`,
  never anything on disk.
- No argument is built from anything but the checked branch and the folder made here; git runs
  as an argv, never through a shell.
