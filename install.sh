#!/bin/sh
# Installs ottospace.app into ~/Applications and an `ottospace` command into ~/.local/bin. No sudo.
#   curl -fsSL https://raw.githubusercontent.com/bryan-uipath/ottospace/main/install.sh | sh
# Pin a release: ... | OTTOSPACE_VERSION=v0.1.0 sh
set -eu

# System tools only; the caller's PATH is kept just to check for ~/.local/bin.
user_path="${PATH:-}"
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

repo="bryan-uipath/ottospace"
# Asset name must match release.yml: ottospace-<version>-<target>.zip.
target="aarch64-apple-darwin"
# Marks the wrapper as ours, so a second run may replace it.
marker="# Installed by ottospace install.sh"

tmp=""
staged=""
old=""
dest=""

say() { printf '%s\n' "$*"; }
die() {
  printf 'ottospace install: %s\n' "$*" >&2
  exit 1
}

# Runs on any exit: drops temp files and, if interrupted mid-swap, puts the old app back.
cleanup() {
  if [ -n "$staged" ]; then rm -rf "$staged"; fi
  if [ -n "$old" ] && [ -e "$old" ] && [ ! -e "$dest" ]; then mv "$old" "$dest"; fi
  if [ -n "$tmp" ]; then rm -rf "$tmp"; fi
}

# HTTPS only, also across redirects (release assets redirect to GitHub's download host).
fetch() {
  curl -fsSL --proto '=https' --tlsv1.2 --retry 3 -o "$2" "$1"
}

# Tags look like v0.1.0; refuse anything else before it goes into a URL.
check_tag() {
  case "$1" in
    v[0-9]*) ;;
    *) die "bad version '$1' (expected vX.Y.Z)" ;;
  esac
  case "$1" in
    *[!0-9A-Za-z.+-]*) die "bad version '$1' (expected vX.Y.Z)" ;;
  esac
}

latest_tag() {
  fetch "https://api.github.com/repos/$repo/releases/latest" "$tmp/latest.json" ||
    die "could not read the latest release from api.github.com"
  # No jq: split the JSON at commas and braces, take the first top-level "tag_name" value.
  tr ',{}' '[\n*]' <"$tmp/latest.json" |
    sed -n 's/^[[:space:]]*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*$/\1/p' |
    head -n 1
}

# Checks $tmp/ottospace.zip against the first field of a "<sha256>  <name>" file.
verify() {
  hash="$(awk 'NR == 1 { print $1 }' "$1")"
  case "$hash" in
    *[!0-9a-f]*) die "checksum file is not a SHA-256: $1" ;;
  esac
  [ "${#hash}" -eq 64 ] || die "checksum file is not a SHA-256: $1"
  (cd "$tmp" && printf '%s  ottospace.zip\n' "$hash" | shasum -a 256 -c - >/dev/null) ||
    die "checksum mismatch: the download is damaged or was changed. Nothing was installed."
}

main() {
  [ "$(uname -s)" = Darwin ] || die "ottospace runs on macOS only."
  # hw.optional.arm64 is 1 on Apple Silicon, also under Rosetta where uname -m says x86_64.
  [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ] ||
    die "ottospace releases are built for Apple Silicon only. Build from source on Intel Macs."
  case "${HOME:-}" in
    /*) ;;
    *) die "HOME must be an absolute path" ;;
  esac

  apps="$HOME/Applications"
  bin="$HOME/.local/bin"
  dest="$apps/ottospace.app"
  cmd="$bin/ottospace"

  # Never overwrite an `ottospace` command this script did not write.
  if [ -L "$cmd" ] || { [ -e "$cmd" ] && ! grep -qxF "$marker" "$cmd"; }; then
    die "$cmd exists and was not installed by this script. Remove it and run again."
  fi

  trap cleanup EXIT
  trap 'exit 1' HUP INT TERM
  tmp="$(mktemp -d)"

  if [ -n "${OTTOSPACE_ZIP:-}" ]; then
    # Local zip, for testing the install steps. A sibling .sha256 file is verified if present.
    cp -- "$OTTOSPACE_ZIP" "$tmp/ottospace.zip"
    if [ -f "$OTTOSPACE_ZIP.sha256" ]; then
      verify "$OTTOSPACE_ZIP.sha256"
    else
      say "No $OTTOSPACE_ZIP.sha256: checksum not verified."
    fi
    tag="local"
  else
    tag="${OTTOSPACE_VERSION:-}"
    if [ -z "$tag" ]; then
      tag="$(latest_tag)"
      [ -n "$tag" ] || die "no release found at https://github.com/$repo/releases"
    fi
    check_tag "$tag"
    asset="ottospace-${tag#v}-$target.zip"
    url="https://github.com/$repo/releases/download/$tag/$asset"
    say "Downloading $url"
    fetch "$url" "$tmp/ottospace.zip" || die "download failed: $url"
    fetch "$url.sha256" "$tmp/ottospace.zip.sha256" || die "download failed: $url.sha256"
    verify "$tmp/ottospace.zip.sha256"
  fi

  ditto -x -k "$tmp/ottospace.zip" "$tmp/unpacked"
  app="$tmp/unpacked/ottospace.app"
  [ -x "$app/Contents/MacOS/ottospace" ] || die "the zip holds no ottospace.app"
  codesign --verify --deep --strict "$app" 2>/dev/null || die "the app's signature does not verify"

  # Stage next to the destination, so the swap is two renames on one volume.
  mkdir -p "$apps"
  staged="$apps/.ottospace.app.new.$$"
  old="$apps/.ottospace.app.old.$$"
  rm -rf "$staged" "$old"
  ditto "$app" "$staged"
  if [ -e "$dest" ] || [ -L "$dest" ]; then mv "$dest" "$old"; fi
  mv "$staged" "$dest"
  staged=""
  rm -rf "$old"
  old=""

  # A wrapper, not a symlink: exec by full path keeps the bundle identity when launched from a shell.
  mkdir -p "$bin"
  exe="$dest/Contents/MacOS/ottospace"
  quoted="'$(printf '%s' "$exe" | sed "s/'/'\\\\''/g")'"
  printf '#!/bin/sh\n%s\nexec %s "$@"\n' "$marker" "$quoted" >"$tmp/wrapper"
  chmod 755 "$tmp/wrapper"
  mv "$tmp/wrapper" "$cmd"

  say "Installed ottospace ($tag):"
  say "  app:     $dest"
  say "  command: $cmd"
  case ":$user_path:" in
    *":$bin:"*) ;;
    *) say "  $bin is not on your PATH. Add it, e.g. in ~/.zshrc: export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
  esac
  say ""
  say "Open it from ~/Applications or run: open \"$dest\""
  say "Then allow ottospace in System Settings > Privacy & Security, under both"
  say "Accessibility and Screen & System Audio Recording. It exits at start without Accessibility."
  say "If ottospace was running, quit and reopen it to use the new version."
}

main "$@"
