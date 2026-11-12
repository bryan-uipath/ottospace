# Zoom: what a meeting exposes through accessibility

Measured facts for `zoom-meeting.md`. Recorded during one real meeting on 2026-11-12 (Zoom
Workplace 7.2.5 (90525), macOS 26.7.2, two displays): every 30 seconds, the window list and each window's
accessibility elements were read through System Events. Read-only: nothing was moved or pressed,
and no picture was taken. The raw log holds people's names and chat text, so it is not in the
repository; this note has structure only.

## Answer first: can we read all participants?

| What | Readable? | How |
|---|---|---|
| How many participants | Yes, always | The toolbar button's description: `Open participants panel, closed, 10 participants` |
| Names of the people on screen | Yes | Each video tile is a `tab group` whose description is `<name>, Computer audio muted\|unmuted, Video on\|off` |
| Names of everyone | Not without acting | The gallery showed 4 tiles a page (`Next page. Currently on page 1, there are 4 pages in total`). The rest are on other pages |
| Names from the participants panel | Not measured | The panel stayed closed for the whole recording |
| Who is muted, whose video is off | Yes, for visible tiles | Same description string |

So a read-only pass gives the count and the visible page. The whole list needs one of:

1. The participants panel open (by the user, or by pressing `Open participants panel`), then
   reading its rows. Expected to work, not measured.
2. Paging the gallery (pressing `Next page`) and reading each page's tiles. It changes what the
   user sees.
3. A gallery large enough to show everyone on one page.

Options 1 and 2 press a control in another app. That is the "agents press controls" trust
decision in `accessibility.md`, not a read.

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
  The share toolbar, the status bar and the annotation layer must be left alone: they are
  Zoom's own floating chrome, and the annotation layer covers the display.
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
- **Spinner feed:** the count is free. Names need the panel open or the gallery paged. The
  simplest honest flow: the user opens the participants panel once, ottospace reads the rows
  and offers "Spin". No press in Zoom, no trust decision.
- **Cleaning names** (strip `(Host, me)` and the like) is still needed for the panel; tile
  descriptions carry the plain name followed by the audio and video state.

## Not measured

1. The participants panel: its element tree, whether rows exist only while it is open, how
   names and roles are written, and what a popped-out panel's window is called.
2. Whether a parked Zoom window keeps rendering video.
3. Minimum sizes of the two `Zoom Meeting` windows.
4. Waiting room, breakout rooms, a webinar, a one-to-one call (the tiles may differ).
5. Whether the second `Zoom Meeting` window exists with one display.
6. How long a full read takes on a large meeting. Here each window's read took well under a
   second, with at most a few hundred elements.
