# Repos: folding, the default folder, new worktrees

The Repos section lists each repository's name over its checkouts. A click on a row selects the
workspace of that checkout, else makes one with it as its default worktree: `docs/ai/spaces.md`,
"Worktrees".

## The list: opt-in (`RepoAdd`, `RepoRemove`, `Effect::SaveRepos`)

- **Only what the user added.** `repos` in the settings is the list, in its order: a path, or
  `{"path": …, "pinned": true}` (`settings::RepoEntry`). No default: a new user sees an empty
  section with "+ Add a repository". `~/code` and `~/notes` are looked in no more.
- **Adding** (the user's alone; the core refuses `RepoAdd` from anyone else; `repo_add` on the
  socket only under `OTTOSPACE_E2E`): the section's + (folder picker), a folder dropped on the
  section, or "Add Folder to Repos" (palette, File menu: the focused terminal's folder, else the
  tab's task's). The shell resolves the folder (links, `..`), then the repository it is in
  (`repos::repository_of`: the nearest `.git` up; a linked worktree's is its main repository's).
  The core takes only a repository's own folder (`RepoDirs::may_add`): anything else says "Not a
  repository: …", nothing written. It goes at the end; one listed already is selected.
- **Writing**: the whole list, in order (`Effect::SaveRepos { dirs }`, `settings::with_repos`),
  into the file as it is now; a `repos` that is not a list of entries is left alone and the
  status bar says so. Only `RepoAdd`, `RepoRemove` (and the confirm that follows it) write it.
- **Migration** (nothing is rewritten): an old entry that is a repository is an entry. One that
  is a folder of repositories (the old "root") is kept as a **group**: one row, `~/clients/ · 12`,
  folded until the user unfolds it (`repos_open`, not kept), its repositories by name under it,
  its trash removes the entry whole. One of them added by itself becomes an entry and leaves the
  group. The old invisible default is not migrated: it was never in the file.
- **Order**: `arrange` sorts by the entry's place (`RepoDirs::place`), a group's repositories by
  name; reads of the disk (`SetRepos`) never change it.
- `OTTOSPACE_REPOS` (tests, scripts): fixed, flat (a folder there lists its repositories with no
  group), nothing added, removed or moved.
- `open_repo` from a client: unchanged, only a path in `state.repos` (a group's repositories
  included, folded or not).

## The + : Local folder… or From GitHub… (`MenuOf::ReposAdd`, `clone.rs`, `model/clone.rs`)

- **The +** opens a row menu of two items, the keys on the first: ↵ is Local folder…, the folder
  picker as before (`RepoPickFolder`, `Effect::PickRepoFolder`; a test run counts it,
  `counts.folder_picks`, and shows none). From GitHub… opens the clone dialog (`CloneOpen`). The
  empty section offers both as rows. Both, and every key, click or ↵ in the dialog, are the user's
  alone; the shell's news of a clone (`CloneProgress`, `CloneDone`) is refused from a client.
- **The line**: `owner/name`, `https://github.com/owner/name` (`.git`, a `/` after it) or
  `git@github.com:owner/name(.git)` is one row (`clone::parse`); anything else is none, and ↵ says
  what it takes. `owner`: letters, digits, `-`, not first or last, at most 39. `name`: letters,
  digits, `.`, `_`, `-`, at most 100, not `.` or `-` first, not ending `.git`: never `..`, a
  path, an option or a line of its own.
- **Where**: `<clone_dir>/<name>` (`clone::destination`), shown in full under the row before ↵.
  `clone_dir` is the setting (`~` expanded; relative: inside the ottospace folder), else `~/code`;
  a test run has none unless its own settings name one. Missing: ↵ says so, and the next ↵ makes it.
- **What runs** (the shell, `clone.rs`, a thread per clone, one clone at a time): refused before
  any process if the destination is there (a link too); then `gh auth status --hostname
  github.com` (`prs::gh`, the same gh lookup and stand-in: `OTTOSPACE_GH` only, in a test run);
  then a hidden staging folder beside it, `<clone_dir>/.ottospace-clone-<name>` (`clone::staging`:
  one volume, so the move is one rename), is made with `mkdir` (a crashed run's folder of that name
  is removed first; a link or file there is refused; its device and inode kept) and
  `gh repo clone <owner>/<name> <staging> -- --progress --no-recurse-submodules`, no shell, no
  standard input, `GIT_TERMINAL_PROMPT=0`, gh's prompts and colours off, no `GIT_DIR`-like
  variables, and `core.hooksPath=/dev/null` for that run only (`GIT_CONFIG_COUNT`, never written to
  the clone's config), in a process group of its own, at most 30 minutes. A repository there
  (`clone::after_gh`: gh ended well, no Esc, a `.git`) is renamed to `<dest>` with `RENAME_EXCL`:
  anything that came there meanwhile is never overwritten (there already). Nothing is at `<dest>`,
  and nothing listed, before that rename.
- **Progress**: gh's last line in the dialog (cleaned, one line), five times a second at most.
  Esc stops it: the group is killed and the staging folder goes, only if it is still the one this
  clone made (not a link, the same inode). A click outside hides the dialog; the clone goes on and its
  end is a toast.
- **The end**: made, it is listed as Local's add lists one (`repo_add`: the end of `repos`, its row
  selected) and said. Not: the dialog says why in plain words (`Failure::explain`: not logged in →
  `gh auth login`, not found, there already, the network, else gh's last line), or a toast if it
  is hidden; the staging folder this clone made goes.
- **Names are the user's data**: the dialog and the clone are never in the ordinary state (its line
  is a hidden field, `field_hidden`: the cursor, never the words); a clone's toasts name no
  repository or path (`Failure::public`, `clone::CLONED`), the dialog says them in full when it
  opens again; a test run's `clone` field shows them. Once listed, a repository is in `repos` as
  any the user adds.
