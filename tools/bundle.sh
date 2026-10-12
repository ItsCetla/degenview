#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
CARGO_BIN="${CARGO_BIN:-cargo}"
identity="${DEGENVIEW_CODESIGN_IDENTITY:-}"
access_group="${DEGENVIEW_KEYCHAIN_ACCESS_GROUP:-}"
# Cargo.toml is the one place the version lives; tools/release.sh bumps it before tagging.
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
if [ -z "$version" ]; then
    echo "could not read the version from Cargo.toml" >&2
    exit 1
fi

if { [ -n "$identity" ] && [ -z "$access_group" ]; } || \
   { [ -z "$identity" ] && [ -n "$access_group" ]; }; then
    echo "DEGENVIEW_CODESIGN_IDENTITY and DEGENVIEW_KEYCHAIN_ACCESS_GROUP must be supplied together" >&2
    exit 1
fi

"$CARGO_BIN" build --release --bin degenview --bin degenview-alert-agent

app="target/DegenView.app"
helper="$app/Contents/Library/LoginItems/DegenViewAlertAgent.app"
executable="$app/Contents/MacOS/DegenView"
helper_executable="$helper/Contents/MacOS/DegenViewAlertAgent"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
mkdir -p "$helper/Contents/MacOS"
cp target/release/degenview "$executable"
cp target/release/degenview-alert-agent "$helper_executable"
cp assets/brand/DegenView.icns "$app/Contents/Resources/DegenView.icns"

cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>DegenView</string>
  <key>CFBundleDisplayName</key><string>DegenView</string>
  <key>CFBundleIdentifier</key><string>com.degenview.rust</string>
  <key>CFBundleExecutable</key><string>DegenView</string>
  <key>CFBundleIconFile</key><string>DegenView.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>__VERSION__</string>
  <key>CFBundleShortVersionString</key><string>__VERSION__</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

cat > "$helper/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>DegenView Alert Agent</string>
  <key>CFBundleDisplayName</key><string>DegenView Alert Agent</string>
  <key>CFBundleIdentifier</key><string>com.degenview.rust.alert-agent</string>
  <key>CFBundleExecutable</key><string>DegenViewAlertAgent</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>__VERSION__</string>
  <key>CFBundleShortVersionString</key><string>__VERSION__</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST

# Info.plist versions are numeric: a pre-release tag (0.2.0-rc1) ships as 0.2.0.
sed -i '' "s/__VERSION__/${version%%-*}/" "$app/Contents/Info.plist" "$helper/Contents/Info.plist"

if [ -n "$identity" ]; then
    entitlements="target/DegenView.entitlements"
    sed "s|__KEYCHAIN_ACCESS_GROUP__|$access_group|g" tools/entitlements.plist > "$entitlements"
    codesign --force --options runtime --timestamp --entitlements "$entitlements" \
        --sign "$identity" "$helper"
    codesign --force --options runtime --timestamp --entitlements "$entitlements" \
        --sign "$identity" "$app"
    codesign --verify --deep --strict --verbose=2 "$app"
    echo "Built and signed $app (background agent available)"
else
    echo "Built $app unsigned; background alerts use the foreground fallback"
fi
