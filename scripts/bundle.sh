#!/usr/bin/env bash
# Builds <target dir>/release/bundle/ottospace.app: release binary, icon, Info.plist, ad hoc signature.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

cargo build --release --locked -p ottospace --bin ottospace

# Package ID ends in the version, e.g. "path+file:///.../crates/ottospace#0.1.0".
version="$(cargo pkgid -p ottospace)"
version="${version##*[#@]}"

# Ask cargo where it built: CARGO_TARGET_DIR or cargo config can move it out of ./target.
target="$(cargo metadata --format-version 1 --no-deps | plutil -extract target_directory raw -)"

app="$target/release/bundle/ottospace.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/release/ottospace" "$app/Contents/MacOS/ottospace"
cp crates/ottospace/assets/ottospace.icns "$app/Contents/Resources/ottospace.icns"

# Accessibility and Screen Recording have no usage key. These cover programs run in terminal panes.
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>ottospace</string>
  <key>CFBundleDisplayName</key><string>ottospace</string>
  <key>CFBundleIdentifier</key><string>com.bryan-uipath.ottospace</string>
  <key>CFBundleExecutable</key><string>ottospace</string>
  <key>CFBundleIconFile</key><string>ottospace</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSAppleEventsUsageDescription</key><string>A program in an ottospace terminal wants to control another app.</string>
  <key>NSDesktopFolderUsageDescription</key><string>A program in an ottospace terminal wants to use the Desktop folder.</string>
  <key>NSDocumentsFolderUsageDescription</key><string>A program in an ottospace terminal wants to use the Documents folder.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>A program in an ottospace terminal wants to use the Downloads folder.</string>
</dict>
</plist>
EOF

# Ad hoc: runs on this Mac, but each build is a new identity, so macOS may ask for permissions again.
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"

echo "$app"
echo "Open it: open '$app'"
