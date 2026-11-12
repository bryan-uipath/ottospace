#!/usr/bin/env bash
# Regenerates crates/ottospace/assets/ottospace.icns from the icon SVG. Needs Google Chrome.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
svg="$root/docs/ai/1-initial-brainstorm/background/otto-icon-cyan.svg"
icns="$root/crates/ottospace/assets/ottospace.icns"
chrome="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
iconset="$tmp/ottospace.iconset"
mkdir "$iconset"

# Headless Chrome keeps the transparent corners; sips and qlmanage can flatten them.
"$chrome" --headless=new --hide-scrollbars --default-background-color=00000000 \
  --window-size=1024,1024 --screenshot="$tmp/1024.png" "file://$svg" 2>/dev/null || true
# Chrome can exit non-zero after writing the file (slow teardown), so check the file instead.
[ -s "$tmp/1024.png" ] || { echo "No screenshot from $chrome (set CHROME to another path)" >&2; exit 1; }

# iconutil wants each size at 1x and 2x, e.g. icon_16x16.png (16px) and icon_16x16@2x.png (32px).
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$tmp/1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$((size * 2))" "$((size * 2))" "$tmp/1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "$iconset" -o "$icns"
echo "$icns"