- **A second provider** (GitLab, a plain URL) would be a `Provider` variant in the core, with its
  own `parse` rules and destination name, and a `Source` in the shell giving the argv that clones
  into an empty folder (`clone_argv`), and, with the list (next pull request), how it lists the
  user's repositories and which remotes are its. Nothing is configurable now.

## The model (core: `repos.rs`, `model.rs`)

- **A repository's name is a list row** (`ListRow::RepoHead`): ↵ or space on it folds or
  unfolds its rows; ← folds the selected row's repository, → unfolds it. List mode starts on the
  first checkout, as before.
- **Folded** (`repos_folded`, by path, kept in `layout.json`): its rows are not drawn and are not
  list rows; the name shows a count of its checkouts. A folded repository is shown unfolded
  while a live task in one of its checkouts runs or needs the user, and folded again when that
  ends. Folded by the user while one does, it stays folded (`repos_held`, not kept) until that
  ends. Anyone may fold (`RepoFold`): it changes what is drawn, not what runs.
- **The default folder**: the repository's own directory, the first row. `default` at its
  right says so (where a pull request's mark is not), and its tooltip. The dot no longer marks
  it: the dot is the row's activity (below). Nothing sets a default: there is no "default
  repository for new tasks".
- **Rows are one line**: the branch, cut short before what is drawn at the row's end (the task's
  badge), never under it. The path is the row's tooltip.

## The dot (`model/repo_dots.rs`, `SetHeads`)

The dot at a checkout row's left says what happens in that checkout (`Model::checkout_dot`).

- **Needs you** (amber, the needs-you colour): a task there not done needs the user (its group
  is Needs you; a snoozed one does not). Several: the first, by the Tasks section's order.
- **Working**: a task's agent works there (its turn runs, as its dial says, or the session it
  follows is active), in the task's identity colour; several, the one started last. Else a
  session no task has is active there (the Elsewhere notion: its file changed in the last 30 s),
  in the working colour. Sessions are read while the Tasks or the Repos section, a city or a
  session's space is on show, and every such read is one list (`SetSessions`).
- **Idle**: grey, every idle checkout alike, the repository's own folder too.
- The row's task badge keeps its letters and identity border, without its corner dot: the left
  dot says the state, not both.
- **A commit**: while the Repos section is open the shell stats each checkout's `HEAD` log
  (`<gitdir>/logs/HEAD`, a worktree's in `.git/worktrees/<name>`) about once a second, no `git`,
  and tells the core its length (`SetHeads`, `Origin::System`, with the time and Reduce Motion).
  A log that grew pulses the dot once: a ring widens and fades from it, 500 ms, one ease out,
  then the dot is as above. The first read of a checkout only records it, and the reads are
  forgotten when the section closes: a commit made meanwhile does not pulse on its opening. A
  checkout with no commit yet reads as an empty log, so its first commit pulses. Under Reduce Motion
  there is no pulse. Frames are asked for only while one plays (`counts.repo_pulse_frames`).
  Any `HEAD` move grows the log (a checkout of a branch, a reset, a pull): those pulse too.
- A folded repository draws no rows, so no row dots and no pulse: its name carries one dot after
  its count (`Model::repo_dot`), by the same rule over all its checkouts: amber if a task in any
  needs you (first by the Tasks order), else lit while any works (the task started last, in its
  colour; the working colour for a session no task has), else none: no grey dot on a quiet folded
  name. An unfolded name has none (its rows say it). It never pulses: a pulse is a checkout's,
  and folded means quiet. Sessions are read for a folded repository's checkouts as for any
  other's (the read covers every listed checkout), and still listed nowhere with Tasks closed.
  A checkout no longer listed keeps no read.
- The state's rows carry `activity` and `pulsing`, a repository its name's `dot` (`working`,
  `needs_you`, null); never a commit's message.

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
- The trash on a name (or a group's) stops listing the repository (`RepoRemove`): it changes `settings.json`,
  never anything on disk. While a live task works in one of its checkouts it asks first (the shared
  confirm, `Confirm::unlist`, the user's to answer); after, the status bar says, as information,
  "Unlisted <name> · nothing on disk changed" (`Effect::Told`).
- No argument is built from anything but the checked branch and the folder made here; git runs
  as an argv, never through a shell.
