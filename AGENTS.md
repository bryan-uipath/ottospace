# ottospace

macOS agent multiplexer: terminals and real app windows tiled in one workspace. Rust + GPUI.

## Architecture: functional core, imperative shell

| Crate | Role | May use |
|---|---|---|
| `ottospace-core` | State and rules. `Model::update(Envelope { action, origin }) -> Vec<Effect>` | std, serde, unicode-segmentation. No I/O, UI, clock, threads |
| `ottospace-mac` | macOS calls: AX, ScreenCaptureKit, displays, input | system APIs |
| `ottospace-sessions` | Reads Claude Code and Codex session files: sessions of a directory, events, snapshot. A `SessionReader` per `RunnerId` | std, serde, read-only files, `ottospace-core` (pure: for `runner::RunnerId`). No network, clock, writes |
| `ottospace-agents` | Speaks to agent CLIs in their structured modes: starts, sends, reads events. A `Wire` per `RunnerId` | std, serde_json, the two crates above; spawns child processes. No network of its own, clock decisions, or reading other apps |
| `ottospace` | Shell: GPUI views, control socket. Turns input into `Action`s, carries out `Effect`s | everything; the network only to read the calendar feed (`calendar_ics`), through `/usr/bin/curl`, `https:` only |

Rules:

- **A runner is its spec.** What differs between Claude Code and Codex is a field of its
  `RunnerSpec` (`ottospace-core::runner`), asked by capability; never a `match` on its name.
- **Decisions go in the core.** If code chooses what happens next (which pane gets focus, where a
  split goes, what closes), it belongs in `ottospace-core` with a unit test.
- **The shell only translates.** Input → `Action`; `Effect` → system call or draw. All state
  changes go through `Workspace::dispatch`. No second copy of model state in the shell.
- **New behaviour starts as a core test.** Add the `Action`/`Effect`, test it, then wire the shell.
- **Core stays pure.** If the core needs time, randomness or system data, pass it in the `Action`.
- **Every action says who sent it.** `Workspace::dispatch(action, origin, ..)`: `Origin::User` (the
  person's keys, clicks and menu items: pass `self.input`), `Socket` (a control-socket request, also
  a key or click it injects) or `System` (timers, a shell that exited, lists read again). A rule
  about what may interrupt the user asks the origin, not the action.

## Tests

- **Inner loop:** `cargo test`. Core unit tests. Fast, no window.
- **Property tests**, part of `cargo test`: seeded random sequences, with what must hold after every
  step. The model's (`crates/ottospace-core/src/model/tests/prop.rs`) sends every `Action` from every
  `Origin`; `layout.rs`, `compose.rs` and `edit.rs` have their own. A new `Action` needs a maker in
  `MAKERS` (a test fails until it has one), and a new rule about the model belongs in `check`. A
  failure prints its seed and the least steps that still fail, as a test: paste it under
  "Regressions" and fix the bug. `OTTOSPACE_PROP_SEED=<n>` runs one sequence again;
  `cargo test -p ottospace-core --release -- --ignored random_actions` is the long run.
- **Outer loop:** `cargo e2e`. Launches the real app and drives it over the control socket
  (`crates/ottospace/tests/e2e.rs`). Use it for anything the core cannot prove: drawing, layout
  bounds, key bindings, socket protocol.
- In an E2E run a reply with the state waits for the frame that draws its request: read bounds
  from it, no poll needed. A failed test writes a folder that explains it (its panic names it),
  and `scripts/e2e-repeat N [filter]` tallies flaky tests. See `docs/ai/e2e-reliability.md`.
- A test leaves nothing in `$TMPDIR`, pass or fail. An e2e test makes its paths with
  `support::temp`, removed when it ends; a unit test holds a guard that removes its folder on
  drop. `a_test_leaves_nothing_in_the_temp_dir` checks it.
- An E2E run reads no `settings.json` and keeps no tasks on disk, unless the test names its own
  folder with `OTTOSPACE_HOME`. Tests must never read or write the user's `~/.ottospace`.
- E2E runs set `OTTOSPACE_E2E`, which turns window hosting off. Tests must never move the
  user's real windows.
- A `cargo e2e` run stays out of the user's way: the app is a background app (no Dock icon,
  it does not come to the front or take the keyboard by itself), and its window sits at a free
  corner of the screens with a sliver showing (about 12 points wide, as tall as the title bar).
  The sliver floats above other windows on every Space and lets clicks through. It is there
  because GPUI does not draw a window that nobody can see, and a test waiting for layout would
  time out. A layout timeout prints the last state. To look at the window, set
  `OTTOSPACE_E2E_SHOW`: it opens in the middle of the screen and takes clicks, still without
  taking the keyboard by itself.
