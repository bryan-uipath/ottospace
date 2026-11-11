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

## Probe results

`cargo run -p probe -- "Google Chrome" 5` prints frames/sec by SCK status for on-screen → sliver →
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
