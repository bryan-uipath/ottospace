# Install and update

How ottospace gets onto a Mac and stays current: `install.sh`, `scripts/osi`, and the workflows
that build what they install. Written 2026-11-12, with the repository newly public and no tag or
release yet.

## Decisions

| Question | Decision |
|---|---|
| What is installed by default | The bare `ottospace` binary in `~/.local/bin`. The `.app` only with `--app` |
| Where `main` builds come from | A rolling prerelease, tag `main-latest`, replaced on every push to `main` that changes more than docs |
| Where pull request builds come from | Workflow artifacts of `build.yml`, fetched with `gh` |
| Where releases come from | `release.yml` on a `v*` tag, same four files |
| Default channel of the one-liner | `main`, until there is a tagged release |
| State | `~/.ottospace/install/` |
| Auto-update | None. `osi update` on demand, `osi update --check` to ask |

## Why the bare binary is the default

ottospace needs Accessibility and Screen Recording. macOS (TCC) grants them to a code identity.

- **`.app`, ad hoc signed** (what `scripts/bundle.sh` makes): the identity is the hash of the
  code, so every build is a new program to macOS. After an update the old grant does not apply,
  and the stale entry often has to be removed in System Settings before the new one works.
  Installing `main` several times a day would mean granting both permissions several times a day.
- **Bare binary run from a terminal**: macOS charges the request to the terminal app. The
  terminal's grant is made once and covers every build. This is how `cargo run` already works.

So `osi install` puts the binary on `PATH` and says to run it from a terminal. `--app` exists for
a Dock icon, at the price above.

**What a Developer ID certificate would change.** With a Developer ID signature the identity is
the bundle id plus the team id, not the hash, so grants survive updates and the `.app` becomes
the right default. Notarized, a zip downloaded in a browser also opens without a Gatekeeper
refusal. It costs an Apple Developer Program membership, and the certificate and notary
credentials become CI secrets. Those must reach only the jobs that publish (`publish-main`,
`release`), never a pull request build; pull request builds would stay ad hoc.

## Where builds come from

`scripts/package.sh` makes the same four files for every channel:

| File | What |
|---|---|
| `ottospace-aarch64-apple-darwin.tar.gz` | The binary |
| `ottospace-aarch64-apple-darwin.app.zip` | `ottospace.app` |
| `BUILD` | `version=`, `commit=`, `date=` (the commit's date, UTC) |
| `SHA256SUMS` | Checksums of the three files above |

The names carry no version, so one URL pattern serves every channel, and
`releases/latest/download/<name>` works for `stable` without a call to the API.

### main: a rolling prerelease

`build.yml`, on a push to `main`: `build` makes the files, `publish-main` uploads them to the
release `main-latest`, creating it the first time. Release assets download with plain `curl`.

- It is a prerelease, so it is never "latest": `osi install stable` does not see it.
- The tag is `main-latest`, not `main` (a tag named like the branch makes `main` ambiguous) and
  not `v*` (so `release.yml` does not fire; a tag pushed with the workflow token would not start
  a workflow anyway).
- The tag is moved to the new commit, so the release page's source links are true. Cost: a clone
  that has the tag reports "would clobber existing tag" on `git fetch --tags`. A plain
  `git fetch` does not update tags it already has. Leaving the tag on its first commit forever was
  the alternative; it avoids the message and makes the page lie.
- The release is edited, not deleted and made again: watchers are notified once, not per push.
- Assets are replaced one at a time. An install in that window gets files from two builds; the
  checksum fails, osi changes nothing and says to try again.
- A run publishes only when its commit is ahead of the published one. A run that finishes late
  does not replace a newer build; one that a newer push overtook still publishes until the newer
  build lands.

### Pull requests: workflow artifacts

`build.yml`, on `pull_request`: the same files as one artifact, `ottospace-aarch64-apple-darwin`,
kept 14 days. `osi install pr <n>` asks `gh` for the pull request, finds the
`build.yml` run for its head commit, and downloads the artifact. A run that is queued or running
is waited for (state, jobs and time on one line; `--no-wait` stops instead, an hour is the limit);
a failed one is named with its link.

Pull requests are built on request (2026-11-13). Every push to every pull request started five
macOS jobs, and with several pull requests open the queue held back `main-latest`. `build.yml`
builds a pull request only while it carries the label `pr-build`; then every push builds again.
Without it the run's jobs are skipped and use no runner, and osi ignores such a run. Adding
another label builds nothing, and such a run cannot cancel a build (its concurrency group is its
own). `scripts.yml` still tests `scripts/test-osi.sh` on a pull request that changes the scripts.

- `osi install pr <n>` with no build: if the label is missing, osi adds it
  (`gh pr edit --add-label`) and says so, then waits as for a queued build. A run takes a moment
  to appear: "no build" while labelled is waited for up to two minutes (`OSI_START_MAX`), then osi
  gives up with the reasons (not started, docs only, older than 14 days).
- Adding a label needs write access. When it fails (a fork's author, say) osi stops and says to
  ask a maintainer; it does not retry.
- `--no-wait` still adds the label, says the build was requested, and exits non-zero.
- `osi list` shows an unlabelled pull request without a build as "no build (to build it: osi
  install pr <n>)": not a failure.
- The label's name is a constant in osi, never input.

- Artifacts need a signed-in `gh` even on a public repository. That is acceptable here: someone
  installing a pull request build has `gh`.
- A prerelease per pull request would allow `curl`, but needs a write token in a run that a pull
  request triggers, and leaves releases and tags to clean up. Rejected.
- The build is of the pull request's **head commit**, not of the merge with `main` (which is what
  `ci.yml` tests). So the commit osi shows, the commit in `BUILD` and the commit in
  `ottospace --version` are one, and osi refuses a build whose `BUILD` names another.

