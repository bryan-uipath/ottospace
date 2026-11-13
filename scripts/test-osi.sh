#!/usr/bin/env bash
# Tests install.sh and scripts/osi against fixture builds in a temp directory: no network, no
# real gh, and HOME points into the temp directory, so the real install is never touched.
#   scripts/test-osi.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
osi="$root/scripts/osi"
target="aarch64-apple-darwin"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Nothing from the caller's environment may point osi at a real directory.
unset OTTOSPACE_HOME OSI_BIN_DIR OSI_APP_DIR OTTOSPACE_CHANNEL OTTOSPACE_VERSION OTTOSPACE_APP
export HOME="$work/home"
export OSI_RELEASES_URL="file://$work/releases"
export OSI_SELF_URL="file://$osi"
export OSI_GH="$work/fake-gh"
# main's tip, as GitHub would say it; no file: the lookup fails.
export OSI_MAIN_URL="file://$work/main-tip"
export FAKE_GH_DIR="$work/gh"
mkdir -p "$HOME" "$FAKE_GH_DIR"

bin="$HOME/.local/bin"
state="$HOME/.ottospace/install"
app="$HOME/Applications/ottospace.app"

passed=0
failed=0
out=""
status=0

ok() {
  passed=$((passed + 1))
  echo "ok    $1"
}
fail() {
  failed=$((failed + 1))
  echo "FAIL  $1"
  if [ -n "$out" ]; then printf '%s\n' "$out" | sed 's/^/      | /'; fi
}
# check <name> <command...>: passes when the command succeeds.
check() {
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then ok "$name"; else fail "$name"; fi
}
# run <command...>: keeps the output in $out and the exit status in $status.
run() {
  status=0
  out="$("$@" 2>&1)" || status=$?
}
says() { printf '%s' "$out" | grep -qF -- "$1"; }
installed_is() { [ "$("$bin/ottospace")" = "fake ottospace $1" ]; }
state_has() { grep -qxF -- "$1" "$state/current"; }
app_is() { [ "$(cat "$app/Contents/Resources/id")" = "$1" ]; }

# A 40-character commit made of one letter, e.g. `commit a`.
commit() { printf "%040d" 0 | tr 0 "$1"; }

# make_build <dir> <id> <commit>: the files scripts/package.sh would publish, with a fake binary
# that prints its id and an ad hoc signed fake app.
make_build() {
  local dir="$1" id="$2" sha="$3" stage="$work/stage"
  rm -rf "$stage" "$dir"
  mkdir -p "$stage/ottospace.app/Contents/MacOS" "$stage/ottospace.app/Contents/Resources" "$dir"
  printf '#!/bin/sh\necho "fake ottospace %s"\n' "$id" >"$stage/ottospace"
  chmod 755 "$stage/ottospace"
  tar -czf "$dir/ottospace-$target.tar.gz" -C "$stage" ottospace
  # A real Mach-O, so the signature check in osi has something to verify.
  cp /usr/bin/true "$stage/ottospace.app/Contents/MacOS/ottospace"
  printf '%s' "$id" >"$stage/ottospace.app/Contents/Resources/id"
  codesign --force --sign - "$stage/ottospace.app" 2>/dev/null
  ditto -c -k --keepParent "$stage/ottospace.app" "$dir/ottospace-$target.app.zip"
  printf 'version=0.1.0\ncommit=%s\ndate=2026-11-12T08:09:00Z\ntarget=%s\n' "$sha" "$target" >"$dir/BUILD"
  resum "$dir"
}

# resum <dir>: writes SHA256SUMS for the files as they are now.
resum() {
  (cd "$1" && shasum -a 256 "ottospace-$target.tar.gz" "ottospace-$target.app.zip" BUILD >SHA256SUMS)
}

# make_pr <number> <head commit> <head repo> <is fork: true|false> <author>: what fake gh answers.
make_pr() {
  printf 'OPEN\t%s\t%s\t%s\t%s\n' "$5" "$2" "$3" "$4" >"$FAKE_GH_DIR/pr-$1"
}

