# Names drawn in the city

Asked: "is there a reason we do text rendering in gpui and not in the wgpu city renderer? the
in-renderer text seemed to work pretty well for the web code city. and i'm thinking it'd be
better for the folder names to be in the plinths". The original city paints its top two folder
tiers on the terrace wall that faces the camera ([original.md](original.md), "Labels"). Then:
"can we move file names to the renderer as well?"

State: slices 1 (top folders, #545) and 2 (deeper folders and files, #549) **built**, behind
`city_wall_names` (Experimental, off; with `code_city`). With it on GPUI draws no name of the
city but the hovered box's label.

## Why in the renderer

GPUI's names float over a picture made with another frame's camera: on the live layer they
wait out a turn (#534), on the image path a drag's pictures are unnamed. A name the renderer
draws is part of the picture: it turns with it in the same refresh, a building in front hides
it (the depth test), and it costs no GPUI shaping or layout per frame.

## What the web city settled

The owner's web city (`codebase-visualizer`: `DESIGN.md` "Hierarchy legibility",
`viewer/src/terrace.ts`, `viewer/src/labels.ts`) iterated to this, and #549 follows it after a
first try put file names on roofs at 0.8 of the roof (a file's name larger than its folder's:
"it doesn't make sense for filenames to be larger than" folders):

- **Font size conveys hierarchy.** Only the top two folder tiers are signs on their terrace wall.
  Sized on the wall alone a top-level name is about 10 px at the overview, so the top tier holds an
  apparent size: it grows (to at most 2.2× its wall, standing on the wall's foot so it never sinks
  into the plate below) until its letters read about 15 px, and shrinks back to the wall near in.
  The second tier holds 11 px, at most 1.6× (its overgrowth rises in front of its own buildings),
  in a brighter colour. Deeper tiers never outgrow a wall: a sign over the terrace above reads as
  broken letters, so they are not walls at all.
- **Everything else is a map label.** Deeper folders and files: screen-facing labels over their
  box's top at a constant size a tier (folder 26 px, file 18 px pills), picked like a map's: the
  largest on screen first, none over another, at most 50, re-picked every 150 ms and frozen while
  the camera flies. Identifiers in their own case, folders in capitals.

## Decided

- **Walls.** Tiers 1 and 2 (`city_walls::WALL_TIERS`), each on the side of its plinth that faces
  the camera most and holds the name (a long thin plinth's short end may not: then its long
  side). In capitals. The em is 0.8 of the wall's height, grown while that is smaller on screen
  than the tier's size (capitals 15 points tall for the top tier, at most 2.2×; 11, at most 1.6×,
  for the second: the web's px, as points; capitals 0.7 of the em, so ems of 21.4 and 15.7), then
  less to fit the wall's length with half an em clear at each end. Grown past its wall it stands
  on the wall's foot. Smaller than 12 points (`TEXT_XS`, GPUI's size for them) or squeezed edge on:
  not drawn. Top tier in the text colour, the second a step dimmer, both brighter than labels.
