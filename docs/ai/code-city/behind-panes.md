# The city behind panes (design)

The owner, 2026-11-23: "we can have it transparent behind terminal windows and explorer windows
(ie, set opacity for both) so activity is somewhat visible behind the terminal".

## What changes

Today a pane's background is opaque, and the city shows only where a tab or pane is empty. Here
a tab's city (the task's or checkout's, from the same camera and the same persistent view) is
drawn once behind all its panes, and a pane may let it show through.

- **Which panes:** terminals and file viewers. App panes are real windows hosted from other
  apps: ottospace draws nothing under them, so they stay opaque. The empty state is the city in
  front, as today.
- **Settings:** `terminal_opacity` and `viewer_opacity`, each 0.6 to 1.0, default 1.0 (opaque,
  today's look). Below 0.6 text loses contrast on a lit city, so the floor is enforced.
- **Readability:** text draws on the pane's own background at its opacity, plus a dim layer
  over the city under the pane (the city at about 20% brightness). A blur would cost a pass per
  frame over the whole pane; the dim costs one quad, so dim first.

## Cost: the city renders nothing at rest

- One picture per tab, made as today on the city's thread. It is made again only when
  something in it changes: an agent moves or edits (its icon, the lit building), a rise, or the
  tab's size.
- Terminal scrolling repaints the terminal over the same picture and never asks for a new one.
  The frame counters (`counts`) prove it: a test streams a terminal over a city and asserts no
  `city_picture` event.
- GPUI: a pane's background with alpha over an image painted below it is ordinary compositing,
  one image quad per tab per frame. That is cheap: the picture is already on the GPU after its
  upload. The upload happens only when the picture changes.

## How it fits the rest

- **Zoom-out (#422):** in the zoomed-out grid each tab is a tile, and its city behind it is
  drawn at the tile's size. Probably later: first slice only for the tab on show.
- **Plinth names:** hidden behind panes, since text under text is noise. Agent icons stay; they
  are the point.
- **Reduce Motion:** no rise and no glide behind panes; changes swap at once.
- **Camera:** the city behind a tab uses that tab's view (#453). Dragging it needs an empty
  place; behind panes it does not take the mouse.

## First slice (recommended)

One city per tab, drawn once behind all its panes; `terminal_opacity` only; activity behind is
agent icons and a short pulse on a building when an agent edits it. Viewers next.

## Questions for the owner (with my picks)

1. Behind app panes too? Pick: no. They are other apps' windows, and we would draw under the
   window's own background.
2. Names behind panes? Pick: none. Icons and edit pulses only.
3. Default opacity? Pick: 1.0 (off) until turned on; suggested value 0.85.
4. Dim or blur under text? Pick: dim (cheap); blur only if dim reads poorly.
5. Only the tab on show, or every tab's tile in the zoomed-out grid? Pick: on show first.
