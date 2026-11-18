# Hosting other apps' windows: approach, measurements, risks

Moved out of the README on 2026-11-11. Technical background for `ottospace-mac` and the shell.

## Approach

macOS cannot reparent another process's window, so ottospace fakes embedding:

- **Live pane:** the real app window, placed over its pane through the Accessibility (AX) API
  (the AeroSpace model). Native input, no capture.
- **Parked pane:** a ScreenCaptureKit (SCK) stream of the window, with the real window parked on a
  1px off-screen sliver.
- **Fallback, not built:** park on a private `CGVirtualDisplay` if sliver-parked windows stop
  rendering.

Input forwarding to parked windows is a dead end: Chrome ignores `CGEventPostToPid` clicks even
on screen (tried window-under-pointer fields, private window-local location, HID-state source).
So interaction means: place the real window over the pane, then replay the click at HID level.

## One title bar per pane

An app pane shows one bar. Parked: the stream starts below the app's title bar
(`SCStreamConfiguration::with_source_rect`), so only ottospace's header shows; the window is
parked that much taller, so its content is pane-sized. Live: the window is raised by the bar's
height, so its own bar covers the pane header and its content lines up with the pane's. Apps
without a plain title bar keep the old behaviour: no crop, no raise.

Rule (`ottospace_core::titlebar::height`): crop only when AX reports a close button and an
`AXTitleUIElement`, no `AXToolbar` child, a title no taller than the button + 4pt, and the title
centred in the strip the button implies (bar = 2 × button offset + button height, 16–40pt).
Any failed read means no crop.

Measured 2026-11-11, macOS 26, attributes read with System Events (frames relative to window top):

| Window | Close button | Title element | Toolbar child | Result |
|---|---|---|---|---|
| ottospace-canary (GPUI) | y+8, 16pt | y+8, 16pt | no | crop 32 |
| Terminal | y+8, 16pt | y+8, 16pt | no | crop 32 |
| Finder (unified toolbar) | y+18, 16pt | y+0, 52pt | yes, 52pt | none |
| Activity Monitor (toolbar) | y+18, 16pt | y+0, 52pt | yes, 52pt | none |
| Chrome (tabs in title bar) | y+12, 16pt | none | no | none |
| VS Code (Electron) | y+9, 16pt | none | no | none |
| Spotify (CEF) | y+24, 16pt | none | no | none |

Safari and Slack were not running. An app pane with a bar gets a header as tall as the bar
(`titlebar::header`), inside the same 2pt gap as the content: the raised live window's bar covers
it exactly, and the pane's border stays visible on all four sides. The header does not shrink in
a short pane. A pane whose raised window would reach above ottospace's top bar gets no crop
(`titlebar::crop`); the layout keeps that from happening, and the top bar is below the menu bar,
so a raised window cannot go under it either. Not hidden: while live, the app's real title bar shows,
including the purple pill macOS draws in it while the window is captured.

## A sharp picture

A parked pane's picture is drawn 1:1: the stream captures the pane's points × the scale of
ottospace's display (`hosted::capture_pixels`), and GPUI draws it into the same pixels (its
sampler is linear; at 1:1 on whole pixels that changes nothing). Three things made it blurry:

- The scale was read at each re-park but the stream was reconfigured only for a new size or
  crop: ottospace moved from a 1× display to a 2× one kept its 1× picture, drawn up 2×.
  The check (every ~500ms) now also compares the scale.
- Windows were parked at the first free corner, whatever its display: with a 2× laptop and a
  1× external, often the external's. A window is drawn at its display's scale, so the app drew
  1× and no capture setting could add pixels. Parking now takes a free corner of a display with
  ottospace's scale (`hosted::parking_corner`): not more, or each park and enter would move the
  window between scales and the app would draw itself again. A window re-parks when the spot
  moves (displays changed, ottospace moved). Recovery counts every free corner as a parking spot
  (`hosted::parking_corners`). With no free corner on a 2× display (an external to the right
  that reaches lower than the laptop), the picture stays 1×.
- Memory: a picture is at most a 5K display's pixels (`hosted::MOST_PIXELS`, 14.7 M, 22 MB a
  4:2:0 frame); a larger pane is captured at a lower scale.

