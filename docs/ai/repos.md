# Repos: folding, the default folder, new worktrees

The Repos section lists each repository's name over its checkouts. A click on a row selects the
workspace of that checkout, else makes one with it as its default worktree: `docs/ai/spaces.md`,
"Worktrees".

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
5. Made: the list is read again and the worktree opens as the user's click on its row would
   (`WorktreeAdded`, `Origin::System`): a new workspace of it. Not made: git's message, cleaned and cut to 200
   characters, in the status bar; an empty `<repo>` folder made in step 4 goes; anything git
   left is left, and said.

The new-task box makes one too (`docs/ai/spaces.md`, "A new worktree from the box"): the same
folder and checks, `fresh` (only `git worktree add -b <branch> -- <folder> HEAD`; a branch any
local or remote ref has is refused, nothing made), and `WorktreeAdded`/`WorktreeFailed` carry the
folder as asked, so the core finds the task that waits for it.

## Pull requests (`prs.rs` in the core and the shell; `SetPrs`, `PrOpen`, `Effect::OpenPr`)

- **When**: only while the Repos section is open (`prs::due`): a repository is read every 5
  minutes, and on the section's opening if its last read is a minute old or more. Only a
  repository whose `origin` is on github.com (`.git/config`). `repos_prs`: `auto` (default),
  `on`, `off`. gh is found in the absolute folders of `PATH`, then `/opt/homebrew/bin`, `/usr/local/bin`; a test run uses
  only `OTTOSPACE_GH`, a stand-in.
- **What runs**, on its own thread, 20 s each, no prompt, no colour, no update check:
  - once, and again after 5 minutes while it fails: `gh auth status --hostname github.com`
  - per repository, in its folder: `git config --get-regexp '^(branch\..*\.remote|remote\..*\.url)$'`,
    `git -C <checkout> rev-parse HEAD` per checkout, then
    `gh pr list --state open --limit 100 --json number,title,isDraft,headRefName,headRefOid,headRepository,headRepositoryOwner,isCrossRepository,baseRefName,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup,url`
- **Whose pull request** (`prs::pick`): of the checkout's branch name, and from the repository
  the branch pushes to: its own remote (`branch.<name>.remote`) by `owner/name`, else `origin`,
  and then only a same-repository pull request (`isCrossRepository` false). A fork's pull
  request with the same branch name is never a worktree's. Two candidates: the one whose
  `headRefOid` is the checkout's commit; else none.
- **The mark** (`Summary::look`), by precedence: `failing` (a failed check, changes requested,
  a conflict: red dot), `draft` (number muted, no dot), `pending` (checks running, review
  required: amber dot), `ready` (approved, checks passing or none, mergeable and clean: green
  tick), else `open` (the number alone).
- **The card** on hover: title, `Draft`/`Open` → base, checks (passing, running, or up to three
  failing names and how many more), review, merge. Titles and check names are other people's
  text: cleaned (`transcript::clean`), one line, cut short, never a link, never in the state
  (the state's `pr` is numbers and kinds). Nothing gh says is ever an argument or a path.
- **Open**: the user's click only (`PrOpen` is refused from anyone else), and only a link of the
  form `https://github.com/<owner>/<repo>/pull/<n>` (`openable`, checked again at the door).
- **Failures** (no gh, logged out, rate-limited, a timeout): no marks, no popup; with
  `"repos_prs": "on"` one quiet line at the pane's foot says why.

## Never done

- No worktree is removed: that deletes a folder, and was not asked for.
- The trash on a name stops listing the repository (`RepoRemove`): it changes `settings.json`,
  never anything on disk. While a live task works in one of its checkouts it asks first (the shared
  confirm, `Confirm::unlist`, the user's to answer); after, the status bar says, as information,
  "Unlisted <name> · nothing on disk changed" (`Effect::Told`).
- No argument is built from anything but the checked branch and the folder made here; git runs
  as an argv, never through a shell.