- **App-window loop:** `cargo e2e-apps`. Hosts real windows of `ottospace-canary`, a test app
  built from this package (`src/bin`). `OTTOSPACE_HOST_ONLY=<pid>` limits hosting (and `ottospace rescue`) to that process. A test
  run keeps the record of hosted windows (`hosted.json`) only in a folder it names with
  `OTTOSPACE_HOME`, and starts the guardian only with `OTTOSPACE_GUARD` set.
  The run takes keyboard focus while it lasts. Needs
  Accessibility and Screen Recording for the terminal. Use it for anything that moves, parks,
  adopts or restores a real window.
- `OTTOSPACE_APPS_DIRS` (folders, `:` between them) replaces the folders scanned for installed apps.
  An E2E run that names none lists none, and never starts an app. `OTTOSPACE_DOCK_APPS` (bundle
  paths, `:` between them) stands for the Dock's apps; a test run that names none has none, and
  never reads the user's Dock.
- Requests and fields that answer only under `OTTOSPACE_E2E`, by one gate (`control::test_refusal`).
  They are documented here and on `Request` in `control.rs`, never in `skill/guide.md`: an agent
  must not learn them.
  - `mouse` (`down`, `move`, `up`, `wheel`, `exit`, `enter`) and `drag` hand events to the window's
    own AppKit handlers, never posted. `wheel` takes `dx`, `dy`: lines, or with `precise` points,
    both rounded to whole numbers.
  - `"as":"user"` on `key`, `mouse` and `drag`: the event is `Origin::User`, for what a socket
    client is refused (sneak mode, a mark's wait, the quit question, a pin in the Apps section).
    Every key and pointer event a client sends is otherwise the client's. Outside a test run a
    client that passed for the user could quit over running programs, end the mode the user is
    typing in, or write their settings. A test run hosts no window and is the test's own
    instance, on its own socket.
  - `repo_add` (`"path"`): the folder the user's picker would give the Repos section's +, as the
    user's own act. Outside a test run only the user adds a directory to their settings, and a
    test run that names no `OTTOSPACE_HOME` writes none.
  - `capture_start`: the user's capture key, pressed by the test (`"pane"` optional). Outside a
    test run only the user starts a capture; the core also refuses a `CaptureStart` of any other
    origin, so a client's key, click or sneak label starts none in a test run either.
  - `sneak_app_tree`: a fixture tree as if read from the focused pane's window while sneak mode
    is on (a test run hosts no window), so its elements get labels. The state's `app_presses`
    (null outside a test run) lists each press a label asked for.
  - `"repeat":true` on `key`: each key as one still held down. A key that types is a real repeat
    to `keyDown:`: `enter`, `tab`, `space`, or a character the layout types with no modifier on
    key codes 0 to 50 (`a`, `7`, `/`; not `A`). A `cmd-` shortcut, by the flag alone, is pressed as
    without `repeat` while `key_repeating()` says held. `escape`, `backspace` and arrows answer
    "no key to hold down".
- A feature is done when both loops cover it. Extend `Request` and the state snapshot when a test
  needs to drive or read something new.

## CI

- `.github/workflows/ci.yml`, on pull requests and pushes to `main`, macOS arm64:
  `cargo fmt -p ottospace -p ottospace-core -p ottospace-sessions -p ottospace-agents --check`, `cargo
  clippy -- -D unused` (errors and unused code fail, other warnings do not; locally unused code only
  warns), the libraries again with `-D unreachable_pub` (a `pub` the crate root cannot reach: make
  it `pub(crate)`), `cargo machete --with-metadata` (an unused dependency fails),
  `cargo test --workspace`, and the SDK's type check and request-fixture tests (`sdk/`, Node). `cargo e2e` runs in a separate job that may fail without blocking, under
  cargo-nextest (`.config/nextest.toml`): up to three tries, a test that passes on a retry is
  listed as flaky in the job summary. The same job then runs the SDK's tests against a private
  instance.
- Unused code is deleted, or its `cfg`/visibility fixed. A library's `pub` item that no crate uses
  is not caught: make `pub(crate)` what only its own crate uses. `#[allow(dead_code)]` only with a one-line
  reason; a machete false positive goes in `[package.metadata.cargo-machete] ignored`, with one.
- `.github/workflows/reliability.yml`, nightly and by hand: `cargo e2e` and `cargo e2e-apps` five
  times each, tallied; the nightly run keeps the issue "Flaky tests" (`issues: write` on that job
  only). Never on pull requests.
- A change that touches only `docs/`, `site/`, `README.md`, `AGENTS.md` or `LICENSE` runs neither
  workflow: macOS runners are scarce. Such a pull request has no checks to wait for, and
  after such a push `main-latest` trails `main` by docs commits.
- `.github/workflows/app-windows.yml`, on pull requests and pushes to `main`: `cargo e2e-apps` on a
  hosted runner, whose image grants bash Accessibility and Screen Recording. Not a required check.
  Prefer it to a local run, which takes the keyboard.
- Not in CI: rustfmt on
  `ottospace-mac` (not formatted yet), clippy `-D warnings` (main has warnings).
- `.github/workflows/build.yml`, on pushes to `main` and on pull requests that carry the label
  `pr-build` (`osi install pr <n>` adds it; without it the jobs are skipped): `scripts/test-osi.sh`,
  and `scripts/package.sh` (release binary, `.app` zip, `BUILD`, `SHA256SUMS`) uploaded as a workflow
  artifact. On `main` a third job, the only one with a write token, replaces the assets of the
  `main-latest` prerelease and moves its tag. Not a required check.
- `.github/workflows/scripts.yml`, on pull requests that change `scripts/**` or `install.sh`:
  `scripts/test-osi.sh`.
- `.github/workflows/release.yml`, on a `v*` tag: `scripts/package.sh`, GitHub Release.
- `.github/workflows/pages.yml`, on pushes to `main` that touch `site/**`: deploys `site/` to GitHub
  Pages. Never on pull requests. Only its deploy job has `pages: write` and `id-token: write`.
- Workflows use `pull_request`, never `pull_request_target`, and name their `permissions:`. Pin
  actions by commit.

## Install scripts

- `install.sh` installs `scripts/osi`; `osi` installs builds (`main`, `pr <n>`, a release) into
  `~/.local/bin` and records them in `~/.ottospace/install`. Design: `docs/ai/install-and-update.md`.
- `osi` reads the asset names, the `main-latest` tag, the workflow file name, the artifact name and
  the label `pr-build`.
  Change them in `scripts/package.sh`, `build.yml` and `scripts/osi` together.
- The app writes `<home>/running` (its pid and resolved executable) at start and removes it on a
  clean quit; subcommands and test runs (`OTTOSPACE_E2E`) write none. `osi` reads it to say the
  old build is still running (`crates/ottospace/src/running.rs`).
- Each build's tarball carries `scripts/osi`; an install replaces the installed `osi` with it
  when its `OSI_VERSION` is higher. **Bump `OSI_VERSION` in `scripts/osi` on every change to it**,
  or installs will not deliver the change.
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

`sdk/` is the TypeScript client for the socket: one method per `Request`, and the reply typed.
When you add or change a request, add or change its method in `sdk/src/client.ts` and its call in
`sdk/test/requests.test.ts`, then run `UPDATE_FIXTURE=1 npm test` in `sdk/` and commit
`sdk/test/requests.jsonl`: a unit test parses that file with `Request` and fails on a request with
no method. When the state gains or loses a key, change `sdk/src/types.ts` and `sdk/src/parse.ts`:
the SDK's tests compare the typed state with a real reply. Design: `docs/ai/sdk.md`.

Agents use the same socket to inspect and drive ottospace. Keep `Request` a closed set, and
keep the socket same-user only: `type` sends keys to a terminal, which is as strong as a shell.
`ax_tree` and `ax_overlay` only read. A capture reads a hosted window's picture and its tree
once and keeps what lies under the user's annotations. Only the user starts one (the capture
key, the menu, the bar): the core refuses a `CaptureStart` of any other origin, and
`capture_start` on the socket answers only under `OTTOSPACE_E2E` (`control::test_refusal`).
In a task's brief (`task_brief`) everything captured is quoted and labelled as untrusted: see
`docs/ai/capture.md`. The user's own sneak label may press a control in the focused app pane
(`docs/ai/sneak-apps.md`): the core refuses a label in an app from any other origin. Requests
that press or set values in another app are still a
separate trust decision: see `docs/ai/1-initial-brainstorm/accessibility.md`.

