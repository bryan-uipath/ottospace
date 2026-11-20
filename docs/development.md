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

The property tests run seeded random sequences of actions. The long run is
`cargo test -p ottospace-core --release -- --ignored random_actions`. `OTTOSPACE_PROP_SEED=<n>`
runs one sequence again, and `OTTOSPACE_PROP_WORKERS=<n>` caps each property test's threads (every
core by default; `nice -n 15` and 4 workers leave the machine usable).

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
- `scripts/e2e-for [<base>]` (default `origin/main`) lists the e2e tests your change can affect,
  each with why it was picked; `--run` runs them one at a time under nextest, `--ci-size` at CI's
  window (`OTTOSPACE_E2E_WINDOW=1024x674`; CI's screen is 1024 wide). A path its table
  (`scripts/e2e-for.areas`) does not know gives the full-suite command instead. Before a pull
  request merges on local evidence, run `scripts/e2e-for --run --ci-size` and paste its summary.
- `OTTOSPACE_E2E_WINDOW=<w>x<h> cargo e2e -- <test>` runs one test at another window size.
- `scripts/e2e-repeat 5 [filter]` runs the suite 5 times and tallies each test's failures,
  the slowest tests and the slowest waits (`--apps` for `cargo e2e-apps`, which takes the
  keyboard: prefer the Reliability workflow). `scripts/e2e-report <folder>` tallies again.
- `ottospace ctl '{"cmd":"trace"}'` reads the trace of any running app. `OTTOSPACE_TRACE=1` also
  writes it to `~/.ottospace/trace.jsonl` on a panic or on `kill -USR1`.

## Screenshots and recordings

Every pull request shows evidence (AGENTS.md, "Pull requests"). `scripts/shot` takes the pictures:

```sh
scripts/shot tasks chat                 # .shots/<scenario>/<shot>.png
scripts/shot terminal-copy --record     # also .shots/terminal-copy.gif and .mp4 (needs ffmpeg)
scripts/shot --compare origin/main calendar   # .shots/before, after, compare; prints a snippet
scripts/shot --list
```

It builds this tree (or takes `--bin`), starts a private instance (`OTTOSPACE_E2E`, its home and
socket in a fresh temp folder, the agents crate's stand-ins as `claude`/`codex`, no apps, no
repositories of yours), runs the scenario, takes its window by window id with `screencapture
-l` (never a screen region), and removes the folder. `.shots/` is ignored by git: upload the
pictures with the `pr-image-upload` skill, never commit them.

- **The window's whole picture.** A test window sits at a free corner with a sliver on screen.
  `screencapture -l` still takes all of it: the window server keeps the window's own picture.
  The sliver is what keeps GPUI drawing (a window nothing of which shows is not drawn). A
  locked screen draws nothing: the script stops and says so, and refuses a picture of one
  colour or with most of it see-through.
- **Size.** The window is 1400x900 points where the screen has room: 2800x1800 pixels on a
  Retina screen. A hosted runner's screen is 1024x768, so there it is 1024x674. Both sides of a
  compare run on one machine, so their pictures always match.
- **When.** A state reply waits for the frame that draws its request; each shot waits for that,
  then 150 ms for the window server.
- **Recordings.** `--record` films the window with ScreenCaptureKit through the test-only
  `record_start`/`record_stop` requests (AGENTS.md, "Tests"): a frame per change, timed by the
  app, resampled to a steady 30 fps MP4 and a 15 fps GIF. A debug build draws motion at about
  20 frames a second. While it records, `screencapture -l` sees only the sliver, so a shot then
  comes from the recording's last frame, at one pixel a point.
- **Compare.** `--compare <ref>` builds the ref in a temporary worktree with its own target
  folder (a clone of this one's), runs the same scenarios with the same fixtures on both, and
  writes one labelled side-by-side per shot. A scenario that fails on the base (its view did not
  exist) gives an "after only" picture. `--bin-before`/`--bin-after` skip the builds.
- **CI.** `.github/workflows/shots.yml` runs `--compare` against the merge base on a hosted runner
  for a pull request with the label `pr-shots` that changes the app's sources or the scenarios, and keeps the pictures as
  the artifact `shots`. It writes nothing to the pull request: fetch them with
  `gh run download <run id> -n shots`, then upload them with the `pr-image-upload` skill.
- **Its test.** `scripts/test-shot.sh [binary]`: one scenario, a compare of a build with itself,
  and nothing left in `$TMPDIR`. It needs a screen that draws; `shots.yml` runs it.

### Adding a scenario

`scripts/shots/<name>.jsonl`: control-socket requests, one a line, and the script's own steps.
A folder `scripts/shots/<name>/` is copied into the instance's folder first (`home/settings.json`,
`repos/`, `fixtures/` for the stand-in `claude`); `_git` there becomes `.git`. `{ROOT}` (that
folder), `{REPO}` (the checkout), `{NOW+3600}` (an ICS time from now) and `{TODAY+34200}`
(09:30 today, local: what must fall on today whatever the hour) are filled in.

| Step | Does |
|---|---|
| `{"cmd":..}` | a request; it must answer `ok` |
| `{"shot":"name"}` | waits for the frame, takes `name.png` |
| `{"click":"task:1"}`, `{"hover":"task:1"}` | the user's pointer on a named control (`"at":[0.3,0.5]`), or on a rect of the state: `{"hover":{"rect":"calendar.band.blocks.2.rect"}}` |
| `{"drag":{"rect":"drawer.body"},"at":[0,0.5],"by":[150,0]}` | the user's drag from a control or a rect of the state, by points (here: the drawer's edge, 150 narrower) |
| `{"until":"drawer.native.chats.1.transcript.cards","len":1}` | waits on the state (`"is"`, `"len"`, or there at all) |
| `{"settle":500}`, `{"sleep_ms":700}` | waits |
| `{"record":"start"}`, `{"record":"stop"}` | what `--record` covers (default: all of it) |
| `{"write":"home/settings.json","text":"{}"}` | a file in the instance's own folder, as an editor saves it |
| `{"env":{..}}` | first line only: more environment for the instance |
| `{"controls":true}`, `{"print":"repos_bar"}` | prints the named controls, or a part of the state, while writing one |

