#!/usr/bin/env bash
# Tests scripts/e2e-for's selection: fixture changes in a temp git repo holding this e2e.rs, and the
# table against the tests that exist. No build, no screen, no network: git and python3 only.
#   scripts/test-e2e-for.sh
set -euo pipefail
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

root="$(cd "$(dirname "$0")/.." && pwd)"
for_="$root/scripts/e2e-for"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

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
check() {
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then ok "$name"; else fail "$name"; fi
}
run() {
  status=0
  out="$(cd "$repo" && "$@" 2>&1)" || status=$?
}
says() { printf '%s' "$out" | grep -qF -- "$1"; }
# picks <test>: the test is listed (a line of its own, two spaces in).
picks() { printf '%s\n' "$out" | grep -qx "  $1"; }
# A change on a branch off the fixture's main, then back to main for the next case.
change() {
  git -C "$repo" checkout -q -b "case-$1" main
  shift
  "$@"
  git -C "$repo" add -A
  git -C "$repo" commit -qm case
}
edit() { /usr/bin/python3 -I -c 'import sys; p,a,b=sys.argv[1:]; s=open(p).read(); assert a in s, a; open(p,"w").write(s.replace(a,b,1))' "$@"; }

repo="$work/repo"
mkdir -p "$repo/crates/ottospace/tests" "$repo/crates/ottospace/src" "$repo/docs"
cp "$root/crates/ottospace/tests/e2e.rs" "$repo/crates/ottospace/tests/e2e.rs"
for f in calendar.rs terminal.rs; do echo "// $f" >"$repo/crates/ottospace/src/$f"; done
echo "# docs" >"$repo/docs/a.md"
git -C "$repo" init -q -b main
git -C "$repo" -c user.name=t -c user.email=t@t add -A
git -C "$repo" -c user.name=t -c user.email=t@t commit -qm base
export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@t GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@t
e2e="$repo/crates/ottospace/tests/e2e.rs"

echo "# the table"
out="$(cd "$root" && "$for_" --check 2>&1)" && status=0 || status=$?
check "matches this repository's tests and source files" [ "$status" = 0 ]
run "$for_" --check
check "and the fixture's" [ "$status" = 0 ]

echo "# docs only: the smoke set"
change docs sh -c "echo more >>'$repo/docs/a.md'"
run "$for_" main
check "exits 0" [ "$status" = 0 ]
check "the smoke set" picks splits_are_drawn_where_the_layout_says
check "says why" says "smoke"
check "nothing else" [ "$(printf '%s\n' "$out" | grep -cE '^  [a-z_]+$')" = 5 ]

echo "# a test changed in e2e.rs"
git -C "$repo" checkout -q main
change test edit "$e2e" 'fn terminal_runs_a_typed_command() {' $'fn terminal_runs_a_typed_command() {\n    let _ = 1;'
run "$for_" main
check "picks it" picks terminal_runs_a_typed_command
check "because it changed" says "changed in e2e.rs"
check "not the calendar's" sh -c "! printf '%s' \"\$1\" | grep -q calendar" _ "$out"

echo "# a helper changed in e2e.rs: the tests that name it"
git -C "$repo" checkout -q main
change helper edit "$e2e" 'fn transcript_header(name: &str' $'// more\nfn transcript_header(name: &str'
run "$for_" main
check "a test that uses it" picks a_transcript_with_no_meeting_on_has_no_calendar_block
check "says so" says "uses transcript_header, changed in e2e.rs"

check "not one that does not" sh -c "! printf '%s' \"\$1\" | grep -q '^  the_calendar_says'" _ "$out"

echo "# the harness, or a helper it uses, changed: the full suite"
git -C "$repo" checkout -q main
change harness edit "$e2e" '    fn launch(name: &str) -> Self {' $'    fn launch(name: &str) -> Self {\n        let _ = 1;'
run "$for_" main
check "exits 0" [ "$status" = 0 ]
check "says it reaches every test" says "(Otto, not a test) reaches every test"
check "prints the full suite" says "cargo nextest run -p ottospace --test e2e --run-ignored only --profile e2e"
check "lists no test" [ "$(printf '%s\n' "$out" | grep -cE '^  [a-z_]+$')" = 0 ]
git -C "$repo" checkout -q main
change harness-helper edit "$e2e" 'fn rect(value: &Value) -> [f64; 4] {' $'fn rect(value: &Value) -> [f64; 4] {\n    let _ = 1;'
run "$for_" main
check "a helper the harness uses: every test" says "(rect, not a test) reaches every test"

echo "# a source file, by area"
git -C "$repo" checkout -q main
change area sh -c "echo '// more' >>'$repo/crates/ottospace/src/calendar.rs'"
run "$for_" main
check "a calendar test" picks the_calendar_says_it_loads_what_failed_and_tries_again
check "says which path and what matched" says 'crates/ottospace/src/calendar.rs ("calendar")'
check "not a transcript's own" sh -c "! printf '%s' \"\$1\" | grep -q '^  a_transcript_starts'" _ "$out"

echo "# a file the table does not know"
git -C "$repo" checkout -q main
change unknown sh -c "echo '//' >'$repo/crates/ottospace/src/zz_new_thing.rs'"
run "$for_" main
check "exits 0" [ "$status" = 0 ]
check "says so" says "the table does not know crates/ottospace/src/zz_new_thing.rs"
check "prints the full suite" says "run the full suite instead"
run "$for_" main --run --ci-size
check "--run runs nothing: exits 3" [ "$status" = 3 ]
check "--ci-size names the window" says "OTTOSPACE_E2E_WINDOW=1024x674 cargo nextest run"

echo "# the working tree, untracked files too"
git -C "$repo" checkout -q main
mkdir -p "$repo/crates/ottospace-core/src"
echo '//' >"$repo/crates/ottospace-core/src/vim.rs"
run "$for_" main
check "an untracked file counts" picks capture_and_wait_read_one_pane
check "against the working tree" says "to the working tree"
rm "$repo/crates/ottospace-core/src/vim.rs"

echo "# --head"
run "$for_" main --head case-area
check "the diff to that revision" picks the_calendar_says_it_loads_what_failed_and_tries_again
run "$for_" main --head case-area --run
check "--run with --head is refused" [ "$status" = 2 ]

echo "# a table that names a test there is not"
mkdir -p "$work/bad"
cp "$for_" "$work/bad/e2e-for"
{ cat "$root/scripts/e2e-for.areas"; echo "smoke: no_such_test"; } >"$work/bad/e2e-for.areas"
run "$work/bad/e2e-for" --check
check "fails" [ "$status" = 1 ]
check "names it" says "no test no_such_test"

echo
echo "$passed passed, $failed failed"
[ "$failed" = 0 ]
