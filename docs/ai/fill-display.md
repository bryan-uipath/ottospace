# Fill the whole display: over the menu bar and the Dock

⌃⌘F (the Mac's Enter Full Screen key), View ▸ Fill the Whole Display, the window's green button,
Window ▸ Zoom, a title bar double-click (when the system's setting is Zoom), and the expand
button at the top row's right. There is no zoom to the visible frame any more: its key (⌘⌃F)
and the green button now fill the display.
Asked for as "pseudo full screen [that takes] up the full monitor space (over menu bar and dock)".

## What it does

- The window takes its display's whole frame (`NSScreen.frame`), and the app's presentation
  options become `AutoHideMenuBar | AutoHideDock`. Again: both back.
- Native full screen is not used: it moves the window to its own Space, where other apps'
  windows cannot follow (`window::zoom_instead_of_fullscreen`).
- The options count only while ottospace is the app in front. In an app pane the app is in
  front: its menu bar and the Dock come back, over ottospace's top row and bottom edge.
- The window cannot go above them: hosted windows are other apps' normal-level windows and
  would be behind it.

## The green button

Native full screen is off (`FullScreenNone`), so the green button is a zoom button (+). Every
zoom goes through `zoom:`: the button's `performZoom:`, a menu item, GPUI's `zoom_window` and its
`titlebar_double_click` (which reads `AppleActionOnDoubleClick`). GPUI 0.2.2's window class
(`GPUIWindow`, an NSWindow subclass) has no `zoom:` of its own, so `window::zoom_asks` adds one
with `class_addMethod` (no fork): it sets a flag and does not call NSWindow's, so the window never
zooms. The shell's 60 Hz tick takes the flag (`take_zoom`) and dispatches `FillDisplay` as
`Origin::User`, as it does the return key.

First tried: the delegate's `windowShouldZoom:toFrame:` (GPUI's window is its own delegate). It
worked locally, but on the CI runner `zoom:` resized the test window without asking it.

Safe because: `class_addMethod` adds only, and refuses if the class defines `zoom:` itself (a
later GPUI that does keeps its own, and the green button zooms as before); the method only
stores an atomic, so it never reenters GPUI while AppKit is in `zoom:`. It is ottospace's own
window class in its own process; the canary and other apps are untouched. Who can zoom: a click,
a menu item or a double-click (injected pointer events never double-click the title bar), or
another process pressing the button through Accessibility, which could as well post ⌃⌘F.
ottospace itself zooms only from Window ▸ Zoom, and in a test run from `zoom_press`.

## Measured

2026-11-16, the `e2e-apps` runner (macOS 26, one 1024×768 display, no camera housing, Dock at
the bottom, not hidden), with the canary as the hosted app. A temporary test read AppKit in both
processes (`presentationOptions`, `currentSystemPresentationOptions`, `NSScreen.frame` and
`visibleFrame`, the window's frame), listed the window server's menu bar and Dock windows
(`CGWindowListCopyWindowInfo`, on screen only) and took pictures of the runner's screen.
Frames are `[x, y, w, h]` from the display's top left.

| Step | ottospace's window | System options | Menu bar, Dock windows | Pane 1 | Canary's window |
|---|---|---|---|---|---|
| Before | `[-188,31,1024,677]` | 0 | on screen (bar 30 tall, Dock from 708) | `[319,75,702,575]` | parked, 702×607 |
| Mode on, ottospace in front | `[0,0,1024,768]` | 5 | not on screen | `[319,75,702,666]` | parked, 702×667 |
| In the app pane (canary in front) | `[0,0,1024,768]` | 0 | on screen | `[319,75,702,666]` | `[319,43,702,667]` |
| Back in ottospace | `[0,0,1024,768]` | 5 | not on screen | `[319,75,702,666]` | parked, 702×667 |
| Zoom (then ⌘⌃F) while on | `[0,31,1004,679]` | 0 | on screen | `[319,75,682,577]` | parked, 682×609 |
| Mode off | `[-188,31,1024,677]` | 0 | on screen | `[319,75,702,575]` | parked, 702×607 |
| Mode on, then `kill -9` | gone | 0 after 165 ms | on screen | | |

The five questions:

1. **Does the window cover the display, under the menu bar?** Yes: `[0,0,1024,768]`, not
   constrained by AppKit, and the bar's and the Dock's windows leave the screen. No camera
   housing to test: the runner reports `safeAreaInsets` 0 and empty `auxiliaryTop…Area`s. On a
   display that has one the frame starts below its strip (`fill::frame`): the menu bar is as
   tall as that strip, so it returns into it and covers nothing. **Untested on hardware.**
2. **When an app pane takes the keyboard?** The system's options go to 0 and both bars are back
   on screen by the first read (0.5 s after the request, which includes the request). They
   **overlay** ottospace's window: macOS does not move or resize it, and ottospace's layout does
   not change (pane 1 the same to the point). ottospace's own `visibleFrame` is no guide: it
   stays the whole display while its options are set, in front or not. Another process keeps
   seeing `[0,31,1024,679]` throughout. macOS does not move the hosted window, but does limit it:
   see 5.
3. **Back in ottospace?** Options 5 and both bars off screen by the first read (0.2 s). Window
   and layout the same to the point: nothing jumps. How the slide looks is not something a
   headless runner shows.
4. **Two displays?** Not measurable: the runner has one. By hand, below.
5. **Does hosting break?** One thing did, and is handled. An app's window cannot be resized
   into the Dock's strip, nor be taller than the visible frame: asked for 702×698 over a pane
   that reached under the Dock, the canary's window came out 666 tall the first time (the header
   said `fixed size 702×634`) and 679 the next (14 pt under the Dock). So while the mode is on a
   live app pane asks its window for no more than what lies above the Dock (`fill::app_rect`),
   and the pane's last 33 pt show its background, under the Dock. Parked, it asks for as much as
   macOS allows (see "Bug: an empty strip where the Dock was" below). `top_limit`
   and the title-bar crop are unchanged (the raised window's top, 43, is below the 30 pt bar),
   parking is unchanged, and the recovery path does not involve the mode.

A background app cannot answer question 1: a `cargo e2e` instance is never the app in front, so
its options never apply; any app's window may be framed over the menu bar's place, but at level
0 (or floating, 3) it is under the bar (level 24). The shell leaves a test run's window alone
in any case: it floats over the user's screen.

GPUI drops the resize AppKit reports when the frame is set inside one of its updates: the
first try left the layout at the old size inside a larger window. The frame is set from a task
of its own, as GPUI's `zoom_window` does.

Turning the mode off, the window was once not put back: read after the options were reset, its
frame was no longer the display's, so it counted as moved by the user and stayed. The cause was
not measured; the likeliest is AppKit keeping the window under the menu bar that comes back.
Where the window was is now read before the bars return, and the test for it passes.

## Bug: an empty strip where the Dock was

Reported on the first build: "initially 'full' down to the dock but then reserved space for the
dock". ottospace's window was not the cause: CI read `[0,0,1024,768]`, the display's frame,
before and after the fix, a second after turning it on, in the app pane, back in ottospace and
after a split beside the pane. The parked app window was. It was sized as the live one
(`fill::within`), so while ottospace was in front and the Dock away its picture stopped at the
Dock's top and the pane's foot was empty: the layout grew to the display, the picture only to
the Dock.

Fix: `fill::app_rect`. Only a live (or peeked) window stops at the Dock's top, where the Dock is
back over the pane anyway. A parked one is as tall as macOS lets it be. That is not the whole
pane: asked for 702×700 parked (off screen), the canary's window came out 679 tall, the height
of the visible frame as an app sees it with the Dock showing. So a parked window asks for no
more than that (ottospace's own reading, 677, taken before the bars go), and no misfit is named.

| CI, ottospace in front, the Dock away | Pane 1 | Canary's window, parked | Empty at the pane's foot |
|---|---|---|---|
| Before | `[319,75,702,666]` | 702×667: 633 under a 34 pt crop | 33 pt (the Dock's height) |
| Asked for the whole pane | `[319,75,702,666]` | 702×679: 645 | 21 pt |
| After | `[319,75,702,666]` | asks 702×677: 643 (the test holds it to 641 or more, no misfit) | 23 pt |

What is left empty is the pane's height plus its title bar less the visible frame's height: on
the runner 33 pt became 23. It is never more than the Dock's height, and it is none for a pane no
taller than an app's window can be (a split below, or the Dock set to hide).

The two sizes now differ, so `Hosted::place` makes the next park size the window again (the
review found the parked window kept the live height after one enter and leave). Costs: the app
re-lays out its last few points on each enter and leave of a pane that reaches the Dock's
strip, and a click in the part of the picture the live window will not cover enters the pane
but is not replayed (`hosting::click` judges it against the live window).

## What was chosen, and why

- **The bars overlay; nothing is inset.** While an app is in front its menu bar covers
  ottospace's top row (30 of its 40 pt) and the Dock its bottom edge. The other two ways both
  cost more. Insetting the layout while an app is in front moves every pane under the pointer
  at the click that enters the pane (a pane below a split moves up by its share of the Dock's
  height), and the click is replayed into a window that has moved. Keeping both strips free
  always is a zoom to the visible frame with the bars hidden: no gain.
- **A live app's window stops at the Dock's top.** macOS enforces it; asking for it names no
  misfit. Parked, the window is as tall as macOS allows: the Dock is away while ottospace is in
  front, but no window is taller than the visible frame, so a tall pane keeps a smaller strip.
- **What it gains.** Terminals and ottospace's own chrome: the whole display while ottospace
  is in front (91 pt more on the runner). An app pane: the menu bar's height, and no more at
  the bottom unless the Dock is set to hide in System Settings.
- **What it costs.** In an app pane the top row is under the app's menu bar, and other panes'
  last lines are under the Dock, until ottospace is in front again (the return key, or a click
  on it).
- **Only the user turns it on** (`fill::USERS_ALONE`): a client's key, click, sneak label or
  palette choice is refused. Anyone may end it.
- **Kept across restarts.** On at quit, `layout.json` says `"fill": true`, and the next start
  fills again (`RestoreLayout`, once the window exists). The window's frame from before is not
  kept: off after a restart, the window stays where macOS put it.

## How it ends

| How | Bars | Window |
|---|---|---|
| ⌃⌘F, the green button or the top row's button again | options to default | back where it was (`fill::back`), or centred if that place is gone |
| A drag, a display unplugged | the shell sees the frame is no longer the display's (`fill_holds`: on a bounds change, and every 500 ms for a display that changed under it) and ends the mode | stays where it now is |
| Quit, a crash, `kill -9` | the options are the process's: gone with it (measured: 165 ms) | gone |
| Sleep, lock, hide, another app in front | the options apply only while ottospace is in front | unchanged |

The bars are only ever hidden with auto-hide: the pointer at the screen's edge shows them, mode
or no mode. The guardian has nothing to do: it gives other apps' windows back, and the mode
holds none.

## To check by hand

A headless runner has one virtual display, no camera housing, and nobody watching.

1. **How the change looks.** Host an app, ⌃⌘F, click into the app pane, then press the return
   key. Watch the menu bar and the Dock slide in and out: is it tolerable, and does anything in
   ottospace move?
2. **The top row under the menu bar.** In an app pane, is losing the top row until you are back
   in ottospace acceptable, or should the row move down while an app is in front?
3. **Two displays.** ⌃⌘F on one. Does the other keep its menu bar? Click a window on the other
   display: do the bars return on the first? Drag ottospace to the other display: the mode
   should end.
4. **Real apps.** A browser and an Electron app in a pane that reaches the bottom: does the
   window stop cleanly at the Dock's top, with no `fixed size` in the pane's header?
5. **Dock at the side** (only if you use it so): does an app's window at that edge sit under
   the Dock? `fill::app_rect` only keeps windows above a Dock at the bottom.
6. **A camera housing** (a MacBook's own display): the window should start below the strip, and
   the menu bar return into it.
7. **A Dock resized, moved or set to hide while the mode is on** is not seen: turn the mode off
   and on.