Name controls and read the state; never pixel positions, which move with every layout change.
Requests a client is refused (keys and clicks that are the user's) take `"as":"user"`, which
answers in a test run only. Made-up content only.

## Benchmarks

`scripts/bench` measures startup, memory and CPU on private test instances (`scripts/shot`'s:
`OTTOSPACE_E2E`, home and socket in a fresh temp folder, `/bin/sh` terminals, made-up content):

```sh
scripts/bench                        # this tree: .bench/bench.json and bench.md
scripts/bench --compare origin/main  # the base and this tree, alternating; prints the deltas
scripts/bench --release --runs 20    # release builds, more launches
```

It builds the dev profile by default, as CI does: CI's Rust cache holds the dev build's
dependencies (`shared-key: e2e`), so a pull request rebuilds only the workspace's crates, twice.
A release build of GPUI from nothing takes too long for every pull request. Dev numbers are slower
than a user's; compare them with each other, not with a release build.

| Number | What it is |
|---|---|
| startup, warm | process start until the socket answers and a frame is drawn (`frame.number` > 0). Median and p90 of `--runs` (10) launches, after one unmeasured |
| startup, cold | the same, the first launch of a fresh `cp -c` copy of the binary at a new path: none of its pages cached, no launch closure for it. The system's own libraries stay cached (dropping those needs root `purge`), so this is "first launch after a build", not after a reboot. `--cold-runs` (5) |
| startup, 50 sessions | the same, with a home of 50 made-up Claude Code and Codex sessions in 5 repositories, 12 tasks and a saved layout of 3 tabs and 6 terminals |
| then the sessions listed | with that home, until the Tasks section, opened on Elsewhere, lists them |
| RSS, footprint, idle | resident size and `phys_footprint` (what Activity Monitor shows as Memory), 5 s after start |
| RSS, footprint, 8 terminals | after 8 terminals each printed 5000 lines of scrollback |
| CPU, idle | the app's CPU over 30 s with nothing moving, 100 = one core; with its wakeups a second, frames drawn and `counts.dial_frames` (both 0 at rest), and the tick's runs (`counts.ticks`: each frame for a second after the first request, then about one a second) |
| CPU, streaming | over 10 s while one terminal prints about 2500 lines a second: a fixed load |

Memory and CPU come from `proc_pid_rusage` of the app's own pid, never its terminals' shells.

Thresholds (`THRESHOLDS` in the script), past which `--compare` exits 1 and CI's job is red: a
startup median +20% and +50 ms, RSS +25% and +10 MB, idle CPU above 2% of a core and a point more
than the base. A startup past its threshold runs a second round of launches and fails only if it
is past it in each round. Exit 2: the bench itself failed (a build, a timeout), not a regression.

Noise. Both sides run on one machine, alternating, so drift falls on both. On an M-series Mac, two
runs of `main` against itself differed by at most 4% on startup medians, 7% (13 MB) on RSS with 8
terminals and 0.2 points of idle CPU. A hosted runner is far noisier: one launch takes 345 to 1090
ms, and a run of identical code once came out +20% on warm startup. So CI launches 20 (cold: 10)
times a round. A red bench is not a required check: whoever merges says in the pull request
whether the regression is real and accepted.

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

- `.github/workflows/ci.yml`'s `check`, on pull requests, the merge queue and pushes to `main`, on
  Linux: `cargo fmt --check` on the formatted crates; on the three pure crates `cargo clippy -- -D
  unused -D unreachable_pub`, for Linux and for `aarch64-apple-darwin` (checked, not linked),
  and `cargo test`; `cargo machete`; the SDK's type check and the tests that need no app. A pull
  request takes no macOS runner unless labelled (an installer change still runs `scripts.yml` on one). `check` does not compile `ottospace-mac` or the
  app: their build scripts need Apple's SDK (Swift bridges, `cc`, `bindgen`), which is not put on
  a Linux runner.
- `.github/workflows/macos.yml`, the one macOS job: clippy over the workspace, `cargo test
  --workspace`, `cargo e2e` under cargo-nextest (a failed test is tried twice more, and one that
  then passes is listed as flaky in the summary, with the slowest tests and waits; failure folders
  are an artifact), the SDK's tests against a private instance, then `cargo e2e-apps`. One build;
  each suite runs though an earlier one failed, and the summary names the failed tests. On every
  push to `main` (`ci.yml` calls it; a newer push cancels the run). While main's or `check` fails, the issue
  "main is red" lists the failed tests and the commits since the last green run; it closes itself.
- `.github/workflows/e2e.yml`, on pull requests with the label `pr-e2e`: `macos.yml` with the e2e
  tests `scripts/e2e-for` selects from the diff (all of them when the diff reaches every test), no app-window
  suite; `pr-e2e-apps` adds it. Otherwise a pull request's e2e evidence is a local `scripts/e2e-for`
  run's summary in its body.
- `.github/workflows/app-windows.yml`, by hand: `cargo e2e-apps`, or one test repeated.
- `.github/workflows/reliability.yml`, nightly and by hand (`repeat`, `test`): `cargo e2e` and
  `cargo e2e-apps` five times each, tallied. The nightly run keeps the issue "Flaky tests", one it opened (never one a
  person opened).
- `.github/workflows/build.yml`, on pushes to `main` and on pull requests with the label
  `pr-build`: tests the installer and packages a build. On `main` it publishes the `main-latest`
  prerelease. `osi install pr <n>` adds the label.
- `.github/workflows/scripts.yml`, on pull requests that change `install.sh`, `scripts/osi`,
  `scripts/test-osi.sh` or `scripts/package.sh`: tests the installer.
- `.github/workflows/duped.yml`, on pull requests (not docs-only ones) and `merge_group`, on Linux:
  `duped check --base origin/<base>` (a merge group: its `base_sha`) fails when the change adds a helper identical to one
  already there, or an identical file of 10 lines or more. A deliberate copy is acknowledged in
  `duped.json`, with a `reason` saying why the two stay separate. The
  release binary is pinned by version and checksum. Run it locally with
  `duped check --base origin/main`.
- The Rust build cache: `macos.yml` saves one, on `main` only; its `pr-e2e` runs, `app-windows.yml`,
  `shots.yml`, `bench.yml` and `reliability.yml` read it (`shared-key: e2e`) and save none. `check`
  (Linux) and `build` keep their own.
- `.github/workflows/shots.yml`, on pull requests with the label `pr-shots` that change the app's,
  core's or mac crate's sources or the scenarios: before and after pictures, an artifact (see "Screenshots and recordings").
- `.github/workflows/bench.yml`, on pull requests with the label `pr-bench` that change a crate, `Cargo.lock`,
  `scripts/bench`, `scripts/shot` or the shots fixtures it copies: `scripts/bench --compare` against the base branch's tip (the pull request merged into it after), the table
  in the job summary, the JSON as an artifact (see "Benchmarks"). Nightly and by hand on `main`:
  this tree alone, the JSON kept 90 days for trends.
- `.github/workflows/deps.yml`, on pull requests that change `Cargo.lock`, a `Cargo.toml`,
  `deny.toml` or the workflow, and weekly on `main`, on Linux: `cargo audit` and `cargo deny --locked check`
  (`deny.toml`: advisories, licences, bans, sources, for the macOS graph). Both run locally as
  they are.
- Dependencies: `.cargo/config.toml` resolves only versions published at least 7 days ago (Cargo
  1.100), CI builds with `--locked`, and Dependabot (`.github/dependabot.yml`) opens weekly,
  grouped updates with the same cooldown.
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
