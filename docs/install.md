# Install and update

Short version: [README](../README.md#install).

Apple Silicon, macOS 14 or later:

```sh
curl -fsSL https://raw.githubusercontent.com/bryan-uipath/ottospace/main/install.sh | sh
```

This installs two commands into `~/.local/bin`, with no `sudo`:

- `osi`, the ottospace installer.
- `ottospace`, the latest build of `main`. There is no tagged release yet.

Then run `ottospace` from a terminal. It uses that terminal's permissions: grant the terminal
Accessibility and Screen Recording once (see [Permissions](reference.md#requirements-and-permissions)), and updates never ask again.

`osi` installs other builds and switches between them:

```sh
osi install main      # the latest build of main
osi install pr 30     # the build of a pull request. Needs gh, signed in. Without a build it
                      # adds the label pr-build, which asks for one. Waits for a build that is
                      # requested, queued or running (--no-wait: say its state and stop)
osi list              # builds you can install: main, and each open pull request with its state
osi install stable    # the latest release; osi install v0.1.0 for one release
osi current           # what is installed: channel, version, commit, date
osi update            # install the channel's newer build, if there is one (waits like install)
osi update --check    # only say whether there is one
osi rollback          # put the previous build back; again to undo
osi uninstall         # remove ottospace and osi; settings and tasks are kept
osi self-update       # replace osi with the one on main
```

- Every download is checked against the SHA-256 published in the same release. A failed check
  changes nothing. The check catches a damaged or half-published download. It does not catch a
  compromised release: whoever can replace the build can replace its checksum.
- `install.sh` and `osi self-update` fetch `osi` from `main` over HTTPS with no checksum at all.
  You trust this repository and GitHub, as with any `curl | sh`.
- The new binary replaces the old one by rename. A running ottospace keeps the build it started
  with: quit it (`ottospace ctl '{"cmd":"quit"}'` puts hosted windows back; terminal panes and
  what runs in them close) and start it again. osi says so when the app is running: the app
  records its pid and program in `~/.ottospace/running`. `ottospace ctl` and other subcommands
  do not count.
- After installing `main`, osi says when `main` has a newer commit than the build: its build is
  not published yet, or it changed only docs (which publish none).
- Undo an install: `osi rollback`.
- **A pull request build runs that pull request's code with your terminal's permissions.** `osi`
  shows the author and the head commit first. It refuses a pull request from a fork unless you
  pass `--allow-fork` and type the commit. Read the diff before you do.
- `ottospace --version` prints the version, the commit and the commit's date.
- `osi` keeps its records in `~/.ottospace/install`.

To also get `ottospace.app` in `~/Applications`, add `--app` (with the one-liner:
`... | OTTOSPACE_APP=1 sh`). The app has a Dock icon and its own identity, so macOS asks for both
permissions for ottospace itself. Builds are signed ad hoc, not with a Developer ID, and not
notarized: each build is a new identity, so expect to grant both again after every update. That
is why the bare command is the default. A zip downloaded in a browser gets a quarantine flag and
Gatekeeper refuses to open the app; `osi` uses `curl`, which sets no such flag. `osi` will not
replace an `ottospace.app` it did not install (one from `scripts/bundle.sh`, say) without
`--force`. Once it has installed the app, `osi update` keeps it at the same build as the command.

How this works and why: [docs/ai/install-and-update.md](ai/install-and-update.md).
