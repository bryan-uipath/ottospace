# Developing ottospace

Front page: [README](../README.md). What the app does: [reference.md](reference.md).
Architecture rules and the test loops in full: [AGENTS.md](../AGENTS.md).

## Build from source

Requirements:

- macOS 14 or later.
- A Rust toolchain (edition 2024).
- The two permissions in [the reference](reference.md#requirements-and-permissions).

Build and run:

```sh
git clone https://github.com/bryan-uipath/ottospace
cd ottospace
cargo run -p ottospace
```

A window opens. Press ⌘D to open a terminal pane.

## The `ottospace` command

```sh
cargo install --path crates/ottospace --bin ottospace
ottospace
```

This puts `ottospace` on your `PATH`. It shows its icon in the Dock without an app bundle,
and uses the permissions of the terminal you run it from.

## Build the app

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

`scripts/icon.sh` regenerates the icon from `crates/ottospace/assets/icon.svg`, and from
`icon-small.svg` (heavier lines) for the 16 and 32 pixel sizes. It needs Google Chrome.

## Tests

```sh
cargo test   # core unit tests. No window.
cargo e2e    # launches the app and drives it over the socket.
cargo e2e-apps   # hosts real windows of the canary test app; needs both permissions.
```

Terminal panes run your login shell. Set `OTTOSPACE_SHELL` to run another program instead (the
tests use `/bin/sh`). `cargo install` also installs the test app; use `--bin ottospace` to skip it.

`cargo e2e` is an alias in `.cargo/config.toml`. Each test opens a window on a private socket. It
sets `OTTOSPACE_E2E`, which turns window hosting off, so tests never touch your real windows.

### When an e2e test fails or is flaky

Design: [docs/ai/e2e-reliability.md](ai/e2e-reliability.md).

- In a test run, a reply that carries the state waits until a frame has drawn what the request
  did, so its `bounds` are never of the layout before. `frame` in the state has the numbers.
- A failed test writes a folder under `$OTTOSPACE_E2E_OUT` (default `$TMPDIR/ottospace-e2e`),
  `failures/<test>-<pid>/`, and its panic message names it: `summary.txt`, `state.json`,
  `timeline.jsonl` (every request, its time and frame numbers), `trace.jsonl` (the app's own
  events), on a timeout `sample.txt` (where every thread of the app was), and `screenshot.png`
  when the window server allows it. `results.jsonl` beside it holds each test's times and waits.
- `scripts/e2e-repeat 5 [filter]` runs the suite 5 times and tallies each test's failures,
  the slowest tests and the slowest waits (`--apps` for `cargo e2e-apps`, which takes the
  keyboard: prefer the Reliability workflow). `scripts/e2e-report <folder>` tallies again.
- `ottospace ctl '{"cmd":"trace"}'` reads the trace of any running app. `OTTOSPACE_TRACE=1` also
  writes it to `~/.ottospace/trace.jsonl` on a panic or on `kill -USR1`.

## Crates

| Path | Role |
|---|---|
| `crates/ottospace-core` | State and rules: `Model::update(Envelope { action, origin }) -> Vec<Effect>`. Split tree, key encoding. No I/O |
| `crates/ottospace-mac` | macOS calls: AX window control, SCK lookup, display geometry, HID clicks, crash recovery |
| `crates/ottospace-sessions` | Reads Claude Code and Codex session files, read-only: sessions of a directory, events, snapshot. See `docs/ai/sessions.md` |
| `crates/ottospace-agents` | Speaks to Claude Code (`claude -p`, stream-json) and Codex (`codex app-server`) in their structured modes: starts, sends, reads events. Not wired into the app yet. See `docs/ai/agent-runners.md` |
| `crates/ottospace` | The app: GPUI views, terminal, control socket, `ctl` client. Turns input into `Action`s, carries out `Effect`s |
| `sdk/` | TypeScript client for the control socket: [`sdk/README.md`](../sdk/README.md). Tests: `npm test` in `sdk/`, after `cargo build -p ottospace` |
| `harness/` | Local input test page (`python3 -m http.server 8765 -d harness`) |
| `docs/` | Design notes |

## CI

- `.github/workflows/ci.yml`, on pull requests and pushes to `main`: `cargo fmt --check` on the
  formatted crates, `cargo clippy -- -D unused` (unused code fails; locally it only warns), the
  libraries with `-D unreachable_pub`, `cargo machete` (an unused dependency fails),
  `cargo test --workspace`, and the SDK's type check and the tests that need no app. A library's
  `pub` item that no crate uses is not caught. `cargo e2e` runs in a separate job that may fail without blocking, under
  cargo-nextest: a failed test is tried twice more, and one that then passes is listed as flaky in
  the job's summary, with the slowest tests and waits. Failure folders are an artifact. The same
  job then runs the SDK's tests against a private instance.
- `.github/workflows/reliability.yml`, nightly and by hand (`repeat`, `test`): `cargo e2e` and
  `cargo e2e-apps` five times each, tallied. The nightly run keeps the issue "Flaky tests".
- `.github/workflows/build.yml`, on pushes to `main` and on pull requests with the label
  `pr-build`: tests the installer and packages a build. On `main` it publishes the `main-latest`
  prerelease. `osi install pr <n>` adds the label.
- `.github/workflows/scripts.yml`, on pull requests that change `scripts/` or `install.sh`: tests
  the installer.
- `.github/workflows/release.yml`, on a `v*` tag: the GitHub Release.
- `.github/workflows/pages.yml`, on pushes to `main` that change `site/`: deploys `site/` to
  GitHub Pages. Pages must be on for the repository, with GitHub Actions as its source.

What each job runs, and what is not in CI: [AGENTS.md](../AGENTS.md).

## Releasing

1. Bump `version` in the workspace `Cargo.toml` and merge it to `main`.
2. Tag that commit and push the tag: `git tag v0.2.0 && git push origin v0.2.0`.

The release workflow runs `scripts/package.sh` and attaches four files to a GitHub Release:
`ottospace-aarch64-apple-darwin.tar.gz` (the command), `ottospace-aarch64-apple-darwin.app.zip`
(the app), `BUILD` (version, commit, date) and `SHA256SUMS`. It fails if the tag does not match
the crate version. `osi install stable` then installs it.

Every push to `main` that changes more than docs publishes the same four files to the `main-latest` prerelease, whose tag
moves to the new commit (`git fetch --tags` then reports a clobbered tag: use `--force`). Every
pull request gets them as a workflow artifact, kept 14 days.

## The site

`site/` is the static site at <https://bryan-uipath.github.io/ottospace/>: `index.html` (the
landing page), `docs.html` (quickstart and reference) and `shots/` (screenshots of a test
instance with made-up data). Plain HTML, no build step. README is the same pitch in Markdown:
when a claim changes in one, change the other.
