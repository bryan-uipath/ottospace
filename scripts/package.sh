#!/usr/bin/env bash
# Builds what a release or a CI build publishes, into <out dir> (default: dist):
#   ottospace-<target>.tar.gz   the bare `ottospace` binary
#   ottospace-<target>.app.zip  ottospace.app
#   BUILD                       version, commit and commit date, one key=value per line
#   SHA256SUMS                  checksums of the three files above
# `scripts/osi` reads these names: change them in both places.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

out="${1:-dist}"
target_triple="aarch64-apple-darwin"

# CI passes the commit in: a pull request build checks out a detached commit.
commit="${OTTOSPACE_COMMIT:-$(git rev-parse HEAD)}"
# The commit's date, not the clock: the same commit always gives the same line.
date="${OTTOSPACE_BUILD_DATE:-$(TZ=UTC git show -s --format=%cd --date=format-local:%Y-%m-%dT%H:%M:%SZ "$commit")}"

# The binary reads both at compile time (`option_env!` in main.rs) for `ottospace --version`.
export OTTOSPACE_COMMIT="${commit:0:12}"
export OTTOSPACE_BUILD_DATE="$date"

scripts/bundle.sh

version="$(cargo pkgid -p ottospace)"
version="${version##*[#@]}"
target="$(cargo metadata --format-version 1 --no-deps | plutil -extract target_directory raw -)"

# The build must say which commit it is; a stale cached binary would not.
line="$("$target/release/ottospace" --version)"
echo "$line"
case "$line" in
  *"$OTTOSPACE_COMMIT"*) ;;
  *) echo "package: the binary does not report commit $OTTOSPACE_COMMIT" >&2; exit 1 ;;
esac

mkdir -p "$out"
name="ottospace-$target_triple"
tar -czf "$out/$name.tar.gz" -C "$target/release" ottospace
# ditto keeps the bundle layout and its signature intact.
ditto -c -k --keepParent "$target/release/bundle/ottospace.app" "$out/$name.app.zip"
printf 'version=%s\ncommit=%s\ndate=%s\ntarget=%s\n' "$version" "$commit" "$date" "$target_triple" > "$out/BUILD"
(cd "$out" && shasum -a 256 "$name.tar.gz" "$name.app.zip" BUILD > SHA256SUMS)

cat "$out/BUILD" "$out/SHA256SUMS"
ls -l "$out"