Measured 2026-11-17: a mostly off-screen window on a 1× display, captured at 1:1, equals
`screencapture -l` of it exactly in BGRA, and in 420f 92.7% of pixels within 2 levels (chroma at
half resolution). `e2e_apps.rs` (`a_parked_panes_picture_is_its_window_pixel_for_pixel`) compares
the canary's one-pixel stripes as ottospace draws them with the live window. CI's runner has one
1024×768 display at 1× and no HiDPI modes, so the 2× cases are checked by hand.

## Probe results

Measured with `probe`, a CLI since removed (`d483a37:crates/probe` in git history).
`cargo run -p probe -- "Google Chrome" 5` printed frames/sec by SCK status for on-screen → sliver →
restored. Play a video or animation in the target app first. Live means "complete/s" stays near
the on-screen rate while parked.

- 2026-11-10 Chrome + YouTube (with audio), 5s/phase: ~53 complete fps on-screen and parked. Live.
- 2026-11-10 Slack (Electron) + animated GIF, unfocused, 10s/phase: ~51 on-screen, ~50 parked. Live.
- 2026-11-10 Chrome + YouTube muted (site + player), 30s/phase: 39 on-screen, 51 parked, 56 restored. Live.

## Pop-ups: a new window that does not split

A new window of a hosted app (`adopt_new_windows`, every tick) used to split its app's pane if it
was standard: a Zoom dialog halved Zoom's pane and cut its main window off. Now `rules::place`
judges it once, on facts the shell reads (`AXSubrole`, `AXModal`, `AXRole`/`AXParent` for a sheet,
its size, the app pane's size, the width that pane's window has refused to go below):

- **Split**: a standard window that is not small, or any window a `"do": "split"` rule selects.
- **Float**: a dialog or panel subrole, a modal window, a standard window under 600×400 or under
  half the pane both ways, one whose split would leave the pane narrower than its window refused
  to go, or a `"do": "float"` rule. No pane, no stream: the window is moved, at its own size, to
  the middle of its app's pane (`rules::float_to`) unless its middle is already over it, and
  `AXRaise`d once without activating the app.
- **Leave**: a sheet or a child window (macOS keeps it on its parent), an `ignore` rule, a window
  under 100×100 with no rule (the canary, a GPUI app, has a 66×20 one off screen). Any other
  subrole (a toast, a popover) is not judged at all, a `split` rule or not, and is looked at again
  each tick in case it becomes a window.

Each window is judged once, at first sight: one that opens small and grows stays floating.
A pop-up of an app with no pane left is not touched.

AX has no minimum size. The width a window kept when its pane asked for less (read on the
half-second check) stands for one; a window never squeezed has none, and splits.

A floating pop-up is placed once, then left to its app. Live, the app is in front and its pop-up
with it. Parked, ottospace is in front of it, as of every window of the app; entering the pane
brings the app, and its key window, forward again. A child window or sheet moves with its parent,
by macOS. ottospace does not follow, park, give back or close a pop-up.

## Risks

- Capture costs input: while ScreenCaptureKit streams a Finder window, macOS loses a release of
  most quick double-clicks on it (CI, 2026-11-17: 28 of 48 captured, 0 of 48 not), so Finder opens
  nothing. The stream stops while the pane is live and starts again as it parks.
- Capturable ≠ still rendering. Apps may throttle parked/occluded windows; must be measured per app.
- Parked windows must not overlap each other, or they occlude and throttle.
- AX moves/resizes are synchronous IPC to the target app's main thread: busy apps stall; drag-follow lags.
- Min-size / non-resizable windows won't fit small panes. Sheets, popovers and child windows escape panes.
- Z-order: clicking a real window can bury the ottospace window. Fixing that needs private SkyLight APIs.
- Screen Recording: persistent indicator, periodic re-prompts; DRM / `sharingType = .none` windows capture black.
- TCC: ad-hoc rebuilds churn grants. Needs a signed `.app` with a stable identity.
- N streams cost IOSurface memory (~95 MiB at 4K); capture at pane size, adapt frame rate.
- Lifecycle: sleep/wake, display hotplug, Spaces, Stage Manager, crashes leaving windows parked.

Prior art: AeroSpace (sliver parking, AX), yabai (SkyLight), DeskPad / BetterDisplay
(`CGVirtualDisplay`), AltTab (SCK previews).
