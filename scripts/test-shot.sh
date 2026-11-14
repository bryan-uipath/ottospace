#!/usr/bin/env bash
# Tests scripts/shot against a debug build: one tiny scenario, its picture, and the compare path
# with the same build on both sides. Needs a screen that draws (not locked) and Screen Recording
# for the terminal; a hosted macOS runner has both (shots.yml runs it).
#   scripts/test-shot.sh [path/to/ottospace]
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
shot="$root/scripts/shot"
image() { /usr/bin/osascript -l JavaScript "$root/scripts/shots/image.js" "$@"; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

bin="${1:-}"
if [ -z "$bin" ]; then
  cargo build -q -p ottospace --bin ottospace --manifest-path "$root/Cargo.toml"
  bin="$(cargo metadata --format-version 1 --no-deps --manifest-path "$root/Cargo.toml" |
    /usr/bin/python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/debug/ottospace"
fi

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
  out="$("$@" 2>&1)" || status=$?
}
says() { printf '%s' "$out" | grep -qF -- "$1"; }
# field <png> <key>: a number from image.js stats.
field() { image stats "$1" | /usr/bin/python3 -c "import json,sys; print(json.load(sys.stdin)['$2'])"; }
# The instances' own folders: none may be left.
leftovers() { find "${TMPDIR:-/tmp}" -maxdepth 1 -name 'ottospace-shot-*' -newer "$work/start" | wc -l | tr -d ' '; }
touch "$work/start"

echo "# one scenario"
run "$shot" first-run --bin "$bin" --out "$work/out"
png="$work/out/first-run/empty.png"
check "it runs" [ "$status" = 0 ]
check "the picture is written" [ -s "$png" ]
check "more than one colour" [ "$(field "$png" colors)" -gt 1 ]
check "nothing see-through: the whole window drew" [ "$(field "$png" clear)" -le 128 ]
# The window is 1400x900 points: one or two pixels a point.
size="$(field "$png" w)x$(field "$png" h)"
check "the window's size ($size)" test "$size" = 1400x900 -o "$size" = 2800x1800
check "its temp folder is gone" [ "$(leftovers)" = 0 ]

echo "# compare, the same build on both sides"
run "$shot" --compare HEAD first-run --bin-before "$bin" --bin-after "$bin" --out "$work/cmp"
check "it runs" [ "$status" = 0 ]
before="$work/cmp/before/first-run/empty.png"
after="$work/cmp/after/first-run/empty.png"
check "before and after are written" test -s "$before" -a -s "$after"
check "the same size" [ "$(field "$before" w)x$(field "$before" h)" = "$(field "$after" w)x$(field "$after" h)" ]
check "one side-by-side" [ -s "$work/cmp/compare/first-run-empty.png" ]
check "a snippet for the body" says "![first-run/empty](<url of compare/first-run-empty.png>)"
check "no temp folder left" [ "$(leftovers)" = 0 ]

echo "# refusals"
run "$shot" no-such-scenario --bin "$bin" --out "$work/none"
check "an unknown scenario fails, naming the others" says "no scenario no-such-scenario"
check "and leaves nothing" [ "$(leftovers)" = 0 ]

echo
echo "$passed passed, $failed failed"
[ "$failed" = 0 ]