### Fork safety

A pull request build is that pull request's code, run with the terminal's permissions.

- Workflows use `pull_request`, never `pull_request_target`. A fork's run has a read-only token
  and no secrets, whatever its workflow file says. Every workflow sets `permissions:` itself;
  the only write token is in `publish-main` (pushes to `main` only, runs no repository code) and
  in `release`.
- Pull requests read the build cache of `main` and do not write one.
- `osi install pr <n>` prints the author, the head commit and the head repository first. If the
  head repository is not `bryan-uipath/ottospace` it refuses. `--allow-fork` turns that into a
  prompt to type the first 7 characters of the head commit; `osi update` on a fork's pull request
  prompts again, because a new head is new code.
- A pull request from a branch of this repository installs without a prompt: only people with
  write access can push there.

## osi

POSIX `sh`, at `scripts/osi`. Needs `curl`, `shasum`, `tar`; `gh` only for pull requests; `ditto`
and `codesign` only for `--app`. All ship with macOS except `gh`.

| Command | Does |
|---|---|
| `osi install main` / `stable` / `vX.Y.Z` / `pr <n>` | Installs that build. `--app` adds `~/Applications/ottospace.app` |
| `osi list` | The main build and each open pull request: commit, build state (ready, queued, running, failed, no build), which is installed |
| `osi current` | Channel, version, commit, date, when installed, the previous build |
| `osi update` | Installs the channel's build if its commit differs. `--check` only reports |
| `osi rollback` | Swaps the installed and the previous build. Twice: back where you were |
| `osi uninstall` | Removes the command, the app it installed, its state and itself. Keeps settings and tasks |
| `osi self-update` | Replaces itself with `scripts/osi` on `main` |

- **Verified before changed.** Every file is checked against `SHA256SUMS`; `BUILD` is parsed, never
  sourced. Any failure leaves the installed build as it was.
- **What the checksum is worth.** It comes from the same release as the file, so it catches a
  damaged or half-replaced download, not a compromised release. See open question 5.
- **Atomic.** The new binary is written beside the old one and renamed over it. A running
  ottospace keeps its old file. (Writing over the file in place gets a running, signed process
  killed on Apple Silicon.)
- **A way back.** One previous build is kept in `~/.ottospace/install/previous.bin` with its record.
- **State** is `~/.ottospace/install/current`, lines of `key=value`. It sits beside
  `settings.json` and `tasks/` and collides with neither. `OTTOSPACE_HOME` moves it, as it moves
  the rest.
- **It will not replace** an `ottospace` in `~/.local/bin`, or an `ottospace.app`, that it has no
  record of, without `--force`. A hand-built `scripts/bundle.sh` app is safe from `--app`.
- **The app has its own record**, `~/.ottospace/install/app` (path and commit), apart from
  `current`. So an install without `--app` leaves the app, osi still knows it owns it, `update`
  brings it level, and `uninstall` removes it. `previous.app` is kept only while it is the app of
  the previous build: `rollback` restores it only if its commit is the one rolled back to.
- **Staged, then swapped.** Binary and app are both staged before either moves. If a rename
  fails, the old app goes back and `current` still names what is installed.
- **gh is pinned to github.com**: every call passes `-R github.com/bryan-uipath/ottospace`, and
  `GH_HOST` and `GH_REPO` are dropped, so an enterprise host cannot stand in for the repository.
