# ottospace

A macOS **super agent multiplexer**: agent terminals and the real app windows they work with, tiled in one
workspace. Agents can open panes with app windows and read/drive them through the accessibility tree
(see `docs/ideas.md`).
Rust + [GPUI](https://gpui.rs).

## Approach

macOS can't reparent another process's window, so ottospace fakes embedding:

- **Visible panes** — the real app window, positioned over its pane via the Accessibility (AX) API
  (the AeroSpace model). Native input, no capture.
- **Hidden / covered panes** — a ScreenCaptureKit (SCK) stream of the window, with the real window parked
  on a 1px off-screen sliver.
- **Fallback** — park on a private `CGVirtualDisplay` only if sliver-parked windows stop rendering.

## Risks (from Fable + Astra review)

- Capturable ≠ still rendering. Apps may throttle parked/occluded windows; must be measured per app.
- Parked windows must not overlap each other, or they occlude and throttle.
- AX moves/resizes are synchronous IPC to the target app's main thread: busy apps stall; drag-follow lags.
- Min-size / non-resizable windows won't fit small panes. Sheets, popovers and child windows escape panes.
- Z-order: clicking a real window can bury the ottospace window. Fixing that needs private SkyLight APIs.
- Screen Recording: persistent indicator, periodic re-prompts; DRM / `sharingType = .none` windows capture black.
- TCC: move to a signed `.app` with a stable identity before step 2 (ad-hoc rebuilds churn grants).
- N streams cost IOSurface memory (~95 MiB at 4K); capture at pane size, adapt frame rate.
- Lifecycle: sleep/wake, display hotplug, Spaces, Stage Manager, crashes leaving windows parked.

## Prototype order

1. **Sliver parking + one SCK stream** — does it stay live? Test Chrome, an Electron app, a video player. ← **current** (`probe`)
   - 2026-11-10 Chrome + YouTube (with audio), 5s/phase: ~53 complete fps on-screen and parked. Live.
   - 2026-11-10 Slack (Electron) + animated GIF, unfocused, 10s/phase: ~51 on-screen, ~50 parked. Live.
   - 2026-11-10 Chrome + YouTube muted (site + player), 30s/phase: 39 on-screen, 51 parked, 56 restored. Live.
2. **Real windows over GPUI panes** — swap feel, drag-follow lag, z-order behaviour. ← **current** (`ottospace`)
   - Input forwarding to parked windows is a dead end: Chrome ignores `CGEventPostToPid` clicks even
     on-screen (tried window-under-pointer fields, private window-local location, HID-state source).
     Interaction = place the real window over the pane, then replay the swap click at HID level.
3. **Virtual display** — only if step 1 fails.
4. Layout, terminals, multiple windows.

## Layout

- `crates/ottospace-mac` — platform layer: AX window control, display geometry.
- `crates/probe` — step-1 experiment CLI.
- `harness/` — local input test page (`python3 -m http.server 8765 -d harness`).
- `crates/ottospace` — GPUI app.
- `docs/ideas.md` — feature backlog.

## Run

```sh
# Step 1: play a video / animation in the target app first, then:
cargo run -p probe -- "Google Chrome" 5
```

Prints frames/sec by SCK status for on-screen → sliver → restored. Live = "complete/s" stays near the
on-screen rate in the sliver phase. Needs Accessibility + Screen Recording for your terminal.

GPUI needs the Metal toolchain: `xcodebuild -downloadComponent MetalToolchain`.

## Prior art

AeroSpace (sliver parking, AX), yabai (SkyLight), DeskPad / BetterDisplay (`CGVirtualDisplay`), AltTab (SCK previews).
