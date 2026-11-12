# Zoom: what a meeting exposes through accessibility

Measured facts for `zoom-meeting.md`. Recorded during one real meeting on 2026-11-12 (Zoom
Workplace 7.2.5 (90525), macOS 26.7.2, two displays): every 30 seconds, the window list and each window's
accessibility elements were read through System Events. Read-only: nothing was moved or pressed,
and no picture was taken. The raw log holds people's names and chat text, so it is not in the
repository; this note has structure only.

## Answer first: can we read all participants?

**Yes, while the participants panel is open.** With it closed, only the count and the people on
the visible gallery page.

| What | Readable? | How |
|---|---|---|
| How many participants | Yes, always | The toolbar button's description: `Open participants panel, closed, 10 participants`. With the panel open, also its heading: `Participants (11)` |
| Everyone's name | Yes, with the panel open | The panel is an `outline` with one `row` a participant. All 11 rows gave a name, in about 1 second |
| Names with the panel closed | Only the visible gallery page | Each video tile is a `tab group` whose description is `<name>, Computer audio muted\|unmuted, Video on\|off`. The gallery showed 4 tiles a page, of 4 pages |
| Who is muted, whose video is off | Yes | Panel: the row's images, described `Computer audio muted` / `unmuted`, `Video on` / `off`. Tiles: the same words in the description |

Reading is all it takes once the panel is open: nothing in Zoom is pressed. Opening the panel
for the user would be a press in another app, which is the "agents press controls" trust
decision in `accessibility.md`. So the first version asks the user to open it.

### The participants panel

```
window "Zoom Meeting" (the main one)
  static text "Participants (11)"
  scroll area
    outline
      row            AXRow, subrole AXTableRow, about 339 × 48 points
        cell         AXCell, no description, no value
          static text    the name, e.g. "Ana Example" or "Ana Example (me)"
          menu button
          image ...      "Computer audio muted", "Video off", ...
```

- **The cell's children are made on request.** A whole-window read (`entire contents`) returned
  the rows and cells but nothing inside the cells, except for one row. Asking each cell for its
  children returned them. A reader must walk row by row; the bounded tree reader in
  `ottospace-mac` asks for children at each node, so it should see them (not run against Zoom).
- **The name is the static text's value.** Row and cell have no title or description.
- **Suffixes seen:** `(me)` on the user's own row. `(Host)`, `(Co-host)`, `(Guest)` were not seen
  in this meeting; cleaning must still expect them.
- **Not every row is a person.** One row was a meeting-room system, with a longer name made of
  several words and numbers. The spinner should let the user drop rows.
- The panel sits inside the main window (beside or above chat). A popped-out panel was not seen.

## Windows

Positions are left out; sizes are in points.

| State | Window title | Subrole | Size | What it is |
|---|---|---|---|---|
| In a meeting | `Zoom Meeting` | `AXStandardWindow` | 696 × 940, and 1065 × 940 with chat open | Main window: tiles, toolbar, and the chat panel in a splitter when chat is open |
| In a meeting | `Zoom Meeting` | `AXStandardWindow` | 880 × 496 | Second window with the same title (video on the other display). Few controls: `View`, security, window buttons |
| Reaction picker open | `Reactions` | `AXUnknown` | 44 × 44 | Transient |
| Sharing | `zoom share statusbar window` | `AXSystemDialog` | 334 × 22 | "You are screen sharing", 3 buttons |
| Sharing | `zoom share toolbar window` | `AXSystemDialog` | 961 × 58 | The floating meeting controls, 16 buttons |
| Sharing | `zoom annotation entrypoint` | `AXSystemDialog` | 56 × 56 | One button |
| Sharing | `Annotation - Zoom` | `AXSystemDialog` | 1920 × 1080 | A layer over the whole shared display. No elements |
| Sharing, chat open | `Meeting chat` | `AXSystemDialog` | not recorded | The chat panel, popped out of the main window |
| Sharing | `Zoom Meeting` | `AXStandardWindow` | 880 × 496 | The only standard window left |

- **Two windows share one title.** A rule cannot tell them apart by title; it needs size, the
  display, or what the window contains (the main one has the toolbar buttons).
