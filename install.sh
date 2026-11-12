#!/bin/sh
# Installs `osi`, the ottospace installer, into ~/.local/bin, then runs `osi install main`. No sudo.
#   curl -fsSL https://raw.githubusercontent.com/bryan-uipath/ottospace/main/install.sh | sh
# Another build:  ... | OTTOSPACE_CHANNEL=stable sh   (or v0.1.0)
# With the app:   ... | OTTOSPACE_APP=1 sh
set -eu

# System tools only; osi finds what else it needs.
user_path="${PATH:-}"
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

# Second line of scripts/osi: only an osi that carries it is replaced.
marker="# ottospace-installer"

tmp=""
staged=""

die() {
  printf 'ottospace install: %s\n' "$*" >&2
  exit 1
}

cleanup() {
  if [ -n "$staged" ]; then rm -f "$staged"; fi
  if [ -n "$tmp" ]; then rm -rf "$tmp"; fi
}

main() {
  case "${HOME:-}" in
    /*) ;;
    *) die "HOME must be an absolute path" ;;
  esac
  # The same overrides osi reads, so a test run stays inside its own directories.
  bin="${OSI_BIN_DIR:-$HOME/.local/bin}"
  url="${OSI_SELF_URL:-https://raw.githubusercontent.com/bryan-uipath/ottospace/main/scripts/osi}"
  # main until there is a tagged release: see docs/ai/install-and-update.md.
  channel="${OTTOSPACE_CHANNEL:-${OTTOSPACE_VERSION:-main}}"
  dest="$bin/osi"

  if { [ -e "$dest" ] || [ -L "$dest" ]; } && [ "$(sed -n 2p "$dest")" != "$marker" ]; then
    die "$dest exists and is not the ottospace installer. Remove it and run again."
  fi

  trap cleanup EXIT
  trap 'exit 1' HUP INT TERM
  tmp="$(mktemp -d)"

  # HTTPS only, also across redirects. file:// is for tests: only an override can name one.
  case "$url" in
    https://*) curl -fsSL --proto '=https' --tlsv1.2 --retry 3 -o "$tmp/osi" "$url" || die "download failed: $url" ;;
    file://*) curl -fsS --proto '=file' -o "$tmp/osi" "$url" || die "download failed: $url" ;;
    *) die "refusing to download from $url" ;;
  esac
  if [ "$(sed -n 1p "$tmp/osi")" != "#!/bin/sh" ] || [ "$(sed -n 2p "$tmp/osi")" != "$marker" ] || ! sh -n "$tmp/osi"; then
    die "the download is not the ottospace installer. Nothing was installed."
  fi

  # Stage beside the destination, then rename.
  mkdir -p "$bin"
  staged="$bin/.osi.new.$$"
  cp "$tmp/osi" "$staged"
  chmod 755 "$staged"
  mv -f "$staged" "$dest"
  staged=""
  printf 'Installed %s\n' "$dest"

  # exec skips the exit trap.
  cleanup
  tmp=""
  PATH="$user_path"
  export PATH
  if [ -n "${OTTOSPACE_APP:-}" ]; then
    exec "$dest" install "$channel" --app
  fi
  exec "$dest" install "$channel"
}

main "$@"
