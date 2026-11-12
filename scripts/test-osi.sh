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
  (cd "$dir" && shasum -a 256 "ottospace-$target.tar.gz" "ottospace-$target.app.zip" BUILD >SHA256SUMS)
}

# make_pr <number> <head commit> <head repo> <is fork: true|false> <author>: what fake gh answers.
make_pr() {
  printf 'OPEN\t%s\t%s\t%s\t%s\n' "$5" "$2" "$3" "$4" >"$FAKE_GH_DIR/pr-$1"
}

# Answers the three gh calls osi makes, from files in $FAKE_GH_DIR.
cat >"$OSI_GH" <<'EOF'
#!/bin/sh
case "$1 $2" in
  "pr view") cat "$FAKE_GH_DIR/pr-$3" ;;
  "run list")
    while [ $# -gt 0 ]; do
      if [ "$1" = --commit ]; then sha="$2"; fi
      shift
    done
    if [ -d "$FAKE_GH_DIR/build-$sha" ]; then echo 4242; fi
    ;;
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
run "$osi" update
check "new head without a finished build fails" says "no finished build for commit ffffffffffff"
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
run "$osi" install main --app
check "installs the app" [ "$(cat "$app/Contents/Resources/id")" = B ]
check "the app's signature verifies" codesign --verify --deep --strict "$app"
check "state records the app" state_has "app=$app"
make_build "$main_dir" G "$(commit 6)"
run "$osi" update
check "update also updates the app" [ "$(cat "$app/Contents/Resources/id")" = G ]
run "$osi" rollback
check "rollback also puts the app back" [ "$(cat "$app/Contents/Resources/id")" = B ]
check "and the binary" installed_is B

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
check "removes the app it installed" [ ! -e "$app" ]
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
run "$osi" install main --force
check "--force replaces it" installed_is G
printf '#!/bin/sh\necho mine\n' >"$bin/osi"
run sh "$root/install.sh"
check "install.sh does not replace another osi" [ "$(sh "$bin/osi")" = mine ]

echo "# overrides"
run env OSI_BIN_DIR="$work/elsewhere/bin" OTTOSPACE_HOME="$work/elsewhere/home" "$osi" install main
check "OSI_BIN_DIR moves the command" [ -x "$work/elsewhere/bin/ottospace" ]
check "OTTOSPACE_HOME moves the state" [ -f "$work/elsewhere/home/install/current" ]
run env OSI_RELEASES_URL="http://example.invalid" "$osi" install main
check "plain http is refused" says "refusing to download"

echo
echo "$passed passed, $failed failed"
[ "$failed" = 0 ]
