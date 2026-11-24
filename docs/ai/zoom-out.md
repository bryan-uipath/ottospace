# Zoom-out: every tab of a workspace at once

The owner: "'zoom out': show all tabs in a workspace zoomed out with space between and other
artifacts (drag/drop files, draw shapes, annotate)", "a workspace is a json file", "'creative' vs
'executive' mode", "rail edge shadows evoke depth", "need a hotkey", "render the city visualizer in
the background", "animate zooming". The boards are "A" on the design canvas (A1 to A6).

Decided by the owner (2026-11-23): the key is **⌘⇧\\**; tabs sit on a **grid** first, free
placement later; build in three slices. This note is slice 1's design, and the order after it.

## What it is

Executive mode works in one tab: today's ottospace. **Zoomed out** (creative mode) the panes area
shows every tab of the workspace or task on show as a tile on a grid, with room between them. The
tab you came from is marked. Agents, terminals and transcripts keep running: zooming changes the
view, never the work. One key in, one key back.

## Slices

1. **The overview** (built, #422): ⌘⇧\\ toggles it; tiles on an automatic grid; arrows or H J K L
   select, ↵ or a click goes in, ⌘⇧\\ or Esc goes back; the zoom animated both ways.
2. **The canvas** (this slice, below): the overview zooms and pans freely, and tiles can be placed.
   The owner after slice 1: "we don't zoom out very far - and it's not like a canvas where I can
   zoom in and out and pan and stuff".
3. **Things on the canvas**: dropped files (linked by path), pasted images, notes, text, shapes,
   arrows, ink (A5, A6); the code city as the ground (A4.1). The capture annotator's pen and
   shapes are the likely base for ink and shapes. Not built.

## Slice 2: the canvas

**World and camera.** Tiles live in a world where one tab is 1 × 1 (its panes area's shape,
whatever the window), with 0.12 between grid cells. The camera is a `scale` (1: one tab fills the
area) and a `pan` (the world point at the area's top left). Both are pure numbers, so the core
needs no window size: the shell maps world to points by the area's width and height. Per space
(workspace or task), kept in `layout.json` beside the space.

- **Range:** scale 1 (one tab fills the area: the nearest; nearer is the tab itself, ↵) out to
  0.08. At 0.08 a 4 × 4 workspace (16 tabs) is 0.35 of the area wide: small, with room around
  it for slice 3's things. Farther shows nothing more.
- **Bounds:** the pan keeps the area's middle inside the tiles' box grown by half a view: some
  tile is always in view. ⌘0 (fit) always recovers: it shows every tile, centred.
- **⌘⇧\\:** opens fitted (every tile, centred; never nearer than 0.5), the zoom out flies from
  the tab to there; again, or Esc, flies back into the tab you came from. ↵, a click or a double
  click on a tile flies into it.

**Gestures.** GPUI 0.2.2 delivers no pinch (as the code city found): pinch waits for a native
hook. Today, as the code city's camera does:

| Input | Does |
| --- | --- |
| Two-finger scroll (trackpad) | pans: the canvas follows the fingers |
| ⌘ + two-finger scroll, a mouse wheel | zooms about the pointer |
| Drag on the empty canvas, space-drag anywhere | pans |
| ⌘= / ⌘- | zoom in / out about the area's middle, by 1.25 |
| ⌘0 | fit every tile |
| Arrows, H J K L | the selection, as before; the camera follows it into view |

⌘= / ⌘- / ⌘0 are free in ottospace (no binding; a terminal program never gets ⌘ keys) and act
only while zoomed out; elsewhere they do nothing.

**Free placement.** Tiles start on the grid. A drag of a tile (past 4 points) places it where it
is dropped; its world position is kept on the tab in `layout.json`. Snap: to a 0.25-tile grid
(about 28 points at fit for four tabs), light enough to place by eye, firm enough that tiles line
up without effort. Tidy (the canvas's corner button, T zoomed out) puts every tile back on the
grid. A new tab takes the next grid cell that no placed tile covers. Only the user's drag places:
a client's is refused (`only the user places a tile`).

**Detail by size** (a tile's width on screen): over 360 points, terminals' text; 120 to 360,
each terminal row a bar of its colours (blocks); under 120, the tab's name alone. Cheap at any
distance: far out, nothing is shaped.

**Idle.** The camera moves only on input; a fly in or out plays 220 ms and asks for frames only
while it moves (`counts.zoom_frames`); Reduce Motion: a cut. Hosted windows stay parked while
zoomed out (slice 1).

## The model (core)

`Model::zoom: Option<Zoom>`, in the state as `state.zoom`:

- `from`: the tab you came from. `selected`: the tile the keys are on. Both tab ids.
- `live`: the app pane that had the keyboard when it opened (not in the state).

Actions, all of the `zoom_out` flag:

| Action | What it does |
| --- | --- |
| `ZoomOut` | Closed: opens on the main window's space, `from` = `selected` = the tab on show; an app you are in is parked (`Effect::Parked`), as ⌘/ does. Open: back to `from`, and the app that was live is live again (`Effect::Live`). |
| `ZoomMove { dir }` | The selection moves one cell on the grid; it stops at the edges. |
| `ZoomEnter { tab }` | Into `tab` (a click, a label) or the selected tile (↵): the overview closes and that tab is selected, as ⌘1 would. |

- **The grid is the core's**: `zoom::grid(n)` gives columns and rows (`cols = ⌈√n⌉`: 1, 2×2 for 4,
  3 + 3 + 1 for 7), `zoom::cells(n, area)` each tile's rect, with a gap between tiles and around
  them. Tiles keep the panes area's aspect. Tab order is the tab row's (⌘1…⌘9): a new tab takes the
  next cell and nothing else moves.
- **It never changes what exists.** No action of it adds, closes or moves a tab or a pane. The
  property test checks it: while open, `from` and `selected` are tabs of the space on show (never
  Home's), no app is live, and leaving returns to a tab that exists.
- **Any other move of the user's closes it** (a click on a rail, ⌘T, ⌘1, the palette): the
  overview just goes, and the move does what it does. Sneak mode's own steps do not close it.
- **A client's step runs underneath** (a split in a tab redraws its tile). If its tab goes, the
  selection goes to the tab on show; if no tab is left, or the space on show changes, it closes.
- **Home** has no tabs to show: ⌘⇧\\ there says so in a notice and opens nothing.

## Who may (origins)

It covers the user's work and takes the keyboard, so by the Principles it is the user's alone, as
the side overlays and the key reference are: a socket client's `ZoomOut`, `ZoomMove` or
`ZoomEnter` is refused ("only the user zooms out or in"), and so is a client's key ⌘⇧\\ (the key
is the client's). A timer or a notice never zooms. A client's action that would take the keyboard
is refused while it is open (as under sneak mode). A test run drives it as the user with the
existing test path: `key` and `mouse` with `"as":"user"`. No new request.

## How a tile is pictured

Chosen: a **miniature drawn live from what the shell already holds**, never a new capture.

- The tab's tree laid out in the tile (`Node::rects`), each pane a small header with its title and
  a body:
  - a **terminal**: its screen painted at the tile's scale, live (`terminal::paint`, the font
    scaled). No resize: the program sees nothing.
  - an **app pane**: its window's **last stream frame**, scaled. Every hosted window that is not
    live is parked and streamed already (ScreenCaptureKit, for its pane's picture); the overview
    parks the live one, so every app pane has a frame. Nothing new is captured.
  - anything else (a file viewer, an empty pane, a slot, Decisions): its header name over a plain
    body.
- The current tab (`from`) has the accent ring and "you were here"; the selected tile a ring of
  the focus colour. A tab out (`multi_monitor`) is a tile marked `out`: ↵ on it goes to its window,
  as a click on its tab does.
- Rejected: a picture of ottospace's own window per tab (SCK of ourselves): it is a capture the
  user did not ask for, and a background tab is never drawn to take one.
- **Cost**: while open, each frame paints every tab's terminals once more (rows × tabs shaped
  lines, small) and samples one surface per app pane. Nothing at rest when closed. The tab on show
  is not drawn under the overview while it is open and still (only during the zoom), so its
  controls get no sneak labels.

## Hosted windows

A live app window sits above ottospace and cannot be scaled (macOS cannot). So:

- Opening parks the live window (as ⌘/ and the AX overlay do): its picture takes its place in the
  tile. Parked windows stay parked; none is moved anywhere new, none floats over the overview.
- ⌘⇧\\ or Esc back to the tab you came from makes the same app live again: the window comes back
  exactly where it was. ↵ into another tab, as ⌘1: no app is live there until you click one.
- The zoom back plays over the panes; the live window comes back on the first frame (it is above
  ottospace), not the last.

## The zoom

- The core moves first: the overview opens or closes at once; the shell only eases toward it, so
  the next key acts on the new view (↵ mid-zoom goes in at once; ⌘⇧\\ again turns it back from
  where it is).
- Out: the current tab's tile shrinks from the whole panes area to its cell; the other tiles fade
  in to theirs. In: the chosen tile grows to the panes area, the others fade. 220 ms, eased out.
- While closing, the overview is drawn only (no click, no hover): the click goes to the pane under
  it.
- Reduce Motion: a cut. `counts.zoom_frames` counts the frames the zoom asked for: none at rest,
  none under `OTTOSPACE_REDUCE_MOTION`.

## Keys

| Key | Where | What |
| --- | --- | --- |
| ⌘⇧\\ | anywhere | zoom out; zoomed out, back to the tab you came from |
| ← → ↑ ↓, H J K L | zoomed out | move the selection |
| ↵, a click | zoomed out | go into that tab |
| Esc | zoomed out | back to the tab you came from |
| ⌘; | zoomed out | sneak labels each tile; its letters go in |

Also View ▸ Zoom Out or In and the palette.

**⌘⇧\\ is free**: not in `docs/reference.md`'s keys, the key reference (`key_use`) or the keymap;
macOS binds no global shortcut to it (Mission Control is ⌃↑, App Exposé ⌃↓); Safari uses it inside
Safari only (Show All Tabs, the same idea); a terminal program never gets a ⌘ key. GPUI reads it as
`cmd-|` (the shifted character), so that is the binding.

## Open questions, with a recommendation

1. **Pinch to zoom** (A3): recommend later, with slice 2: over an app pane the pinch is the app's.
2. **The rails' shadow at rest** (A3, frame 1): recommend only while zoomed (slice 1b); at rest the
   side zones already cast today's shadow.
3. **City ground** (A4.1): recommend slice 1b: the city is a picture per folder and size, so one
   behind the grid is a new render each window size. Cheap once cached.
4. **Close and rename on a tile**: recommend not in slice 1 (Executive work: close a tab in it).
5. **Typing zoomed out**: recommend never (A5): you go in first. Keeps "no surprise keystrokes into
   a terminal you can barely read".
6. **A tab added zoomed out** (a client's): recommend it takes the next cell, unselected.
7. **Home zoomed out**: recommend nothing in slice 1 (a notice); every workspace as one tile later.
