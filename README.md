# ottospace

A macOS agent multiplexer: terminals and real app windows tiled in one workspace. Agents and tests
drive it over a local control socket. Rust + [GPUI](https://gpui.rs).

## Status

Prototype. Phase A of the [plan](docs/ai/1-initial-brainstorm/plan.md).

Works today:

- **Split tree.** Binary splits, right or down, equal halves. Focus moves by direction.
- **Terminal panes.** The user's shell on a PTY, parsed by `alacritty_terminal`, drawn by our own
  renderer. Keyboard input, 256 colours, resizes with the pane. The pane closes when the shell exits.
- **App-window panes.** Another app's window hosted in a pane through the Accessibility (AX) API
  and ScreenCaptureKit (SCK). New windows of a hosted app get their own pane. Original window
  frames are saved to disk and restored on quit or after a crash.
- **Sidebar.** Lists running apps. Click one to host its main window.
- **Key bindings.** Split, close, focus by direction.
- **Status bar.** `NORMAL` / `APP` mode and the focused pane.
- **Control socket** and the `ottospace ctl` client.
- **Tests.** Core unit tests, plus e2e tests that launch the app and drive it over the socket.

Not yet:

- Tabs, tasks
- Sidebar categories, app switcher
- Focus mode
- Persistence (session restore)
- Global key to return from `APP` mode
- Draggable dividers

## Requirements

- macOS 14 or later (ScreenCaptureKit features in use).
- Rust, edition 2024.
- Accessibility and Screen Recording permission for the terminal that launches it.
  The app exits at start without Accessibility. Screen Recording is requested on first capture.

## Run

```sh
cargo run -p ottospace
```

Open a pane with ⌘D, or click an app in the sidebar. Shift-click splits down instead of right.

## Keys

| Key | Action |
|---|---|
| ⌘D | Split right, new terminal |
| ⌘⇧D | Split down, new terminal |
| ⌘W | Close the focused pane |
| ⌘⌥←/→/↑/↓ | Focus the pane in that direction |

Other keys go to the focused terminal. ⌃ sends control codes, ⌥ is Meta.

Click an app pane to give it the keyboard (`APP` mode). ottospace shortcuts do not fire in that
mode. Click ottospace itself (sidebar, title bar, another pane) to return.

## Control socket

Unix socket, mode 0600, at `$OTTOSPACE_SOCKET` or `$TMPDIR/ottospace.sock`. One JSON request per
line. The reply is one JSON line: the full state (model, mode, pane bounds, terminal grids) or
`{"ok":false,"error":"..."}`.

```sh
ottospace ctl '{"cmd":"split","axis":"down"}'
ottospace ctl '{"cmd":"type","text":"ls\r"}'
ottospace ctl '{"cmd":"open_app","name":"Safari"}'
ottospace ctl '{"cmd":"state"}'
```

Commands: `state`, `split`, `type`, `close`, `focus`, `focus_dir`, `open_app`, `enter`, `leave`,
`quit`. Fields are in `Request` in [`crates/ottospace/src/control.rs`](crates/ottospace/src/control.rs).
Unknown commands and fields are refused.

## Tests

```sh
cargo test   # core unit tests. No window.
cargo e2e    # launches the app and drives it over the socket.
```

`cargo e2e` is an alias in `.cargo/config.toml`. It sets `OTTOSPACE_E2E`, which turns window
hosting off, so tests never move real windows.

## Layout

| Path | Role |
|---|---|
| `crates/ottospace-core` | State and rules: `Model::update(Action) -> Vec<Effect>`. Split tree, key encoding, palette. No I/O |
| `crates/ottospace-mac` | macOS calls: AX window control, SCK lookup, display geometry, HID clicks, crash recovery |
| `crates/ottospace` | The app: GPUI views, terminal, control socket, `ctl` client. Turns input into `Action`s, carries out `Effect`s |
| `crates/probe` | CLI that measures whether a parked window keeps rendering |
| `harness/` | Local input test page (`python3 -m http.server 8765 -d harness`) |
| `docs/ai/` | Plan and backlog |

## How app-window hosting works

macOS cannot reparent another process's window, so ottospace fakes embedding:

- **Parked.** The real window sits off-screen with only its top-left pixel on a display (the
  AeroSpace model). The pane shows an SCK stream of it. The window is resized to the pane so the
  stream matches.
- **Live.** Clicking the pane moves the real window over it through AX and raises it. The click
  is then replayed at HID level so it lands in the app. Input is native. No capture is involved.
- **Fallback, not built.** Park on a private `CGVirtualDisplay` if sliver-parked windows stop
  rendering.

Forwarding input to parked windows is a dead end: Chrome ignores `CGEventPostToPid` clicks even
on-screen.

### Probe results

`cargo run -p probe -- "Google Chrome" 5` prints frames/sec by SCK status for on-screen → sliver →
restored. Play a video or animation in the target app first. Live means "complete/s" stays near
the on-screen rate while parked.

- 2026-11-10 Chrome + YouTube (with audio), 5s/phase: ~53 complete fps on-screen and parked. Live.
- 2026-11-10 Slack (Electron) + animated GIF, unfocused, 10s/phase: ~51 on-screen, ~50 parked. Live.
- 2026-11-10 Chrome + YouTube muted (site + player), 30s/phase: 39 on-screen, 51 parked, 56 restored. Live.

### Risks

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

## More

- [AGENTS.md](AGENTS.md): architecture rules, test loops, control socket.
- [docs/ai/1-initial-brainstorm/plan.md](docs/ai/1-initial-brainstorm/plan.md): plan, phases, decisions.
