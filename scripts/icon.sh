#!/usr/bin/env bash
# Regenerates crates/ottospace/assets/ottospace.icns from icon.svg, and from icon-small.svg for
# the 16 and 32 pixel sizes (heavier lines). Needs Google Chrome.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
assets="$root/crates/ottospace/assets"
icns="$assets/ottospace.icns"
chrome="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
iconset="$tmp/ottospace.iconset"
mkdir "$iconset"

# Headless Chrome keeps the transparent corners; sips and qlmanage can flatten them.
for name in icon icon-small; do
  "$chrome" --headless=new --hide-scrollbars --default-background-color=00000000 \
    --window-size=1024,1024 --screenshot="$tmp/$name.png" "file://$assets/$name.svg" 2>/dev/null || true
  # Chrome can exit non-zero after writing the file (slow teardown), so check the file instead.
  [ -s "$tmp/$name.png" ] || { echo "No screenshot from $chrome (set CHROME to another path)" >&2; exit 1; }
done

# iconutil wants each size at 1x and 2x, e.g. icon_16x16.png (16px) and icon_16x16@2x.png (32px).
for size in 16 32 128 256 512; do
  for scale in 1 2; do
    pixels=$((size * scale))
    from=icon; [ "$pixels" -le 32 ] && from=icon-small
    suffix=""; [ "$scale" = 2 ] && suffix="@2x"
    sips -z "$pixels" "$pixels" "$tmp/$from.png" --out "$iconset/icon_${size}x${size}${suffix}.png" >/dev/null
  done
done

iconutil -c icns "$iconset" -o "$icns"
echo "$icns"