- **Sharing changes the set.** The main window is gone from the list while sharing,
  and four system dialogs appear. `zoom-meeting.md` guessed this; it is confirmed.
- **Only standard windows can be hosted** today, so while sharing there is one hostable window.

### Windows to leave alone

Bryan, 2026-11-12: the share window and the floating toolbar are things "we would want to
ignore / not manage". The rule, from what was measured:

| Window | Why |
|---|---|
| `zoom share statusbar window`, `zoom share toolbar window`, `zoom annotation entrypoint` | Zoom's floating controls while sharing. Moving them breaks the share controls |
| `Annotation - Zoom` | A layer over the whole shared display |
| `Reactions` and other `AXUnknown` windows | Short-lived pop-ups |
| `Meeting chat` (while sharing) | A system dialog, not a standard window; it goes back into the main window when sharing ends |

All of these have a subrole other than `AXStandardWindow`, so "host standard windows only"
already skips them. The rule set should still name them, so a later change to what is hosted
does not pick them up, and so a new-window watcher does not open panes for them.
- Idle Zoom (no meeting) has one window, `Zoom Workplace`, `AXStandardWindow` (2026-11-11).

## Main window

- **Tiles:** `tab group 1..n`, one a visible participant. Each has an `image` (description
  `profile image` when video is off) and one `button` described with the participant's name.
  Titles are empty; the description carries everything.
- **Toolbar**, as button descriptions, in order: `Mute my audio` / `Unmute my audio`,
  `Audio options`, `Start video`, `Video options`, `Leave`, `Open participants panel` (with
  `closed` and the count), `Participants options`, `Open chat panel` / `Close chat panel`,
  `Chat options`, `React`, `React options` (or `Share`, `Share options`),
  `More meeting controls`, the meeting's title, notes, Zoom AI, `View options`, the current view
  (`Speaker` / `Gallery`), security, transcription (`Transcription is on`), rooms, and the three
  window buttons. Element specifiers give these buttons no name: read `description`.
- **State you can read from descriptions alone:** muted or not (the button says what a press
  would do), video on or off, chat open or closed, participants panel open or closed, the
  participant count, the gallery page and page count, the view, whether transcription is on,
  and the meeting's title.
- **Chat panel:** `splitter group > scroll area > table > row`. A row holds the sender and time
  as text, and the message in `text area` elements. So chat is readable while the panel is open.
  This is other people's words: treat it like any pane's text (`skill/guide.md`), and read it
  only for a task the user asked for.

## Share toolbar (while sharing)

16 buttons, by description: `Mute my audio`, `Audio options`, `Start video`, `Video options`,
`Participants`, `Participants options`, `Chat`, `Chat options`, `Share`, `Share options`,
`Pause`, `Layout`, `Annotate`, `Remote control`, `Open apps panel`, `More meeting controls`.
The participant count is not in this toolbar's descriptions.

## What this changes in `zoom-meeting.md`

- **Rules by title alone are not enough.** `Zoom Meeting` matches two windows, and there is no
  `Participants*` window unless the panel is popped out (not seen). Chat is a panel inside the
  main window, and a separate `Meeting chat` window only while sharing.
- **A "sharing" state is needed**, as guessed: detect it by the `zoom share toolbar window`
  appearing, and stand the layout down (or keep only the one standard window).
- **Spinner feed:** measured to work with the panel open. Flow: the user opens the participants
  panel, ottospace reads the rows (about a second for 11) and offers "Spin". No press in Zoom,
  no trust decision. With the panel closed ottospace can still show the count and say "open
  the participants panel to spin".
- **Cleaning names:** strip `(me)` and expect `(Host)`, `(Co-host)`, `(Guest)`; let the user drop
  rows that are rooms, not people.

## Not measured

1. A popped-out participants panel (its window's title and subrole), and how host, co-host and
   guest are written in the rows.
2. Whether a parked Zoom window keeps rendering video.
3. Minimum sizes of the two `Zoom Meeting` windows.
4. Waiting room, breakout rooms, a webinar, a one-to-one call (the tiles may differ).
5. Whether the second `Zoom Meeting` window exists with one display.
6. How long a full read takes on a large meeting. Here each window's read took well under a
   second, with at most a few hundred elements.
