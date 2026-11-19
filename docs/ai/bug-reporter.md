# Bug reporter: mark what is wrong in ottospace, and hand it to an agent

Built from Bryan's words (2026-11-18):

> if i see a bug in ottospace, ideally i could highlight something wrong, add a comment, and spin
> up an agent

It behaves like the comment tool on a design canvas: box what is wrong, type a comment beside it,
as many as needed, then send. It builds on [capture.md](capture.md): the same frozen picture, the
same fenced brief, the same new-worktree path the new-task box takes.

## The flow

1. **The setting.** `"developer": { "ottospace_repo": "~/code/ottospace" }` in `settings.json`.
   The bug button shows only while that folder is there and is a git checkout (`.git`, a folder or
   a worktree's file). Applied at once when the file is saved.
2. **The button**, in the top bar's right end beside Fill the Whole Display; its key is in its
   tooltip only.
3. **The click.** ottospace takes a picture of its own window as it is, before anything changes:
   the window alone, by its id, with ScreenCaptureKit (as `record.rs` films it). Then the report
   starts over it; a live app is parked. Never the screen,
   never another app's window. The picture of a hosted app pane in it (its stream, drawn by
   ottospace) is blanked unless the user ticks "Include app panes".
4. **Marking.** Over the frozen picture: drag a box round what is wrong, or "Whole app"; a comment
   box opens beside the mark and takes the keyboard. Several marks, a comment each. A click in a
   box goes back to its comment; × removes a mark. ↵ ends a comment; Esc cancels the report; ⌘↵
   or Send sends it.
5. **Send.** A task in a new worktree of the repository, with the user's default agent
   (`default_agent`), by the new-task box's "new worktree" path: git makes
   `<worktrees_dir>/<repo>/<branch>` from HEAD on a new branch named for the first comment
   (`bug-the-tab-row`), and the agent starts there once it is made. The repository need not be in
   Repos. The task has its own workspace, its conversation in the drawer.

## The brief

`~/.ottospace/materials/<task>-<word>/` (folders 0700, files 0600), as a capture's:

| File | What |
|---|---|
| `report.png` | The window as frozen, app panes blanked unless shown |
| `report.annotated.png` | The same with the marks drawn on it |
| `brief.md`, `brief.json` | The brief, for the agent, and its structured form |
| `1.json` | The marks: each one's rect (0..1 of the picture) or the whole app, and its comment |

`brief.md` holds:

- the picture with the marks (both paths), and whether app panes are blanked;
- each mark's rect and the user's comment, as the user's words;
- inside fenced blocks, as data: the build (`ottospace 0.1.0 (a4a2582c0ffe, …)`), the OS version,
  the window's size, and a structure snapshot of what lay under each mark: control ids, pane
  numbers and kinds, rects in points. No text: a control's id keeps its kind and numbers
  (`task:3`, `top:fill`), never a path or a name (`repo:…`, `project:…`). No titles, no
  transcript, no meeting titles;
- inside a fenced block, the event log's last 50 lines, each its event name and numbers only (no
  time, no text).

Everything read of ottospace is fenced and labelled untrusted exactly as capture briefs are
(`<<<captured WORD …>>>`, every line `| `, a made-up WORD), and `brief.json` carries it under
`captured` with `"untrusted": true` and a fixed warning.

## Privacy

The frozen picture is a picture of the window: it can show meeting titles, a transcript, a
terminal's text, anything ottospace draws at that moment. It is the user's own act and stays on
this machine, in the materials folder, for the agent they start. App panes (other apps' windows)
are blanked by default. The structure snapshot and the log hold no text.

## What is in the core, what in the shell

- `ottospace_core::bug`: whether the button shows (`repo_of`), the marks (`Report`), what lay
  under each, what is blanked (`blanked`), ids kept without text (`control_id`), the log reduced
  to names and numbers (`events`), the brief and its markdown.
- `ottospace_core::model::bug`: the origin rules, the report over everything (what is behind it
  waits), Send's task and worktree.
- `ottospace/src/bug.rs`: the setting on disk; the picture, the overlay, the files.

## Trust

- **Only the user** starts, marks, comments, sends or cancels a report: their click, key or menu
  item. The core refuses each from any other origin (`Model::refusal`, held by the property tests
  for every origin), so a client's key, click or sneak label does none. A client's request that
  would take the keyboard while a report is open is refused; the user's own pane keys wait behind
  it. Only the shell says the picture is taken and what the setting names.
- `bug_start` on the socket is for tests: the user's key pressed by the test, only under
  `OTTOSPACE_E2E` (`control::test_refusal`). Not in `skill/guide.md`.
- The picture is ottospace's own window, by its window id. A test run takes it of its own window.

## Decided here

| Question | Answer |
|---|---|
| The setting | `developer.ottospace_repo`: a checkout; nothing shows without it |
| A mode of its own | No: annotate mode, as a capture's picture. The field is the comment |
| Esc while a comment is typed | Ends the comment; Esc again cancels the report |
| No comment typed | The task is "Bug report", in fixed words |
| No folder for worktrees | Nothing is filed; the notice bar says so and the report stays |
| The repository not in Repos | Its worktree is made all the same; it is not listed |
| No picture (no Screen Recording) | The report works over the window as it is; the brief says no picture was kept |

## Next

- The drawer's Brief view shows a capture's brief; a bug report's shows its `brief.md` text only.
- A report that survives a restart.
