# Fill the whole display: over the menu bar and the Dock

⌘⌃⇧F, View ▸ Fill the Whole Display. Beside ⌘⌃F (zoom to the visible frame), not in its place.
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
| ⌘⌃F while on | `[0,31,1004,679]` | 0 | on screen | `[319,75,682,577]` | parked, 682×609 |
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
   said `fixed size 702×634`) and 679 the next (14 pt under the Dock). So while the mode is on an
   app pane asks its window for no more than what lies above the Dock (`fill::within`): 702×667
   parked and live, no misfit named, and the pane's last 31 pt show its background. `top_limit`
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

## What was chosen, and why

- **The bars overlay; nothing is inset.** While an app is in front its menu bar covers
  ottospace's top row (30 of its 40 pt) and the Dock its bottom edge. The other two ways both
  cost more. Insetting the layout while an app is in front moves every pane under the pointer
  at the click that enters the pane (a pane below a split moves up by its share of the Dock's
  height), and the click is replayed into a window that has moved. Keeping both strips free
  always is today's ⌘⌃F with the bars hidden: no gain.
- **An app's window stops at the Dock's top.** macOS enforces it; asking for it makes it the
  same size parked and live, with nothing named a misfit.
- **What it gains.** Terminals and ottospace's own chrome: the whole display while ottospace
  is in front (91 pt more on the runner). An app pane: the menu bar's height, and no more at
  the bottom unless the Dock is set to hide in System Settings.
- **What it costs.** In an app pane the top row is under the app's menu bar, and other panes'
  last lines are under the Dock, until ottospace is in front again (the return key, or a click
  on it).
- **Only the user turns it on** (`fill::USERS_ALONE`): a client's key or palette choice is
  refused. Anyone may end it.
- **Not kept across restarts.** The window's frame is not either.

## How it ends

| How | Bars | Window |
|---|---|---|
| ⌘⌃⇧F again | options to default | back where it was (`fill::back`), or centred if that place is gone |
| ⌘⌃F, the green button, a drag, a display unplugged | the shell sees the frame is no longer the display's (`fill_holds`: on a bounds change, and every 500 ms for a display that changed under it) and ends the mode | stays where it now is |
| Quit, a crash, `kill -9` | the options are the process's: gone with it (measured: 165 ms) | gone |
| Sleep, lock, hide, another app in front | the options apply only while ottospace is in front | unchanged |

The bars are only ever hidden with auto-hide: the pointer at the screen's edge shows them, mode
or no mode. The guardian has nothing to do: it gives other apps' windows back, and the mode
holds none.

## To check by hand

A headless runner has one virtual display, no camera housing, and nobody watching.

1. **How the change looks.** Host an app, ⌘⌃⇧F, click into the app pane, then press the return
   key. Watch the menu bar and the Dock slide in and out: is it tolerable, and does anything in
   ottospace move?
2. **The top row under the menu bar.** In an app pane, is losing the top row until you are back
   in ottospace acceptable, or should the row move down while an app is in front?
3. **Two displays.** ⌘⌃⇧F on one. Does the other keep its menu bar? Click a window on the other
   display: do the bars return on the first? Drag ottospace to the other display: the mode
   should end.
4. **Real apps.** A browser and an Electron app in a pane that reaches the bottom: does the
   window stop cleanly at the Dock's top, with no `fixed size` in the pane's header?
5. **Dock at the side** (only if you use it so): does an app's window at that edge sit under
   the Dock? `fill::within` only keeps windows above a Dock at the bottom.
6. **A camera housing** (a MacBook's own display): the window should start below the strip, and
   the menu bar return into it.
7. **A Dock resized, moved or set to hide while the mode is on** is not seen: turn the mode off
   and on.
