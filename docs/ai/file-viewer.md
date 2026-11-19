# File viewer

A pane that shows a file, read only. Owner-approved plan: A the viewer, B a markdown fixture
(built); C a markdown pass (built); D an explorer, E comments (later).

## Built (A, B)

- **A pane kind**, `Kind::File { path }`. Its view is the model's `viewers` by pane
  (`viewer::Viewer`): `shows` (by the name: `markdown`, `code`, `text`, `image`), `read` (what
  the shell found: `shown` with its lines, `missing`, `too_large`, `not_file`, `binary`,
  `unreadable`), `top` (the first line on show), `most`, `page`, `pending` (a count typed) and
  `opener`. Never the text. `settle_viewers` keeps one view for each viewer pane after every step.
- **Opened from** ⌘E's File… (`PickItem::File` → `Effect::ChooseFile` → the open panel →
  `OpenFile` as the user's; the open panel, not a path field: no completion to build and
  macOS's own recent places and search), ⌘-click on a path in a terminal (`viewer::path_at`,
  `viewer::absolute`, then the shell resolves links and checks it is a regular file) or on a file
  link in the conversation or in a viewed file, and `open_file` on the socket.
- **Reuse** (`OpenFile { reuse }`, for the explorer): the viewer a pane opened with `reuse`, in
  that pane's tab, shows its next file; the keyboard stays on the opener. Without one, or once
  it moved to another tab, a new viewer opens beside it. A terminal's ⌘-clicks use it.
- **Reads** on a thread: opened with `O_NONBLOCK` (a pipe never blocks), then judged by `fstat`
  of what was opened, so only a regular file of at most 5 MB is read. A NUL in the first 8 KB, or
  text that is not UTF-8, is `binary`; a picture is known by its first bytes.
- **Reloads**: the file's mtime, size and inode looked at twice a second; a change read once it
  has held still for 0.2 s (an editor's save may write twice). The core keeps `top`, so the view
  stays at its line.
- **Keys** (`vim::Keys`, reusable by the explorer): a count before a motion (`10j`, `3G`, `12gg`),
  `0` alone no count, capped at 99,999, Esc takes it back, any other key drops it. `vim::moved`
  says where a motion goes. ⌃G or Edit: `Effect::EditFile`, the user's editor in a terminal beside
  the viewer, run as Open Settings runs it (`Terminal::edit`).
- **Draws**: markdown by the conversation's renderer, shared (`conversation::Links` says whose
  links: a chat's, judged in its repository; a viewer's, in its file's folder), with larger caps
  (`markdown::FILE`); code and text as a virtual list of lines with numbers; a picture.
- **At lines** (`OpenFile { at }`, `viewer::Lines`, for evidence): from 1, both ends in; line 0
  is none, a range written backwards the same lines. `Viewer::mark` puts the first `CONTEXT` (3)
  lines below the top and keeps `marked`; once read it is cut to the file (past the end: the last
  line; an empty file or a picture: none). A viewer of the file in `from`'s tab moves and
  re-marks instead of a new one (`viewer_of`); drawn markdown switches to its source (`shows:
  code`) and is read again, since a rendered row is no file line. The mark goes with another
  file, other lines, the pane, or Esc with nothing typed. Typed: `viewer::split_lines`
  (`a.rs:12`, `:12:3`, `:12-14`), for a terminal's ⌘-click; a chat link drops it (left). The
  shell tints the rows and draws a bar in `theme::MARK`; a test run's `marked_rect` says where
  the first is drawn. Not kept in `layout.json`.
- **Syntax** (`syntax.rs`): a code file whose extension has a grammar (`syntax::lang`: Rust,
  TypeScript, TSX, JavaScript, JSON, TOML, Python, shell) is parsed by tree-sitter on the read
  thread, at most 2 MB (`MAX_PARSE`) and 0.5 s (`BUDGET`, a progress callback that stops it);
  else `highlight`'s lexer, as before. The tree is kept with the view; the lines drawn are
  coloured 128 at a time (`CHUNK`, a query cursor over their bytes), each chunk once. A capture's
  name gives its token (`syntax::token`: keyword, string, comment, number, type, function,
  punctuation, key), its colour `highlight::colour`. Of two captures of one node the query's
  first wins, an inner node over an outer. The state's drawn `syntax` names the grammar.
  Markdown stays the renderer's.
- **Kept** in `layout.json` (and marks, workspaces) as `RecordKind::File { path, top }`.
- **B, the fixture**: `crates/ottospace/tests/fixtures/markdown/fixture.md` and `checker.png`.
  Beside the tests, not in `docs/`: the renderer's unit test includes it, the e2e tests copy it,
  `scripts/shots/viewer-markdown.jsonl` opens it at six places; and a change only under `docs/`
  runs no CI, so a change to the fixture would show no pictures.

## What the renderer gets wrong in the fixture (input for C)

See the pull request that built this: headings all one size, checkboxes as `[x]` text, table
alignment ignored and a wide table squeezed, no syntax colours, callouts and footnotes as text,
bare addresses not links, a picture drawn twice (its link and the thumbnail), raw HTML leaves
blank lines.

## Next

- **C, markdown pass** (built): the list above fixed in `markdown.rs` (structure: heading levels
  and slugs, task boxes, table alignment and widths, callouts, footnotes, bare addresses, Slack
  mentions, `#anchor` links); `highlight.rs` colours fences by label and code files by extension;
  `picture.rs` sizes thumbnails. Left: GPUI wraps a long address before any `/`.
- **D, explorer**: a tree pane of a folder; `j`/`k` with `vim::Keys`; each file it moves to goes
  to `OpenFile { beside: explorer, reuse: true }`.
- **E, comments**: `V` in the viewer starts a line selection (a `Keys` motion extends it); a
  comment is kept by path and line range, never in the file.
