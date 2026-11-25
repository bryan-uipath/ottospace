# Code city: detail as you zoom in (design)

The owner, 2026-11-23: "when we zoom in, if we can show more detail (like the internals of files,
kind of like code city)"; 2026-11-24: "LoD makes sense", "the challenge is making things stable
and avoiding jump when transitioning lods", "lod applies to in files too". One mechanism, one
ladder: folder block → file building → the file's parts. Each level appears as the camera nears.

## The ladder (built: `ottospace-core::lod`)

- **One layout for every level.** The plan is laid out once; a level only shows or hides boxes
  inside an outline that was there already. A box's parent is the plinth of the nearest folder
  that holds it (a folder of one child has none). Nothing is file-specific: a box with children
  opens, a leaf has no level, so a file's parts are one more rung (their parent the building).
- **Closed, a box stands for what it hides:** raised by its leaves' heights above it, mean by
  area (one solid block the size of the folder's ground, as tall as its files are on average:
  the silhouette changes least), in a lit file's colour if it hides one. An agent at a hidden
  file is at that box (its icon, its beam); hover over it names the folder.
- **Per box, by size on screen, with hysteresis.** A box opens when its children's lots would
  be 3 px a side (`OPEN_PX`), closes under 2 px (`CLOSE_PX`). The size is taken by the distance
  from the eye to the point looked at, widened by the box's distance from that point on the plan:
  turning round that point changes no level. A box changes at most once in 300 ms (`HOLD_MS`);
  a change that waits is made when it may, with pictures asked for until then.
- **No jump:** an opening box's children grow out of its closed top over 200 ms (`FADE_MS`)
  while it sinks to its plinth; closing, the reverse. At once under Reduce Motion.
- **Tested** (core): no box moves or resizes on the plan at any level or moment of a fade; every
  box lies inside its parent; a full turn changes no level; a camera wobbling in and out changes a
  box at most twice and never within the hold; children start inside their closed box; a closed
  box hiding a lit file is lit.
- Thresholds were measured: 6 / 4 px merged half of a 9,500-file city at a fitted 2560×1500
  view (5,306 of 10,392 boxes drawn); 3 / 2 px keeps 80% (8,350) and still draws 28,138 of 618,858
  at 600,000 files.

## The file's parts (decided 2026-11-24, not built)

| Choice | Decision |
|---|---|
| Where symbols come from | tree-sitter. The grammars are already linked for the viewer's highlighting (`syntax.rs`): Rust, TypeScript, TSX, JavaScript, Python, Go, JSON, TOML, YAML, Markdown and 13 more. No new dependency, licence, cooldown or binary size; the shell crate, as today (`ottospace-core` stays pure: it gets parts as data, `(kind, line span)`, never a name in the state) |
| First languages | Rust, TypeScript/TSX/JavaScript, Python, Go; Markdown headings; JSON/YAML top-level keys |
| A language with no grammar, a parse error or a timeout | the plain building: no approximation |
| Kinds and colours | a fixed set: function/method, type (struct, enum, trait, interface, class), test, comment/doc, import/use, other. Each a stable colour clear of the folder colours and of amber ("needs you"), told apart by more than hue (a legend on hover; parts in source order, bottom to top) |
| Size | lines: a part's share of its building |
| When read | only files near the camera (their building opening, the rule above), off the main thread, at most 1 MB a file, never in excluded folders, never the network |
| Cache | by content: git's blob id for a tracked, unmodified file (free from the index: nothing read to know a parse is still good), a hash of the contents otherwise; with the grammar's and query's version. In memory always; on disk under ottospace's home only the outline (kind, line span, symbol name), 0600, bounded with eviction, never the text; a test run writes it only inside its own `OTTOSPACE_HOME` |
| An agent's edit | lights what it touched at the level on show: the part whose line span holds the edit's lines (the diff a tool event carries), else the building, else the closed folder. A read lights the same way, quieter (the read flash); a search sweeps as today; in the task's colour for the lit rule's time |

Queries: Rust `function_item`, `struct_item`/`enum_item`/`trait_item`/`type_item`, `impl_item`
(its functions nested), `mod_item`, a function with `#[test]` or in `mod tests` as test,
`line_comment`/`block_comment` runs as comment, `use_declaration`; TypeScript/JavaScript
`function_declaration`, `class_declaration` (methods nested), `interface_declaration`,
`type_alias_declaration`, `enum_declaration`, `import_statement`, `describe`/`it`/`test` calls as
test; Python `function_definition`, `class_definition`, `import_statement`, `test_`-named
functions; Go `function_declaration`, `method_declaration`, `type_declaration`, `Test`-named
functions; Markdown `atx_heading`.

Measured (release, this Mac): tree-sitter parses a 2,000-line Rust file (36 KB) in 3.3 ms and a
1,600-line TypeScript one in 2.9 ms; hashing it takes 0.007 ms. The 9,500-file fixture (about 7 KB
a file) would take about 6 s to parse whole, plus about 0.45 s to read; with the blob-id cache a
reopen parses nothing. #481's line counts should use the same key: counted once per blob.

## Earlier notes (kept for the parts' layout; the size thresholds above replace the table)

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