- **Billboards.** Deeper folders (capitals) and files (as typed): facing the screen on a dark
  pill (the background at 0.78, its ends round; half an em past the name each side, 0.275 em
  above and below, as the web's 44 px text on a 68 px pill), the pill 2 points over their box's
  top, an em of 12 points a folder and 10 a file (`FOLDER_PT`, `FILE_PT`:
  GPUI's sizes for them; the web's 26 and 18 px are pills, their text 44/96 of that, 12 and 8). So
  the sizes fall by tier: 21.4, 15.7, 12, 10. Picked by `city_labels::place`, as GPUI picked them:
  a box from 40 points to 45% of the view, the largest first, no pill over another, at most 50,
  those named last kept first. Picked again for each picture at rest; while the camera moves the same
  ones only move (`billboards(.., moving)`), and none while the city rises. The order is one pure
  function, so the review layer can add a priority (changed files) later.
- **Occlusion.** A wall's name is depth tested against the boxes, writing no depth, drawn after
  the solid boxes and before the glass: a building in front hides it, a glass one dims it. A
  billboard is drawn last, on top of everything, as the web city's labels (depth tested, a deeper
  folder's label was clipped by its own buildings); where a box stands between the camera and the
  top it names (`city_walls::hidden`) it is drawn at 0.4 of its opacity, pill and letters.
- **Both paths.** The image-painted picture and the live layer draw the same names; a drag's
  half-size pictures keep them. A picture is made only when it was before: no render at rest.
  None behind panes (`city_behind`), as GPUI's.
- **GPUI.** With the flag on, GPUI names nothing in the city (no `Mark`s are made). The hovered
  box's label and beam (it always names the hovered building), agents and their titles, and the
  hints stay GPUI's. Billboards do not avoid the hints yet: the picture's thread does not know
  where they are.
- **One flag.** `city_wall_names` covers both slices.
- **Privacy.** Names are drawn only. The state's `city.names` lists each one the renderer drew,
  `plinth: true`, its `tier` (`wall`, `folder`, `file`), its `rect` (a wall's em box, a
  billboard's pill) and `hidden` (a shown box, standing, between the camera and what it names: a
  wall's name's middle, a billboard's box's top; `city_walls::hidden`, worked out only for a
  test run's state); never the name.

## How

- Core, pure: `ottospace_core::city_walls::signs(scene, wide, view, scale)` picks the walls,
  fits and sizes each name, and gives its em box in the world and on screen;
  `billboards(scene, wide, view, scale, kept, moving)` builds `city_labels` candidates and places
  them. `wide` is the name's width in ems, measured by the shell: for a wall only with room for a
  name an em wide, for billboards only of the 200 largest boxes. Unit tests: the wall faces the
  camera and reads left to right, grows far out and not near, the second tier less, shrinks to
  fit, the least size, tiers, LOD, edge on and from above, a tower in front, no measure for a
  plinth too small; sizes strictly fall by tier; a billboard holds its tier's size and a file's is
  smaller than its folder's; picked billboards never overlap and a move keeps them.
- Shell, `city_text.rs`: a line shaped with rustybuzz (kerning), its glyphs' outlines read with
  ttf-parser (`rustybuzz::ttf_parser`), each glyph's signed distance field (48 texels an em, 6
  texels each way) computed from the outline's segments, packed on shelves in one 1024² `R8Unorm`
  atlas. A glyph is rasterized once, when a line first needs it; a line is shaped once. Every
  printable ASCII glyph (95) takes 218 of its 1024 rows: no 2048² needed. Full (a repository
  named in many scripts), the atlas starts again after that picture's draw.
- Shell, `city_gpu.rs`: two text pipelines, depth tested and on top, made at the first line
  measured (none with the flag off): a quad per glyph, and per label a pill's quad worked out as a
  rounded rectangle in the same shader, its field sampled and smoothed over a screen pixel (`fwidth`), so it is crisp
  at any size and angle; fogged as the boxes are.
- Font: SF Compact (`/System/Library/Fonts/SFCompact.ttf`, semibold on its `wght` axis), else
  Helvetica Neue. ttf-parser 0.25 reads no outline of SF Pro's `SFNS.ttf`. A name with a glyph the
  font lacks is not drawn; one with no room left in the atlas, not in that picture.
- Dependencies: `rustybuzz` 0.20.1 (MIT), already in the lock under GPUI's SVG stack (usvg), with
  its `ttf-parser` 0.25.1: no new crate, nothing for `cargo deny` or the cooldown to judge. No
  MSDF: the plain field rounds a corner only past about 8× its atlas size, larger than a name ever
  draws, and needs no generator crate.

## Measured

`a_turn_is_timed` (release, `CITY_WALLS=1` on), a turn's 240 half-size pictures: the scene stage
(which now places the names) rises from about 0.08 to 0.15 ms at 9,500 files and from 0.43 to
0.7 ms at 50,000; the first picture at rest pays about 5 ms once (the font read, the glyphs). The
GPU's part is within the run-to-run noise.

Slice 2: see the pull request (#549).

## Next

1. Names fade in and out rather than pop (the web city cross-fades them).
2. Billboards clear of the key hints.
3. A priority in the billboards' order: changed files first, with the review layer.
