# Folder names on the plinths

Asked: "is there a reason we do text rendering in gpui and not in the wgpu city renderer? the
in-renderer text seemed to work pretty well for the web code city. and i'm thinking it'd be
better for the folder names to be in the plinths". The original city paints its top two folder
tiers on the terrace wall that faces the camera ([original.md](original.md), "Labels").

State: slice 1 **built**, behind `city_wall_names` (Experimental, off; with `code_city`).

## Why in the renderer

GPUI's names float over a picture made with another frame's camera: on the live layer they
wait out a turn (#534), on the image path a drag's pictures are unnamed. A name the renderer
draws is part of the picture: it turns with it in the same refresh, a building in front hides
it (the depth test), and it costs no GPUI shaping or layout per frame.

## Decided

- **Which.** Folders of tier 1 and 2 (`city_walls::WALL_TIERS`), each on the side of its plinth
  that faces the camera most and holds the name (a long thin plinth's short end may not: then its
  long side). In capitals, centred on the wall.
- **How large.** The em is 0.8 of the wall's height, less to fit the wall's length with half an
  em clear at each end. GPUI's size for those tiers (`TEXT_XS`, 12 points) is now the least: a
  name whose em is smaller on screen, or whose wall is seen so edge on that the name is squeezed to
  under half its width, is not drawn. Looking near straight down names none.
- **Occlusion.** Depth tested against the boxes, writing no depth; drawn after the solid boxes and
  before the glass, so a glass building in front dims it as it dims what is behind it.
- **Both paths.** The image-painted picture and the live layer draw the same names; a drag's
  half-size pictures keep them. A picture is made only when it was before: no render at rest.
- **GPUI.** With the flag on, GPUI names no folder of those tiers (`Mark`s of tier `Wall` are not
  made). Deeper folders, files, the hover's callout and beam, agents and the hints stay GPUI's.
- **Privacy.** Names are drawn only. The state's `city.names` lists each one drawn on a plinth,
  `plinth: true`, its em box's `rect` and `hidden` (a shown box, standing, between the camera and
  its middle: a solid one hides it, a glass one dims it; `city_walls::hidden`, worked out only for
  a test run's state); never the name.

## How

- Core, pure: `ottospace_core::city_walls::signs(scene, wide, view, least)` picks the walls, fits
  and sizes each name, and gives its em box in the world and on screen; `wide` is the name's width
  in ems, measured by the shell. Unit tests: the wall faces the camera and reads left to right,
  shrinks to fit, the least size, tiers, LOD, edge on and from above, a tower in front.
- Shell, `city_text.rs`: a line shaped with rustybuzz (kerning), its glyphs' outlines read with
  ttf-parser (`rustybuzz::ttf_parser`), each glyph's signed distance field (48 texels an em, 6
  texels each way) computed from the outline's segments, packed on shelves in one 1024² `R8Unorm`
  atlas. A glyph is rasterized once, when a line first needs it; a line is shaped once.
- Shell, `city_gpu.rs`: a text pipeline made at the first line measured (none with the flag off):
  a quad per glyph, its field sampled and smoothed over a screen pixel (`fwidth`), so it is crisp
  at any size and angle; fogged as the boxes are.
- Font: SF Compact (`/System/Library/Fonts/SFCompact.ttf`, semibold on its `wght` axis), else
  Helvetica Neue. ttf-parser 0.25 reads no outline of SF Pro's `SFNS.ttf`. A name with a glyph the
  font lacks, or with no room left in the atlas, is not drawn.
- Dependencies: `rustybuzz` 0.20.1 (MIT), already in the lock under GPUI's SVG stack (usvg), with
  its `ttf-parser` 0.25.1: no new crate, nothing for `cargo deny` or the cooldown to judge. No
  MSDF: the plain field rounds a corner only past about 8× its atlas size, larger than a name ever
  draws, and needs no generator crate.

## Measured

`a_turn_is_timed` (release, `CITY_WALLS=1` on), a turn's 240 half-size pictures: the scene stage
(which now places the names) rises from about 0.08 to 0.15 ms at 9,500 files and from 0.43 to
0.7 ms at 50,000; the first picture at rest pays about 5 ms once (the font read, the glyphs). The
GPU's part is within the run-to-run noise.

## Next

1. **File names in the renderer.** The same `city_text` and pipeline: a building's name on its
   roof or its face, `city_labels`' choice of which. Lowercase and digits fill more of the atlas:
   it may need to grow (2048²), and names under the least size fade rather than pop.
2. Deeper folders on their walls once they are large enough, in place of over their top.
