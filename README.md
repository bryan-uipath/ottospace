# ottospace

<img src="docs/ai/1-initial-brainstorm/background/otto-icon-cyan.svg" width="128" alt="ottospace icon">

ottospace is a macOS workspace that tiles terminals and real app windows in one window. It is for
people who run several coding agents and want them, and the apps around them, in one place.
Agents and tests can drive it over a local control socket. Rust + [GPUI](https://gpui.rs).

## Install

Apple Silicon, macOS 14 or later. Works once the repository is public:

```sh
curl -fsSL https://raw.githubusercontent.com/bryan-uipath/ottospace/main/install.sh | sh
```

The script downloads the latest release, checks its SHA-256, and installs:

- `~/Applications/ottospace.app`, replacing an older copy.
- `~/.local/bin/ottospace`, a wrapper for the app's binary, so `ottospace ctl ...` works. The script
  tells you if `~/.local/bin` is not on your `PATH`.

It needs no `sudo` and is safe to run again to update. To pin a version, run
`... | OTTOSPACE_VERSION=v0.1.0 sh`. Then open the app and grant it Accessibility and Screen
Recording (see Requirements below; the permissions go to ottospace, not the terminal).

Uninstall:

```sh
rm -rf ~/Applications/ottospace.app
rm -f ~/.local/bin/ottospace
```

Releases are signed ad hoc, not with a Developer ID, and not notarized. A zip downloaded in a
browser gets a quarantine flag, and Gatekeeper refuses to open the app. A file fetched with `curl`
carries no such flag, so the install script works without it. Each release is a new identity to
macOS, so you may have to grant the permissions again after an update.

## What it does

| Capability | State |
|---|---|
| Split panes mixing terminals and real app windows | Works today |
| Key bindings and a status bar showing `NORMAL` / `APP` mode | Works today |
| Control socket and `ottospace ctl` for agents and tests | Works today |
| Terminal scrollback, mouse selection, copy and paste | Works today |
| Visual mode: select terminal text with vi keys | Works today |
| Other key modes: insert, sneak | Designed |
| Tasks: each owns a workspace (tabs, then splits) with a brief pane and automatic cleanup | Designed |
| "Pop next": open the next task by priority | Designed |
| Quick capture (⌘⌘): turn something on screen into a task and dispatch it to an agent | Designed |
| Focus blocks: hide Slack, with an agent that can break through | Designed |
| Tabs, each with its own split tree | Works today |
| Pane picker: choose a terminal or a running app for a new pane or tab | Works today |
| Icon rail with a secondary bar; Apps lists running apps | Works today |
| Fast app switcher; Workbook, Tasks and Repos sections | Designed |
| TypeScript scripting, with contextual quick actions in an action deck (Zoom meeting layout, spin for stand-up) | Designed |
| Hide private panes while screen sharing | Designed |
| Session restore | Designed |

## Status

Early prototype, macOS only. Today you can split a window into panes, run your shell in a pane,
host another app's window in a pane, move focus by keyboard, and drive all of it over the control
socket. Tasks, session restore and the rest of the design are not built: see the
[plan](docs/ai/1-initial-brainstorm/plan.md).

## Quick start

Requirements:

- macOS 14 or later.
- A Rust toolchain (edition 2024).
- Accessibility permission for the terminal that launches ottospace: System Settings > Privacy &
  Security > Accessibility. The app exits at start without it.
- Screen Recording permission for the same terminal: System Settings > Privacy & Security > Screen
  & System Audio Recording. macOS asks for it at launch, when ottospace lists running apps.

Build and run:

```sh
git clone https://github.com/bryan-uipath/ottospace
cd ottospace
cargo run -p ottospace
```

A window opens. Press ⌘D to open a terminal pane.

### The `ottospace` command

```sh
cargo install --path crates/ottospace --bin ottospace
ottospace
```

This puts `ottospace` on your `PATH`. It shows the Otto icon in the Dock without an app bundle,
and uses the permissions of the terminal you run it from.

### Build the app

```sh
scripts/bundle.sh
open target/release/bundle/ottospace.app
```

This builds a release binary and wraps it as `ottospace.app` with the icon, so it has a Dock icon
and its own identity.

- Permissions attach to the app, not the terminal: macOS asks for Accessibility and Screen
  Recording for ottospace itself.
- The signature is ad hoc, so each rebuild is a new identity to macOS and you may have to grant
  both again. A stable identity needs a signing certificate, which the script does not set up.
- Opened from Finder or the Dock, terminal panes start in `/`, not your home directory.

`scripts/icon.sh` regenerates the icon from the SVG. It needs Google Chrome.

### Releasing

1. Bump `version` in the workspace `Cargo.toml` and merge it to `main`.
2. Tag that commit and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.

The release workflow builds the app with `scripts/bundle.sh` and attaches
`ottospace-<version>-aarch64-apple-darwin.zip` and its `.sha256` to a GitHub Release. It fails if
the tag does not match the crate version.

## Using it

| Key | Action |
|---|---|
| ⌘D | Split right, new terminal |
| ⌘⇧D | Split down, new terminal |
| ⌘E | Pick what goes in a new pane: a terminal or a running app |
| ⌘W | Close the focused pane |
| ⌘⌥←/→/↑/↓ or ⌘⌥H/L/K/J | Focus the pane in that direction |
| ⌘] / ⌘[ | Focus the next / previous pane |
| ⌘⌃F | Zoom the window to fill the screen; again to restore |
| ⌘⌃A | Debug: outline the focused app pane's accessibility targets (parked panes only) |
| ⌘/ | Show every key |
| F18 | From any app: take the keyboard back and bring ottospace forward |
| ⌘T | New tab: pick what opens in it |
| ⌘1 … ⌘9 | Select a tab |
| ⌘⇧] / ⌘⇧[ or ⌃Tab / ⌃⇧Tab | Next / previous tab |
| ⌘⇧W | Close the tab and its panes |
| Wheel / trackpad | Scroll a terminal back through its history |
| ⇧PageUp / ⇧PageDown | Scroll a terminal a page |
| ⇧Home / ⇧End | Top / bottom of a terminal's history |
| Drag / double-click / triple-click | Select text / a word / a line in a terminal |
| ⌘C | Copy the terminal's selection (nothing selected: does nothing) |
| ⌘V | Paste into the terminal |
| ⌘⇧C | Visual mode on the terminal: `hjkl`/arrows, `w b e 0 ^ $`, `H M L` move; `v` / `V` select; `y` or ↵ copies and leaves; Esc leaves |

Other keys go to the focused terminal. ⌃ sends control codes, ⌥ is Meta.

A terminal pane runs your shell on a PTY. The pane closes when the shell exits. It keeps 10,000
lines of history. Typing returns to the bottom. In programs that use the mouse or the alternate
screen (vim, less, htop) the wheel does nothing, and ⇧PageUp and friends go to the program.
A click on a terminal focuses it and clears its selection; a drag there selects, even in an
unfocused pane. Paste is bracketed when the program asks for it.

To put an app window in a pane, open Apps on the left rail and click the app. It fills the focused pane if that is
empty, otherwise it splits right. Shift-click splits down. New windows of a hosted app get their
own pane. Original window frames are saved to disk and restored on quit or after a crash.

Click an app pane to give it the keyboard (`APP` mode). ottospace shortcuts do not fire in that
mode. Press the return key, or click ottospace itself (rail, top bar, another pane), to return.

The return key works while any app is frontmost. It is F18 by default; set `OTTOSPACE_RETURN_KEY`
to another of `f13`-`f20`. If another program already holds the key, ottospace logs that and runs
without a return key. Most keyboards have no F18, so map Caps Lock to it (lasts until restart):

```sh
hidutil property --set '{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":0x700000039,"HIDKeyboardModifierMappingDst":0x70000006D}]}'
```

After this, Caps Lock no longer toggles capitals anywhere. The command replaces any key mapping
you already have. To undo it: `hidutil property --set '{"UserKeyMapping":[]}'`.

## Driving it from outside

ottospace listens on a Unix socket so tests and agents can inspect and drive it. The socket is mode
0600, at `$OTTOSPACE_SOCKET` or `$TMPDIR/ottospace.sock`. Send one JSON request per line. The reply
is one JSON line: the full state (model, mode, pane bounds, terminal grids) or
`{"ok":false,"error":"..."}`. `ottospace ctl` sends one request, prints the reply, and exits non-zero
if the request was refused. `cargo run` does not put `ottospace` on your `PATH`: use the built binary.

```sh
alias ottospace=./target/debug/ottospace
ottospace ctl '{"cmd":"split","axis":"down"}'
ottospace ctl '{"cmd":"type","text":"ls\r"}'
ottospace ctl '{"cmd":"open_app","name":"Safari"}'
ottospace ctl '{"cmd":"section","section":"apps"}'
ottospace ctl '{"cmd":"state"}'
```

`ottospace skill` prints every request with an example, and what the state fields mean. Fields are
in `Request` in [`crates/ottospace/src/control.rs`](crates/ottospace/src/control.rs). Unknown commands and
fields are refused.

## Agent skill

`ottospace skill` prints a guide for coding agents: the requests, the state fields, and rules for
using them safely. To make it a Claude Code skill, install the stub that points at it:

```sh
mkdir -p ~/.claude/skills/ottospace
ottospace skill --stub > ~/.claude/skills/ottospace/SKILL.md
```

The stub tells the agent to run `ottospace skill`, so the guide always matches the installed
version. A test checks that every request shown in the guide is one the app accepts.

## Development

```sh
cargo test   # core unit tests. No window.
cargo e2e    # launches the app and drives it over the socket.
cargo e2e-apps   # hosts real windows of the canary test app; needs both permissions.
```

Terminal panes run your login shell. Set `OTTOSPACE_SHELL` to run another program instead (the
tests use `/bin/sh`). `cargo install` also installs the test app; use `--bin ottospace` to skip it.

`cargo e2e` is an alias in `.cargo/config.toml`. Each test opens a window on a private socket. It
sets `OTTOSPACE_E2E`, which turns window hosting off, so tests never touch your real windows.

| Path | Role |
|---|---|
| `crates/ottospace-core` | State and rules: `Model::update(Action) -> Vec<Effect>`. Split tree, key encoding. No I/O |
| `crates/ottospace-mac` | macOS calls: AX window control, SCK lookup, display geometry, HID clicks, crash recovery |
| `crates/ottospace` | The app: GPUI views, terminal, control socket, `ctl` client. Turns input into `Action`s, carries out `Effect`s |
| `harness/` | Local input test page (`python3 -m http.server 8765 -d harness`) |
| `docs/` | Design notes |

Architecture rules and the test loops are in [AGENTS.md](AGENTS.md).

## How app windows are hosted

macOS cannot reparent another process's window, so ottospace fakes embedding with the Accessibility
(AX) API and ScreenCaptureKit (SCK). An inactive app window is parked off-screen with one pixel on
a display, and its pane shows an SCK stream of it. Clicking the pane moves the real window over it
and replays the click, so input is native. A pane shows one title bar: the stream leaves out the
app's own, and the live window's covers the pane header. Measurements and risks: [docs/ai/hosting-windows.md](docs/ai/hosting-windows.md).

## Design docs

[`docs/ai/1-initial-brainstorm/`](docs/ai/1-initial-brainstorm/) holds the AI-derived notes for the
first brainstorm:

- [plan.md](docs/ai/1-initial-brainstorm/plan.md): model, phases and decisions.
- [quick-capture.md](docs/ai/1-initial-brainstorm/quick-capture.md): quick capture and task lifecycle.
- [screen-share-privacy.md](docs/ai/1-initial-brainstorm/screen-share-privacy.md): hiding panes from screen sharing.
- [scripting.md](docs/ai/1-initial-brainstorm/scripting.md): scripting in TypeScript.
- [zoom-meeting.md](docs/ai/1-initial-brainstorm/zoom-meeting.md): Zoom meeting layout and participants.
- [accessibility.md](docs/ai/1-initial-brainstorm/accessibility.md): reading app accessibility trees, toward sneak mode and agent actions.
- `background/`: icon and mockup images.

[`docs/ai/backlog.md`](docs/ai/backlog.md) is an older backlog. `docs/ideas/` holds the owner's own
notes.