## App bundle

- `scripts/bundle.sh` builds `target/release/bundle/ottospace.app`. Ship only the `ottospace` binary in it, never `ottospace-canary`.
- `crates/ottospace/assets/ottospace.icns` is generated: change `assets/icon.svg` (and `icon-small.svg`, the 16 and 32 pixel sizes) and run `scripts/icon.sh`, do not edit it.

## Docs

- `README.md` is the front page: the pitch, install, setup and links. Keep it short. Detail lives in
  `docs/reference.md` (status tables and roadmap, keys, panes, settings, `ottospace ctl`, hosting, the list
  of design docs), `docs/install.md` (`osi`, other builds, the app bundle) and `docs/development.md`
  (building, tests, crates, CI, releasing). A new key, setting or capability goes in `docs/reference.md`.
- `site/` is the static site (`index.html`, `docs.html`, `shots/`), deployed to GitHub Pages by
  `.github/workflows/pages.yml`. Keep it in step with README's claims: when one changes, change the
  other, and `site/docs.html` in step with `docs/reference.md`. Neither presents a roadmap item as
  available.
- `docs/cheat-sheet.md` — one page for daily use. Update it when a key, a section or a setting changes.
- `docs/ideas/<n>-<name>.md` — Bryan's own notes. **Never edit, move or reformat.** Read-only input.
- `docs/ai/<n>-<name>/` — AI-derived output for that note: plans, refinements, research.
  e.g. `docs/ideas/1-initial-brainstorm.md` → `docs/ai/1-initial-brainstorm/plan.md`.
  Plan changes go here, never back into `docs/ideas/`.

## Style

- Format with `cargo fmt` (`rustfmt.toml`: 120 columns).
