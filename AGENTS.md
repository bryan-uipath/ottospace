# ottospace

macOS agent multiplexer: terminals and real app windows tiled in one workspace. Rust + GPUI.

## Architecture: functional core, imperative shell

| Crate | Role | May use |
|---|---|---|
| `ottospace-core` | State and rules. `Model::update(Action) -> Vec<Effect>` | std, serde. No I/O, UI, clock, threads |
| `ottospace-mac` | macOS calls: AX, ScreenCaptureKit, displays, input | system APIs |
| `ottospace` | Shell: GPUI views, control socket. Turns input into `Action`s, carries out `Effect`s | everything |

Rules:

- **Decisions go in the core.** If code chooses what happens next (which pane gets focus, where a
  split goes, what closes), it belongs in `ottospace-core` with a unit test.
- **The shell only translates.** Input → `Action`; `Effect` → system call or draw. All state
  changes go through `Workspace::dispatch`. No second copy of model state in the shell.
- **New behaviour starts as a core test.** Add the `Action`/`Effect`, test it, then wire the shell.
- **Core stays pure.** If the core needs time, randomness or system data, pass it in the `Action`.

## Tests

- **Inner loop:** `cargo test`. Core unit tests. Fast, no window.
- **Outer loop:** `cargo e2e`. Launches the real app and drives it over the control socket
  (`crates/ottospace/tests/e2e.rs`). Use it for anything the core cannot prove: drawing, layout
  bounds, key bindings, socket protocol.
- An E2E run reads no `settings.json` and keeps no tasks on disk, unless the test names its own
  folder with `OTTOSPACE_HOME`. Tests must never read or write the user's `~/.ottospace`.
- E2E runs set `OTTOSPACE_E2E`, which turns window hosting off. Tests must never move the
  user's real windows.
- E2E windows float above other windows, because GPUI does not draw a fully covered window and
  a test waiting for layout would time out. A layout timeout prints the last state.
- **App-window loop:** `cargo e2e-apps`. Hosts real windows of `ottospace-canary`, a test app
  built from this package (`src/bin`). `OTTOSPACE_HOST_ONLY=<pid>` limits hosting to that process, and turns the recovery file off.
  The run takes keyboard focus while it lasts. Needs
  Accessibility and Screen Recording for the terminal. Use it for anything that moves, parks,
  adopts or restores a real window.
- A feature is done when both loops cover it. Extend `Request` and the state snapshot when a test
  needs to drive or read something new.

## CI

- `.github/workflows/ci.yml`, on pull requests and pushes to `main`, macOS arm64:
  `cargo fmt -p ottospace -p ottospace-core --check`, `cargo clippy` (errors fail, warnings do not),
  `cargo test --workspace`. `cargo e2e` runs in a separate job that may fail without blocking.
- Not in CI: `cargo e2e-apps` (needs Accessibility and Screen Recording), rustfmt on
  `ottospace-mac` (not formatted yet), clippy `-D warnings` (main has warnings).
- `.github/workflows/build.yml`, on pull requests and pushes to `main`: `scripts/test-osi.sh`, and
  `scripts/package.sh` (release binary, `.app` zip, `BUILD`, `SHA256SUMS`) uploaded as a workflow
  artifact. On `main` a third job, the only one with a write token, replaces the assets of the
  `main-latest` prerelease and moves its tag. Not a required check.
- `.github/workflows/release.yml`, on a `v*` tag: `scripts/package.sh`, GitHub Release.
- Workflows use `pull_request`, never `pull_request_target`, and name their `permissions:`. Pin
  actions by commit.

## Install scripts

- `install.sh` installs `scripts/osi`; `osi` installs builds (`main`, `pr <n>`, a release) into
  `~/.local/bin` and records them in `~/.ottospace/install`. Design: `docs/ai/install-and-update.md`.
- `osi` reads the asset names, the `main-latest` tag, the workflow file name and the artifact name.
  Change them in `scripts/package.sh`, `build.yml` and `scripts/osi` together.
- Test with `scripts/test-osi.sh`: fixture builds, a temp `HOME`, no network. Never run
  `install.sh` or `osi` against the real `HOME` in a test; every path has an override.

## Control socket

Unix socket, mode 0600, at `$OTTOSPACE_SOCKET` or `$TMPDIR/ottospace.sock`. One JSON request per
line, one JSON reply (the full state) per line. See `Request` in `crates/ottospace/src/control.rs`.

```
ottospace ctl '{"cmd":"split","axis":"down"}'
ottospace ctl '{"cmd":"type","text":"ls\r"}'
ottospace ctl '{"cmd":"state"}'
ottospace ctl '{"cmd":"open_repo","path":"/Users/me/code/app"}'  # a terminal in a listed repository
ottospace ctl '{"cmd":"capture","pane":1}'                 # one terminal pane's text, not the state
ottospace ctl '{"cmd":"wait","pane":1,"pattern":"^done$"}'  # replies when a line matches
ottospace ctl '{"cmd":"ax_tree"}'   # the focused app pane's accessibility tree, not the state
```

`crates/ottospace/skill/guide.md` is the agent-facing guide (`ottospace skill` prints it). When
you add or change a request, update the guide; a unit test parses every example in it.

Agents use the same socket to inspect and drive ottospace. Keep `Request` a closed set, and
keep the socket same-user only: `type` sends keys to a terminal, which is as strong as a shell.
`ax_tree` and `ax_overlay` only read. Requests that press or set values in another app are a
separate trust decision: see `docs/ai/1-initial-brainstorm/accessibility.md`.

## App bundle

- `scripts/bundle.sh` builds `target/release/bundle/ottospace.app`. Ship only the `ottospace` binary in it, never `ottospace-canary`.
- `crates/ottospace/assets/ottospace.icns` is generated: change the SVG and run `scripts/icon.sh`, do not edit it.

## Docs

- `docs/ideas/<n>-<name>.md` — Bryan's own notes. **Never edit, move or reformat.** Read-only input.
- `docs/ai/<n>-<name>/` — AI-derived output for that note: plans, refinements, research.
  e.g. `docs/ideas/1-initial-brainstorm.md` → `docs/ai/1-initial-brainstorm/plan.md`.
  Plan changes go here, never back into `docs/ideas/`.

## Style

- Format with `cargo fmt` (`rustfmt.toml`: 120 columns).
