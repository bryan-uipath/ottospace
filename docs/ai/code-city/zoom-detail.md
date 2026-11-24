# Code city: detail as you zoom in (design)

The owner, 2026-11-23: "when we zoom in, if we can show more detail (like the internals of files,
kind of like code city)". This is the file-level zoom already in `plan.md` ("Inside a file"),
now driven by how near the camera is rather than only by a double click.

## What appears at which zoom

Level of detail follows each thing's size on screen, the rule names already use (`city_labels`:
40 points to 45% of the view):

| A thing on screen | Drawn as |
|---|---|
| a district under 40 points | its plinth only, no name |
| a district 40 points or more | its plinth, its name centred on it; its files as buildings |
| a file under about 120 points across | a building (today) |
| a file about 120 points or more | its top-level blocks as lots on its roof: a building of buildings |
| a block about 120 points or more | its nested blocks the same way, one level more |

A file opens up only when it is large on screen, so at most a handful do at once.

## Blocks

- **Source:** tree-sitter, already in the shell (`syntax.rs`, highlighting the viewer). The
  top-level items are the parse tree's children that are declarations: Rust `fn`, `struct`,
  `enum`, `trait`, `impl` (its `fn`s nested), `mod`, `const`/`static`, `macro_rules!`;
  TypeScript `function`, `class` (its methods nested), `interface`, `type`, `enum`, top-level
  `const`/`let`. Markdown headings were the original's cheap first case and stay one.
- **Languages first:** Rust and TypeScript, then the rest the highlighter knows.
- **Layout:** the same atlas, one level in. A file's blocks are lots on its roof in reading
  order, top to bottom. Footprint is by lines, so a 200-line `impl` outweighs a 3-line `const`.
  The building's height is shared out, so nothing towers through the roof. Stable: an edit
  inside a block moves no other block, the same promise the atlas makes for files.
- **Colour by kind**, as the original's `KIND_COLORS`: function cyan, class/impl blue,
  interface/trait violet, type slate, enum yellow (clear of amber, "needs you"), const grey, heading parchment.
  Muted as folder colours are, so a lit block (the agent's) still stands out.

## Cost and caching

- Parsed off the main thread, on the city's reader thread. Only for files that are large on
  screen now (the rule above), at most 8 at a time, each at most 1 MB.
- Kept per file, by its size and modification time, in a bounded cache (64 files): a file read
  again only when it changed. No parse at rest. The reader asks only when the camera brings a
  new file past the threshold.
- Smooth by construction: it builds on #437. A camera move lays nothing out again, so a block
  layout made once is reused for every frame, and only the picture is rendered.

## Double click

A double click on a building flies the camera to that file's detail level (its roof filling
about 60% of the view), eased, at once under Reduce Motion. Esc or Reset view flies back. Only
the user's click does it; a client's does nothing.

## Slice 1

- Rust and TypeScript top-level blocks only, one level, coloured by kind.
- They open past the size threshold. No double click yet.
- A core rule with tests: which files open at which on-screen size, and the block layout is
  stable under an edit inside one block.
- CI pictures at three zooms.

## Later

Nested blocks; more languages; double-click flight; an agent's icon over the block it edits
(the transcript gives a line range for Edit calls).