- **Overrides**, for tests: `HOME`, `OSI_BIN_DIR`, `OSI_APP_DIR`, `OTTOSPACE_HOME`,
  `OSI_RELEASES_URL` and `OSI_SELF_URL` (`file://` allowed), `OSI_GH`; `OSI_POLL`,
  `OSI_WAIT_MAX` and `OSI_START_MAX` (seconds) for the waits; `OSI_MAIN_URL` (main's tip) and
  `OSI_PS` (prints a pid's executable).

`install.sh` downloads `scripts/osi` from `main`, installs it as `~/.local/bin/osi`, and runs
`osi install main`. `OTTOSPACE_CHANNEL=stable` or `=v0.1.0` picks another build;
`OTTOSPACE_APP=1` adds the app. osi itself has no checksum: like `install.sh`, it is trusted
because it comes over HTTPS from this repository.

## A running ottospace

Replacing the file does not touch a running ottospace: the process keeps the old file until it
exits. osi never stops it. It hosts the user's real windows, and only its own quit path puts them
back; a quit also closes terminal panes and the agents in them, and osi could not start the app
again under the right terminal's permissions. After an install osi says, in one line, when the
app is running: "ottospace is running the old build: quit it and start it again."

How osi knows (2026-11-13). Matching processes by program path counted `ottospace --version`
and `ottospace ctl` as the app, and `ps` shows argv[0], not the path, for a program started from
`PATH`. Now the app says it is running: at start, outside a test run, it writes
`<home>/running` (0600, whole): its pid and its executable's resolved path, one per line. A clean
quit removes it if it is still its own; a second instance writes over it. osi reads it (a regular
file of 4 KiB at most) and counts the app as running when the pid is alive, its executable
(`lsof -d txt`) is the path in the file, and that path is the command or the app osi installed.
A stale file (pid gone, or reused by another program) does not count. No socket connection, no
argument parsing.

An install prints at most three lines: the build (`Installed ottospace 0.1.0 (05e8136) from
main`), "main is ahead of this build (<tip>); a newer build is not ready yet." when `main` has
moved past the installed `main` build (one unauthenticated `commits/main` call; silent when it
fails), and the running line. The permissions note shows on a first install only; it and
`osi rollback` are in `osi help`. A docs-only commit on `main` also shows as ahead.

**Proposed, not built: a notice in the app.** ottospace knows its own commit
(`OTTOSPACE_COMMIT`). If the shell read `commit=` from `~/.ottospace/install/current` now and
then and passed it to the core in an `Action`, the status bar could say "a newer build is
installed: restart to use it". The decision belongs in `ottospace-core` with a test; the shell
only reads the file. A restart that keeps terminal sessions needs session restore, which is not
built.

## Auto-update

The smallest honest version is what is here: `osi update` when you want it, and
`osi update --check` to ask. No daemon, no launch agent, nothing runs unasked.

A real auto-updater would need:

1. A Developer ID identity, or every update costs both permissions (for the `.app`).
2. Updates signed with a key that is not the transport (Sparkle's EdDSA feed is the usual way),
   so a compromised release cannot push code.
3. A check from inside the app, with a setting to turn it off.
4. A restart that puts hosted windows back and reopens the workspace: session restore.

## `ottospace --version`

Prints `ottospace 0.1.0 (a4a2582c0ffe, 2026-11-12T05:33:48Z)`: version, commit, the commit's
date. `scripts/package.sh` passes `OTTOSPACE_COMMIT` and `OTTOSPACE_BUILD_DATE` to the compiler
(`option_env!`), and fails the build if the binary does not report the commit. A plain
`cargo build` prints `ottospace 0.1.0 (local build)`. The date is the commit's, not the clock's,
so one commit always gives one line.

## Verified

- The repository is public; it has no tag and no release (`gh repo view`, `gh release list`, `git tag`).
- The repository's default workflow token is read-only; fork pull requests from first-time
  contributors wait for approval; no rulesets; immutable releases are off (`gh api`).
- `gh pr view --json` returns the fields osi reads, checked against this repository's pull requests.
- `scripts/test-osi.sh`: install through `install.sh`, current, update, rollback, checksum
  mismatch, changed `BUILD`, pull request install and update, a build of the wrong commit, fork
  refusal and `--allow-fork`, releases, `--app`, self-update, uninstall, a foreign `ottospace`,
  overrides. Fixture builds, a fake `gh`, a temp `HOME`.
- `shellcheck` is clean on the four scripts.
- `ottospace --version`, with and without the two variables, in a debug build; a unit test.

## Not verified

These cannot be checked until the workflows have run on GitHub:

- `build.yml` at all, and `scripts/package.sh` end to end: no release build was made locally.
- The checkout of a pull request's head commit, for a fork in particular. No fork pull request exists.
- `publish-main`: it first runs after this is merged. Creating the release, moving the tag with
  the workflow token, replacing assets.
- `osi install main` and `osi install pr <n>` against GitHub: the real URLs, the real `gh`
  filters (`gh run list --commit --event --status`), the layout `gh run download` leaves.
- `release.yml`, which has still never run, and `osi install stable` with it.

Not tested here, and stated from how macOS is known to behave:

- That an ad hoc `.app` loses its grants on update, and that a bare binary uses the terminal's.
  The second matches how `cargo run` and `cargo e2e-apps` already work in this repository.
- That replacing by rename leaves a running ottospace unharmed. The app was not started.

## Open questions

1. Switch the one-liner's default to `stable` once `v0.1.0` is tagged? `main` is right for the
   owner; a stranger may want a release.
2. Buy a Developer ID? It decides whether the `.app` can ever be the default.
3. Is a moving `main-latest` tag acceptable, given the `git fetch --tags` message?
4. Should `build` become a required check? It is a second release-mode compile per push.
5. Add build provenance (`actions/attest-build-provenance`, checked with `gh attestation verify`)?
   It would tie a download to the workflow run that made it, at the price of needing `gh` for
   every install.
6. Is 14 days right for pull request artifacts?
7. `cargo install` puts another `ottospace` in `~/.cargo/bin`, which may come first on `PATH`.
   Should osi warn when `command -v ottospace` is not the one it installed?
8. Build the restart notice in the app?
