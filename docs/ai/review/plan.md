# Reviewing a change (design)

The owner, 2026-11-25: "a key thing i'd like to explore is how we can make diffs easier to review
and explore with ottospace, with code city / our own file viewer / diff viewer / combination";
"if the branch / worktree is associated with a pr, we should highlight those changes"; "extend our
file viewer to handle a diff view".

The loop is delegate, wait, decide, verify. Review is the verify step, and today it happens
somewhere else: GitHub in a browser, `git diff` in a terminal. Here it happens in ottospace: the
city says where a change is, a list says what to read next, and the viewer shows each file's diff.

## What exists to build on

| Piece | Where | What it gives |
|---|---|---|
| A worktree's open PR | `ottospace-core/src/prs.rs`, `ottospace/src/prs.rs` (`gh pr list`, read-only, fixed argv) | number, `baseRefName`, `headRefOid`, checks, review decision, matched to a checkout |
| git's marks per file | `explorer_pane.rs` (`git status --porcelain=v1 -z`, hardened by `child::git_read`) | `M`/`A`/`U`/`!` per path; a folder carries the strongest |
| Diff lines as data | `ottospace-core/src/diff.rs` (`Sign`: context/removed/added/hunk; cut, cleaned, untrusted) | the drawer's diff cards; the model a viewer can reuse |
| A file in a pane | `ottospace-core/src/viewer.rs`, `model/viewer.rs`, `ottospace/src/viewer.rs` | lines, syntax colour (tree-sitter), marked ranges, vim moves with counts, reload on change |
| The explorer | `model/explorer.rs`, `explorer_pane.rs` (#510) | a checkout's tree; ↵ opens a file in the viewer beside (`OpenFile { beside, reuse }`) |
| The city | `model/city.rs` (`City { lit, agents, glimpses }`), `scene.rs` (`Kind::Lit`), `city.rs` | lit buildings with beams in a colour, hover beacons, levels of detail, glimmers for reads |
| Go-to ⌘P | `goto.rs` (both crates) | files of the checkout on show, ranked |
| Git layers, in flight | #481 (footprint by lines, height by commits), #485 (strata: a slab a commit); draft, conflicted | one `git log` reader; `original.md` plans a "Working tree" layer |

## The loop as lived

1. **Something needs review.** A worktree's PR is green and awaits the user, or an agent says
   "done" on a task. It joins `needs you` (⌘J), one queue, never a ping of its own.
2. **Open it.** On the Repos row or the task: **Review** (⌘⇧D in the tab). The tab shows the change:
   the city of the checkout with the changed files lit by kind, beside a list of the changed
   files ordered for reading.
3. **Read.** ↵ on a file, or a click on its building: the viewer beside shows its diff, at its first
   hunk. `]c`/`[c` next and previous hunk, `]f`/`[f` next and previous file (the list follows,
   the city's beam moves to that building). A read file is ticked in the list and dimmed in the city.
4. **Note.** `V` selects lines in the diff, `c` writes a comment. It's kept by path and line,
   never in the file or on GitHub (file-viewer.md's slice E).
5. **Decide.** The list's foot: **Send notes to the agent** (the task's agent gets the comments
   as one message, quoted), **Approve in browser** (opens the PR; GitHub writes stay the
   user's), or **Done**. A reward marks the decision, never the reading.

## Recommendation: one review place, three views of one diff

One model of the change (`review::Change`: files, each with its status, `+`/`-` counts and hunks),
read once, seen three ways that stay in step.

- **City, the map.** A `changes` layer lights changed buildings by kind: added green, modified
  amber, removed red (a vacant lot), renamed a beam from its old lot. Height is unchanged
  (strata/commits stay their own layer). Unchanged buildings dim to glass. The PR's own
  files only: the change from the merge base, not the whole tree. A changed folder that is
  closed (levels of detail) glows in the strongest colour under it, as a lit file does today.
- **List, the order.** The changed files grouped by folder, ordered for reading: tests next to
  the code they test, generated and lock files last and folded (`Cargo.lock`, `*.snap`,
  `package-lock.json`), the biggest change first within a folder. Each row shows `+n −m` and
  a tick once read. It's a pane (`Kind::Review`), the explorer's sibling, with the same keys.
- **Viewer, the reading.** A diff mode of the file viewer: inline by default (the drawer's
  colours, with syntax colour kept on each line), side by side when the pane is wider than
  160 columns, `s` to switch. The hunks fold the unchanged runs (`… 40 lines`, ↵ opens them).
  It's the same pane, keys and line numbers (new side), so ⌃G edits the file at that line.

Why this combination: the city answers "where, and how much" at a glance (the attention rule:
the person never hunts); the list answers "what next"; the viewer answers "what exactly". Each
alone is weaker: the city can't show a line, and a list doesn't show the shape of a change.

## Data: where the diff comes from

- **Local git only, no network.** For a checkout with a PR:
  `git diff --merge-base origin/<baseRefName> HEAD` (`-z --numstat` for the list, then
  `--patch -U3 -- <path>` per file as it's opened), hardened by `child::git_read`, with a fixed
  argv; the base name comes from `prs.rs`, checked as a ref name (`git check-ref-format`), never
  a path. With no PR: the merge base with the repository's default branch, and the working tree
  (`git diff HEAD`) as a toggle.
- **`gh pr diff` is not used.** CLAUDE.md allows gh only for `gh pr list` (and the clone's fixed
  argv). A new gh call would widen the network rule for something local git already has. The
  cost: a stale local `origin/<base>` shows a slightly older merge base. The list's head says
  "base as of <fetch time>". The fetch is the user's; ottospace never fetches.
- **A PR not checked out** (someone else's): later. It needs a fetch or `gh pr diff`, both new
  network. Question 1.
- **Cost.** `--numstat` of a 1,000-file PR is milliseconds; a file's patch is read when it's
  opened, capped (5 MB as the viewer, 10,000 lines a file; past that "too large, open in
  terminal"). The city's layer is a recolour of the boxes it has (`Boxes`), no new layout.
  It's re-read when `HEAD` or the base moves, or on the explorer's status tick.

## Core vs shell

| Core (`ottospace-core`, tested) | Shell |
|---|---|
| `review.rs`: parse `--numstat -z` and a patch into `Change`/`Hunk` (reusing `diff::Sign`, `transcript::clean`), the reading order, hunk navigation (`next_hunk(from)`), folding | run git, fixed argv, off the main thread |
| `Kind::Review` pane, `Action::ReviewOpen { checkout }`, `ReviewRead`, `ReviewStep { hunk/file, dir }`, read ticks, comments by path and line | draw the list, the diff lines, the city's colours |
| The city's `changes` layer: which places light and in what kind (`City.changes`) | recolour in `Boxes`, beams for renames |
| What joins `needs you`: a PR green and waiting, a task done with a change | none |

Every action has an `Origin`: only the user opens a PR in the browser or sends notes to an agent
(the client's request is refused, as `USERS_ALONE` today).

## Slices (each a PR on its own, shippable, flag `review`)

1. **Diff in the viewer.** `review.rs` parses one file's patch. `OpenFile { diff: Some(base) }`
   opens the file's diff: inline, hunks, `]c`/`[c`, syntax colour, folded runs. The entry is the
   explorer: on an `M` file, `d` opens its diff (from `HEAD`, the working tree's change). Tests:
   core unit (parse renames, binary, no newline at end, hunk steps); e2e: a fixture repo with a
   change, `d` in the explorer, the state's `viewers[n].diff` (hunks, the line on show, never the
   text); shot `viewer-diff`.
2. **The change list.** `Kind::Review` for a checkout: `--numstat` from the merge base (PR base,
   else the default branch), the reading order, ticks, ↵ into slice 1's viewer, `]f`/`[f`. The
   entry is **Review** on a Repos row and in the palette. Tests: core order and ticks; e2e with
   `OTTOSPACE_GH` fixture PR (its `baseRefName`), a local repo with a merge base; shot
   `review-list`.
3. **The city's changes layer.** The review tab's city (and, behind the flag, any city of a
   checkout with a PR) lights the change by kind; a click on a lit building opens its diff; the
   list's cursor moves the beam. Changed files are named first (label picking's priority). Tests: core (which places, which kind, a closed folder's glow);
   e2e (the state's `city.lit` kinds and counts, a click opens a diff viewer); shot
   `city-changes` before and after.
4. **Side by side** (`s`, wide panes), and the working tree as a toggle.
5. **Comments** (file-viewer.md's E) and **Send notes to the agent**.
6. **`needs you`**: a green PR awaiting the user, or a task done with a change, joins the queue
   and opens the review.

Slices 1 to 3 need nothing of #481/#485. The changes layer is independent of strata: they share
the city's colour path, not its reader.

## Decided (the owner, 2026-11-25)

1. The PR under review is checked out: a worktree. No fetch, no `gh pr diff`.
2. Inline first; side by side later.
3. Comments stay local, handed to the agent. Nothing posted to GitHub.
4. Unchanged buildings dim to glass during review.
5. The list by folder, tests beside their code; commit order later, on the strata reader.
6. The diff from the merge base; a commit-by-commit step later.

The questions as asked, with the picks the owner took:


1. **Someone else's PR, not checked out?** Pick: not yet. Review a checkout (a worktree
   makes one in a second, Repos' +). Fetching or `gh pr diff` widens the network rule.
2. **Inline or side by side first?** Pick: inline (reuses the viewer's one column and the drawer's
   colours); side by side in slice 4.
3. **Comments: local, or posted to GitHub?** Pick: local, and handed to the agent. Posting is a
   write to GitHub; it stays the browser's.
4. **City during review: dim the unchanged, or hide them?** Pick: dim to glass. The shape of
   the codebase is the point of the map.
5. **Order of the list: by folder, or by the commits' order?** Pick: by folder with tests beside
   code; a toggle for commit order once strata (#485) has the commit reader.
6. **Which diff: the merge base, or the PR's commits one at a time?** Pick: the merge base. A
   commit step (`]C`) later, on the strata reader.
