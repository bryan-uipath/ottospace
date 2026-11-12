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

## Probe results

Measured with `probe`, a CLI since removed (`d483a37:crates/probe` in git history).
`cargo run -p probe -- "Google Chrome" 5` printed frames/sec by SCK status for on-screen → sliver →
restored. Play a video or animation in the target app first. Live means "complete/s" stays near
the on-screen rate while parked.

- 2026-11-10 Chrome + YouTube (with audio), 5s/phase: ~53 complete fps on-screen and parked. Live.
- 2026-11-10 Slack (Electron) + animated GIF, unfocused, 10s/phase: ~51 on-screen, ~50 parked. Live.
- 2026-11-10 Chrome + YouTube muted (site + player), 30s/phase: 39 on-screen, 51 parked, 56 restored. Live.

## Risks

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
