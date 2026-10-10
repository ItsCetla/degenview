#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
CARGO_BIN="${CARGO_BIN:-cargo}"
"$CARGO_BIN" build --release

app="target/DegenView.app"
executable="$app/Contents/MacOS/DegenView"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/degenview "$executable"

cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>DegenView</string>
  <key>CFBundleDisplayName</key><string>DegenView</string>
  <key>CFBundleIdentifier</key><string>com.degenview.rust</string>
  <key>CFBundleExecutable</key><string>DegenView</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

echo "Built $app"
