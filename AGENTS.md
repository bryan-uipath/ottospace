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
- E2E runs set `OTTOSPACE_E2E`, which turns window hosting off. Tests must never move the
  user's real windows.
- E2E windows float above other windows, because GPUI does not draw a fully covered window and
  a test waiting for layout would time out. A layout timeout prints the last state.
- **App-window loop:** `cargo e2e-apps`. Hosts real windows of `ottospace-canary`, a test app
  built from this package (`src/bin`). `OTTOSPACE_HOST_ONLY` limits hosting to that app. Needs
  Accessibility and Screen Recording for the terminal. Use it for anything that moves, parks,
  adopts or restores a real window.
- A feature is done when both loops cover it. Extend `Request` and the state snapshot when a test
  needs to drive or read something new.

## Control socket

Unix socket, mode 0600, at `$OTTOSPACE_SOCKET` or `$TMPDIR/ottospace.sock`. One JSON request per
line, one JSON reply (the full state) per line. See `Request` in `crates/ottospace/src/control.rs`.

```
ottospace ctl '{"cmd":"split","axis":"down"}'
ottospace ctl '{"cmd":"type","text":"ls\r"}'
ottospace ctl '{"cmd":"state"}'
```

Agents use the same socket to inspect and drive ottospace. Keep `Request` a closed set, and
keep the socket same-user only: `type` sends keys to a terminal, which is as strong as a shell.

## Docs

- `docs/ideas/<n>-<name>.md` — Bryan's own notes. **Never edit, move or reformat.** Read-only input.
- `docs/ai/<n>-<name>/` — AI-derived output for that note: plans, refinements, research.
  e.g. `docs/ideas/1-initial-brainstorm.md` → `docs/ai/1-initial-brainstorm/plan.md`.
  Plan changes go here, never back into `docs/ideas/`.

## Style

- Format with `cargo fmt` (`rustfmt.toml`: 120 columns).
