# What the original code city decided, and what ottospace takes

The reference is the owner's earlier web code city, `codebase-visualizer` (three.js). Its
design thinking is good; its surrounding interface (file viewer, panels, HUD) is not wanted: in
ottospace the city stays quiet, with at most one small control and a legend line, everything else
through hover, keys and the palette (the owner's word, 2026-11-21).

Each idea, one line on what it does and why it works, and the call: **bring over**, **have**
(ottospace already does it), or **leave**. The owner corrects this list once; the pull requests
follow it.

## Layout and look

| Idea | What it does, why it works | Call |
|---|---|---|
| Terrace steps 13, 8, 4.6, 2.6, 1.7, then 1.1 | Each folder level rises a step, the steps shrinking with depth: nesting reads without towering | have |
| Building height `2 + 6·log2(1 + lines/10)`, 2 to 60 | Log height: a huge file does not flatten the rest | have (lines guessed from bytes) |
| Walls down to the parent's surface | A terrace reads as a solid step, not a floating slab | have |
| Streets narrowing with depth (`14 − 3·depth`, 2 to 14) | Top-level districts stand apart most | bring over (we use one street) |
| Plate rim outlines, brighter for outer folders | Edges carry the hierarchy, not fills | have (edge lines, dimmer per tier) |
| Buildings 0.92 opaque, interfaces 0.45 glass | See-through enough to read what is behind | bring over (glass, #393) |
| Folders too small to lay out drawn as one translucent mass | Level of detail without holes | bring over when a city is too large |
| Fog, dark background, a key light and a violet fill | Depth and mood | have (fog, one light); leave the fill and bloom |

## Labels

| Idea | What it does, why it works | Call |
|---|---|---|
| Chosen by on-screen size: kept when 40 px to 45% of the view tall | Labels follow zoom: districts far out, files near in | bring over |
| Largest first, greedy, no overlap, at most 50 | Deterministic, the important names win | bring over |
| Constant pixel size per tier (folder 26, file 18, item 14) | Legible at any distance | bring over (our sizes) |
| Top two folder tiers painted on the terrace wall that faces the camera | Names sit on the thing they name, not floating | bring over (as a later step; first, edge labels) |
| Re-picked every 0.15 s, cross-faded; frozen during camera flights | No flicker as the camera moves | bring over with the camera |
| Leader lines from a parent's label to its children's, on hover | Shows nesting on demand | leave (noise) |
| Folder names in capitals | Tells folders from files | bring over |

## Hover and selection

| Idea | What it does, why it works | Call |
|---|---|---|
| Hover on a folder: an outline box round its whole footprint | Marks the folder itself, not a point in its middle | bring over |
| Callout on a leader line above the roof, drawn over everything | Never hidden by buildings | have (the beam) |
| Labels and wall signs tested before buildings | Readable text wins the pick | bring over |
| Click selects, double-click drills in, Esc steps back out | Zoom by intent, a way back | bring over (double-click: into a file; Esc out) |

## Camera

| Idea | What it does, why it works | Call |
|---|---|---|
| Orbit: left drag rotates, middle/right drag pans, wheel zooms, damped | The common 3D grammar | bring over (left rotate, middle pan, wheel zoom) |
| Pitch kept between about 5° and 89°; distance bounded | No flipping under the ground | bring over |
| An overseeing start: from a corner, well above, 46° field of view | Heights read against each other | bring over (the owner asked for "overseeing") |
| Flights along a spline at constant apparent speed, any input cancels | Smooth, never in the way | bring over for drill-in; instant under Reduce Motion |
| The camera not kept | | leave: ottospace keeps it per place |

## Layers

| Idea | What it does, why it works | Call |
|---|---|---|
| One `git log --numstat --since=12.months` pass, capped (1,000 commits quick, 20,000 full) | Every layer from one read | bring over |
| Churn: commits touching a file, square-root heat ramp grey → amber → orange → red | Hotspots stand out | bring over |
| Fix churn: commits whose subject says fix/bug | Where bugs cluster | bring over (cheap, same read) |
| Recent: touched in the last 30 days | What moves now | bring over |
| Strata: each file a stack of slabs, one per commit, sized by its lines then, coloured by commit type, with a timeline to rewind | History as height | bring over (owner asked, 2026-11-21): a layer on the layers' one reader, its own PR after them; the timeline its one extra control |
| Structure: buildings coloured by kind (function, class, ...) | Needs the file-level parse | later, with the file level |
| Provenance (a diff or PR), external metric file | Needs inputs ottospace does not have | leave |
| Working tree: modified files by added/removed balance, new files as construction, deleted as vacant lots, the rest ghosted | What is uncommitted now | bring over as a layer |
| People/PRs: an avatar over a PR's files with a light beam | Who works where | have, for agents (#395) |

## Load and performance

| Idea | What it does, why it works | Call |
|---|---|---|
| First load at once, no rising animation | Never makes the person wait for a show | have; a short fade over the lattice only (load PR) |
| Drill-in keeps the picked node in place and animates scale; the old city fades as a ghost | Continuity when the world changes | bring over with the file level |
| Instancing, recolour without rebuild, hit test only when the pointer moved | Cheap frames | have |

## Not taken

The file viewer, side panels, search box, breadcrumbs, HUD counters, tours and the export: the
city in ottospace is a backdrop and a map, not an application of its own.