# Answers the gh calls osi makes, from files in $FAKE_GH_DIR. A pull request carries the label
# pr-build when label-<n> exists; `pr edit --add-label` makes it (fails if edit-fails exists).
cat >"$OSI_GH" <<'EOF'
#!/bin/sh
printf '%s|%s|%s\n' "${GH_HOST:-}" "${GH_REPO:-}" "$*" >>"$FAKE_GH_DIR/log"
case "$1 $2" in
  "pr view")
    if [ -e "$FAKE_GH_DIR/label-$3" ]; then labelled=true; else labelled=false; fi
    printf '%s\t%s\n' "$(cat "$FAKE_GH_DIR/pr-$3")" "$labelled"
    ;;
  "pr edit")
    [ ! -e "$FAKE_GH_DIR/edit-fails" ] || exit 1
    n="$3"
    while [ $# -gt 0 ]; do
      if [ "$1" = --add-label ]; then echo "$n $2" >>"$FAKE_GH_DIR/edits"; fi
      shift
    done
    touch "$FAKE_GH_DIR/label-$n"
    ;;
  "run list")
    while [ $# -gt 0 ]; do
      if [ "$1" = --commit ]; then sha="$2"; fi
      if [ "$1" = --status ]; then only="$2"; fi
      shift
    done
    if [ "${only:-}" = success ] && [ -s "$FAKE_GH_DIR/run-$sha" ]; then
      case "$(head -n 1 "$FAKE_GH_DIR/run-$sha")" in
        *completed*success*) head -n 1 "$FAKE_GH_DIR/run-$sha" ;;
      esac
      exit 0
    fi
    # run-<sha> holds one answer for each call (the last one repeats); else the build decides.
    if [ -s "$FAKE_GH_DIR/run-$sha" ]; then
      head -n 1 "$FAKE_GH_DIR/run-$sha"
      if [ "$(wc -l <"$FAKE_GH_DIR/run-$sha")" -gt 1 ]; then
        tail -n +2 "$FAKE_GH_DIR/run-$sha" >"$FAKE_GH_DIR/run-$sha.next" && mv "$FAKE_GH_DIR/run-$sha.next" "$FAKE_GH_DIR/run-$sha"
      fi
    elif [ -d "$FAKE_GH_DIR/build-$sha" ]; then
      printf '4242\tcompleted\tsuccess\n'
    fi
    ;;
  "run view") echo "scripts success, build in_progress" ;;
  "pr list") cat "$FAKE_GH_DIR/prs" 2>/dev/null ;;
  "run download")
    while [ $# -gt 0 ]; do
      if [ "$1" = -D ]; then dir="$2"; fi
      shift
    done
    cp "$FAKE_GH_DIR"/build-current/* "$dir"
    ;;
  *) exit 1 ;;
esac
EOF
chmod 755 "$OSI_GH"

# publish_pr <head commit> <id> [<built commit>]: the artifact of the run for that head.
publish_pr() {
  make_build "$FAKE_GH_DIR/build-$1" "$2" "${3:-$1}"
  rm -f "$FAKE_GH_DIR/build-current"
  ln -s "$FAKE_GH_DIR/build-$1" "$FAKE_GH_DIR/build-current"
}

main_dir="$work/releases/download/main-latest"

if command -v shellcheck >/dev/null 2>&1; then
  check "shellcheck" shellcheck "$root/install.sh" "$osi" "$root/scripts/package.sh" "$root/scripts/test-osi.sh"
else
  echo "skip  shellcheck (not installed)"
fi

echo "# install.sh: installs osi, then the main build"
make_build "$main_dir" A "$(commit a)"
run sh "$root/install.sh"
check "install.sh exits 0" [ "$status" = 0 ]
check "osi is installed" [ -x "$bin/osi" ]
check "the main build is installed" installed_is A
check "state records the channel" state_has "channel=main"
check "state records the commit" state_has "commit=$(commit a)"
check "says it uses the terminal's permissions" says "terminal's Accessibility and Screen Recording"
check "names the build in one line" says "Installed ottospace 0.1.0 (aaaaaaa) from main"
not_says() { ! says "$1"; }
check "no download URL" not_says "file://"
check "no command path when the bin directory is on PATH" eval "! PATH='$bin:$PATH' '$bin/osi' install main --force 2>&1 | grep -q '$bin'"
check "no app without --app" [ ! -e "$app" ]
# From here on, the installed copy.
osi="$bin/osi"

echo "# current"
run "$osi" current
check "current names channel, version and commit" says "ottospace main  0.1.0  commit aaaaaaaaaaaa  2026-11-12T08:09:00Z"

echo "# update"
run "$osi" update
check "nothing newer: says up to date" says "Up to date"
make_build "$main_dir" B "$(commit b)"
run "$osi" update --check
check "update --check reports the newer build" says "main is at bbbbbbbbbbbb; installed is aaaaaaaaaaaa"
check "update --check installs nothing" installed_is A
run "$osi" update
check "update installs the newer build" installed_is B
check "an update does not repeat the permissions note" not_says "Accessibility"
check "main's tip unknown: nothing about main" not_says "main is ahead"
commit b | tr -d '\n' >"$work/main-tip"
run "$osi" update
check "the build is main's tip: nothing about main" not_says "main is ahead"
commit c | tr -d '\n' >"$work/main-tip"
run "$osi" update
check "main is ahead of an up-to-date build: said" says "main is ahead of this build (ccccccc); a newer build is not ready yet."
rm "$work/main-tip"
check "update keeps the previous binary" [ "$("$state/previous.bin")" = "fake ottospace A" ]

echo "# rollback"
run "$osi" rollback
check "rollback puts the previous build back" installed_is A
check "rollback updates the state" state_has "commit=$(commit a)"
run "$osi" rollback
check "a second rollback undoes the first" installed_is B
run "$osi" current
check "current shows the previous build" says "previous:  main  0.1.0  commit aaaaaaaaaaaa"

echo "# checksum mismatch"
make_build "$main_dir" C "$(commit c)"
echo tampered >>"$main_dir/ottospace-$target.tar.gz"
run "$osi" update
check "a bad checksum fails" [ "$status" != 0 ]
check "it says so" says "checksum mismatch"
check "the old binary is still installed" installed_is B
check "the state is unchanged" state_has "commit=$(commit b)"
make_build "$main_dir" C "$(commit c)"
sed -i '' 's/^commit=.*/commit=dddddddddddddddddddddddddddddddddddddddd/' "$main_dir/BUILD"
run "$osi" install main
check "a changed BUILD file fails" [ "$status" != 0 ]
check "the old binary is still installed" installed_is B
make_build "$main_dir" B "$(commit b)"

echo "# pull request from this repository"
make_pr 30 "$(commit e)" bryan-uipath/ottospace false bryan-uipath
publish_pr "$(commit e)" PR30
run "$osi" install pr 30
check "installs the pull request build" installed_is PR30
check "shows the author" says "Pull request #30 (OPEN) by bryan-uipath"
check "shows the head commit" says "head: $(commit e)"
check "state records the pull request" state_has "ref=30"
run "$osi" update
check "pull request unchanged: up to date" says "Up to date"
make_pr 30 "$(commit f)" bryan-uipath/ottospace false bryan-uipath
touch "$FAKE_GH_DIR/label-30"
run env OSI_START_MAX=0 "$osi" update
check "new head without a build fails" says "no build for commit ffffffffffff"
check "the old binary is still installed" installed_is PR30
publish_pr "$(commit f)" PR30b
run "$osi" update
check "update follows the pull request's head" installed_is PR30b
make_pr 31 "$(commit 1)" bryan-uipath/ottospace false bryan-uipath
publish_pr "$(commit 1)" WRONG "$(commit 2)"
run "$osi" install pr 31
check "a build of another commit is refused" says "not the pull request's head"
check "the old binary is still installed" installed_is PR30b

echo "# pull request from a fork"
make_pr 32 "$(commit 3)" mallory/ottospace true mallory
publish_pr "$(commit 3)" FORK
run "$osi" install pr 32
check "a fork is refused" [ "$status" != 0 ]
check "it warns" says "WARNING: this pull request comes from a fork"
check "it shows the author" says "by mallory"
check "nothing was installed" installed_is PR30b
run "$osi" install pr 32 --allow-fork </dev/null
check "--allow-fork without the typed commit is refused" [ "$status" != 0 ]
check "nothing was installed" installed_is PR30b
out="$(echo nope | "$osi" install pr 32 --allow-fork 2>&1)" || true
check "--allow-fork with the wrong commit is refused" installed_is PR30b
out="$(echo 3333333 | "$osi" install pr 32 --allow-fork 2>&1)" || true
check "--allow-fork with the typed commit installs" installed_is FORK
# Same repository name but the fork flag set, and the reverse: both count as a fork.
make_pr 33 "$(commit 3)" bryan-uipath/ottospace true mallory
run "$osi" install pr 33
check "isCrossRepository alone marks a fork" says "refusing to install a build from a fork"
make_pr 34 "$(commit 3)" mallory/ottospace false mallory
run "$osi" install pr 34
check "another head repository alone marks a fork" says "refusing to install a build from a fork"
run "$osi" rollback
check "rollback leaves the fork build" installed_is PR30b

echo "# releases"
make_build "$work/releases/download/v0.1.0" V1 "$(commit 4)"
make_build "$work/releases/latest/download" V2 "$(commit 5)"
run "$osi" install v0.1.0
check "installs a release by tag" installed_is V1
run "$osi" update
check "a pinned release does not update" says "Pinned to v0.1.0"
run "$osi" install stable
check "installs the latest release" installed_is V2
run "$osi" install v9.9.9
check "a missing release fails" [ "$status" != 0 ]
check "the old binary is still installed" installed_is V2
run "$osi" install 'v1;rm'
check "a bad version is refused" says "bad version"

echo "# --app"
# The app cannot be staged: nothing may change, the binary included.
mkdir -p "$HOME/Applications"
chmod 555 "$HOME/Applications"
run "$osi" install main --app
chmod 755 "$HOME/Applications"
check "an app that cannot be staged fails" [ "$status" != 0 ]
check "the old binary is still installed" installed_is V2
check "the state is unchanged" state_has "commit=$(commit 5)"
run "$osi" install main --app
check "installs the app" app_is B
check "the app's signature verifies" codesign --verify --deep --strict "$app"
check "osi records the app" grep -qxF "path=$app" "$state/app"
make_build "$main_dir" G "$(commit 6)"
run "$osi" update
check "update also updates the app" app_is G
run "$osi" rollback
check "rollback also puts the app back" app_is B
check "and the binary" installed_is B
run "$osi" rollback
check "a second rollback brings the app forward" app_is G
check "and the binary" installed_is G

echo "# --app, then an install without it"
make_build "$main_dir" H "$(commit 7)"
run "$osi" install main --app
make_build "$main_dir" I "$(commit 8)"
run "$osi" install main
check "the app is left as it is" app_is H
check "osi says which build the app is" says "is build 777777777777 and was left"
run "$osi" rollback
check "rollback puts the binary back" installed_is H
check "the app is that build, not an older one" app_is H
check "no stale app is kept" [ ! -e "$state/previous.app" ]
run "$osi" install main
check "the binary moves on without the app" installed_is I

echo "# bad downloads"
make_build "$main_dir" J "$(commit 9)"
echo tampered >>"$main_dir/ottospace-$target.app.zip"
run "$osi" install main --app
check "a bad app checksum fails" says "checksum mismatch for ottospace-$target.app.zip"
check "the old binary is still installed" installed_is I
check "the old app is still installed" app_is H
make_build "$main_dir" J "$(commit 9)"
sed -i '' '/tar\.gz/d' "$main_dir/SHA256SUMS"
run "$osi" install main
check "a file missing from SHA256SUMS fails" says "SHA256SUMS has no checksum for ottospace-$target.tar.gz"
check "the old binary is still installed" installed_is I
# A hostile archive: `ottospace` is a symlink, and one entry points three directories up,
# which from osi's unpack directory is $work.
make_build "$main_dir" J "$(commit 9)"
mkdir -p "$work/evil/a/b/c" "$work/tmp"
ln -s /etc/hosts "$work/evil/a/b/c/ottospace"
echo escaped >"$work/evil/escape-marker"
tar -czf "$main_dir/ottospace-$target.tar.gz" -C "$work/evil/a/b/c" ottospace ../../../escape-marker
resum "$main_dir"
check "the fixture archive holds the escaping entry" eval "tar -tzf '$main_dir/ottospace-$target.tar.gz' | grep -qxF ../../../escape-marker"
run env TMPDIR="$work/tmp" "$osi" install main
check "a symlink in place of the binary fails" [ "$status" != 0 ]
check "the old binary is still installed" installed_is I
check "the command is not a symlink" [ ! -L "$bin/ottospace" ]
check "nothing is written outside the unpack directory" [ ! -e "$work/escape-marker" ]
check "the temp directory is cleaned up" [ -z "$(ls -A "$work/tmp")" ]
# The same escaping entry beside a real binary: the binary installs, the entry is ignored.
rm "$work/evil/a/b/c/ottospace"
printf '#!/bin/sh\necho "fake ottospace J"\n' >"$work/evil/a/b/c/ottospace"
chmod 755 "$work/evil/a/b/c/ottospace"
tar -czf "$main_dir/ottospace-$target.tar.gz" -C "$work/evil/a/b/c" ottospace ../../../escape-marker
resum "$main_dir"
run env TMPDIR="$work/tmp" "$osi" install main
check "only the binary is unpacked" installed_is J
check "the escaping entry is ignored" [ ! -e "$work/escape-marker" ]

echo "# self-update"
run "$osi" self-update
check "self-update exits 0" [ "$status" = 0 ]
printf '#!/bin/sh\necho not osi\n' >"$work/not-osi"
run env OSI_SELF_URL="file://$work/not-osi" "$osi" self-update
check "self-update refuses a file that is not osi" [ "$status" != 0 ]
check "osi still works" "$osi" current

echo "# uninstall"
echo '{}' >"$HOME/.ottospace/settings.json"
mkdir -p "$HOME/.ottospace/tasks"
run "$osi" uninstall
check "removes the command" [ ! -e "$bin/ottospace" ]
check "removes osi" [ ! -e "$bin/osi" ]
check "removes the app it installed, though the last install had no --app" [ ! -e "$app" ]
check "removes its state" [ ! -e "$state" ]
check "keeps settings.json" [ -f "$HOME/.ottospace/settings.json" ]
check "keeps tasks" [ -d "$HOME/.ottospace/tasks" ]

echo "# a command osi did not install"
osi="$root/scripts/osi"
mkdir -p "$bin"
printf '#!/bin/sh\necho mine\n' >"$bin/ottospace"
chmod 755 "$bin/ottospace"
run "$osi" install main
check "is not replaced" [ "$("$bin/ottospace")" = mine ]
check "osi says why" says "osi did not install it"
make_build "$main_dir" K "$(commit a)"
run "$osi" install main --force
check "--force replaces it" installed_is K
printf '#!/bin/sh\necho mine\n' >"$bin/osi"
run sh "$root/install.sh"
check "install.sh does not replace another osi" [ "$(sh "$bin/osi")" = mine ]
rm "$bin/osi"

echo "# an app osi did not install"
mkdir -p "$app/Contents/Resources"
printf mine >"$app/Contents/Resources/id"
make_build "$main_dir" L "$(commit b)"
run "$osi" install main --app
check "is not replaced" app_is mine
check "osi says why" says "ottospace.app exists and osi did not install it"
check "the binary is not updated either" installed_is K
run "$osi" update
check "update leaves it alone" app_is mine
check "and updates the command" installed_is L
run "$osi" uninstall
check "uninstall leaves it alone" app_is mine
make_build "$main_dir" M "$(commit c)"
run "$osi" install main --app --force
check "--force replaces it" app_is M
run "$osi" uninstall
check "then it is osi's to remove" [ ! -e "$app" ]

echo "# rollback with the command missing"
run "$osi" install main
make_build "$main_dir" N "$(commit d)"
run "$osi" update
rm "$bin/ottospace"
run "$osi" rollback
check "puts the previous build back" installed_is M
check "keeps no record of a build it has no copy of" [ ! -e "$state/previous" ]
check "nor a binary for it" [ ! -e "$state/previous.bin" ]
run "$osi" rollback
check "a second rollback has nothing to go back to" says "no previous build"

echo "# gh cannot be pointed at another host"
publish_pr "$(commit f)" PR30c
: >"$FAKE_GH_DIR/log"
run env GH_HOST=evil.example GH_REPO=evil/ottospace "$osi" install pr 30
check "the pull request build installs" installed_is PR30c
check "gh was called three times" [ "$(wc -l <"$FAKE_GH_DIR/log" | tr -d ' ')" = 3 ]
check "every call names github.com" [ "$(grep -c -- '-R github.com/bryan-uipath/ottospace' "$FAKE_GH_DIR/log")" = 3 ]
check "GH_HOST and GH_REPO do not reach gh" eval "! grep -q evil '$FAKE_GH_DIR/log'"

echo "# a build that is not finished"
make_pr 35 "$(commit 5)" bryan-uipath/ottospace false bryan-uipath
publish_pr "$(commit 5)" PR35
printf '77\tqueued\tnone\n77\tin_progress\tnone\n77\tcompleted\tsuccess\n' >"$FAKE_GH_DIR/run-$(commit 5)"
run env OSI_POLL=0 "$osi" install pr 35
check "a queued build is waited for and installed" installed_is PR35
check "it says the build was queued" says "Build queued; waiting"
check "then that it is running" says "Build running; waiting"
check "then that it finished" says "Build finished after"
printf '77\tin_progress\tnone\n' >"$FAKE_GH_DIR/run-$(commit 5)"
run "$osi" install pr 35 --no-wait
check "--no-wait stops at a running build" [ "$status" != 0 ]
check "and says its state with the run" says "is running: https://github.com/bryan-uipath/ottospace/actions/runs/77"
run env OSI_POLL=0 OSI_WAIT_MAX=2 "$osi" install pr 35
check "a build that never ends gives up" says "the build is still running after"
make_pr 36 "$(commit 6)" bryan-uipath/ottospace false bryan-uipath
printf '78\tcompleted\tfailure\n' >"$FAKE_GH_DIR/run-$(commit 6)"
run "$osi" install pr 36
printf '79\tcompleted\tcancelled\n' >"$FAKE_GH_DIR/run-$(commit 6)"
run "$osi" install pr 36
check "a cancelled build says a newer push replaces it" says "was cancelled (a newer push replaces it"
printf '78\tcompleted\tfailure\n' >"$FAKE_GH_DIR/run-$(commit 6)"
run "$osi" install pr 36
check "a failed build is named with its run" says "failed (failure): https://github.com/bryan-uipath/ottospace/actions/runs/78"
check "the old binary is still installed" installed_is PR35
rm -f "$FAKE_GH_DIR/run-$(commit 5)"

echo "# a pull request without the label pr-build"
make_pr 37 "$(commit 0)" bryan-uipath/ottospace false bryan-uipath
publish_pr "$(commit 0)" PR37
# The run appears two polls after the label.
printf '\n\n80\tqueued\tnone\n80\tcompleted\tsuccess\n' >"$FAKE_GH_DIR/run-$(commit 0)"
run env OSI_POLL=0 "$osi" install pr 37
check "adds the label" [ "$(cat "$FAKE_GH_DIR/edits")" = "37 pr-build" ]
check "says it asked for a build" says "Asked GitHub to build pull request 37 (label pr-build)"
check "waits for the run to appear" says "Build requested; waiting"
check "then for the build" says "Build queued; waiting"
check "and installs it" installed_is PR37
make_pr 37 "$(commit 8)" bryan-uipath/ottospace false bryan-uipath
publish_pr "$(commit 8)" PR37b
printf '\n81\tcompleted\tsuccess\n' >"$FAKE_GH_DIR/run-$(commit 8)"
run env OSI_POLL=0 "$osi" update
check "already labelled: the label is not added again" [ "$(wc -l <"$FAKE_GH_DIR/edits" | tr -d ' ')" = 1 ]
check "waits for the run of the new head and installs it" installed_is PR37b
make_pr 38 "$(commit 9)" bryan-uipath/ottospace false bryan-uipath
touch "$FAKE_GH_DIR/edit-fails"
: >"$FAKE_GH_DIR/log"
run env OSI_POLL=0 "$osi" install pr 38
rm "$FAKE_GH_DIR/edit-fails"
check "a label that cannot be added fails" [ "$status" != 0 ]
check "and says to ask a maintainer" says "ask a maintainer to add it"
check "it tries once" [ "$(grep -c '|pr edit 38' "$FAKE_GH_DIR/log")" = 1 ]
check "nothing was installed" installed_is PR37b
make_pr 39 "$(commit 9)" bryan-uipath/ottospace false bryan-uipath
run "$osi" install pr 39 --no-wait
check "--no-wait adds the label" [ -e "$FAKE_GH_DIR/label-39" ]
check "and says the build was requested" says "is requested: run this again in a few minutes"
check "and exits non-zero" [ "$status" != 0 ]
run "$osi" install pr 39 --no-wait
check "--no-wait, labelled, no run yet: says there is no build" says "no build for commit 999999999999"
# Back to pull request 35, which the list below marks as installed.
publish_pr "$(commit 5)" PR35
run "$osi" install pr 35

echo "# list"
printf '35\t%s\tfalse\ttrue\tA title\twith a tab\n36\t%s\tfalse\ttrue\tFails\n32\t%s\ttrue\ttrue\tFrom a fork\n41\t%s\tfalse\tfalse\tNot asked\n' \
  "$(commit 5)" "$(commit 6)" "$(commit 3)" "$(commit b)" >"$FAKE_GH_DIR/prs"
run "$osi" list
check "list shows main" says "osi install main"
check "list marks the installed build" says "pr 35     5555555  ready  (installed)"
check "list shows a failed build" says "pr 36     6666666  failed (failure)"
check "list marks a fork" says "ready, from a fork"
check "list shows titles" says "From a fork"
check "list says how to build an unlabelled pull request" says "pr 41     bbbbbbb  no build (to build it: osi install pr 41)"
printf '40\t%s\tfalse\ttrue\tBad \033[31m\302\233title \342\200\256x\n' "$(commit 6)" >"$FAKE_GH_DIR/prs"
out="$("$osi" list 2>&1)" || true
only_plain() { ! printf '%s' "$out" | LC_ALL=C grep -q '[^[:print:][:space:]]'; }
check "a title cannot send escape, C1 or bidi bytes to the terminal" only_plain
make_pr 35 "$(commit 7)" bryan-uipath/ottospace false bryan-uipath
printf '77\tin_progress\tnone\n' >"$FAKE_GH_DIR/run-$(commit 7)"
run "$osi" update --no-wait
check "update takes --no-wait" says "is running: https://github.com/bryan-uipath/ottospace/actions/runs/77"
check "and installs nothing" installed_is PR35

echo "# a running ottospace"
# The app writes `running` (pid, program); osi checks the pid lives and is that program. A live
# process stands in for the app; the fake OSI_PS says which program it is.
sleep 60 &
alive=$!
printf '#!/bin/sh\necho %s\n' "$bin/ottospace" >"$work/ps-ours"
printf '#!/bin/sh\necho /bin/sleep\n' >"$work/ps-other"
chmod 755 "$work/ps-ours" "$work/ps-other"
running_line="ottospace is running the old build: quit it and start it again."
rm -f "$HOME/.ottospace/running"
run env OSI_PS="$work/ps-ours" "$osi" install main --force
check "no file (nothing running, or only \`ottospace ctl\`): not said" not_says "is running"
printf '%s\n%s\n' "$alive" "$bin/ottospace" >"$HOME/.ottospace/running"
run env OSI_PS="$work/ps-ours" "$osi" install main --force
check "the installed app running: said in one line" says "$running_line"
run env OSI_PS="$work/ps-other" "$osi" install main --force
check "its pid now another program's: not said" not_says "is running"
run "$osi" install main --force
check "the real lister sees sleep, not ottospace: not said" not_says "is running"
printf '%s\n%s\n' "$alive" /somewhere/target/debug/ottospace >"$HOME/.ottospace/running"
run env OSI_PS="$work/ps-ours" "$osi" install main --force
check "an ottospace osi did not install: not said" not_says "is running"
kill "$alive"
wait "$alive" 2>/dev/null || true
printf '%s\n%s\n' "$alive" "$bin/ottospace" >"$HOME/.ottospace/running"
run env OSI_PS="$work/ps-ours" "$osi" install main --force
check "a stale file (pid gone): not said" not_says "is running"
rm "$HOME/.ottospace/running"

echo "# overrides"
run env OSI_BIN_DIR="$work/elsewhere/bin" OTTOSPACE_HOME="$work/elsewhere/home" "$osi" install main
check "OSI_BIN_DIR moves the command" [ -x "$work/elsewhere/bin/ottospace" ]
check "OTTOSPACE_HOME moves the state" [ -f "$work/elsewhere/home/install/current" ]
run env OSI_RELEASES_URL="http://example.invalid" "$osi" install main
check "plain http is refused" says "refusing to download"

echo
echo "$passed passed, $failed failed"
[ "$failed" = 0 ]
