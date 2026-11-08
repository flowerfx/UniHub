#!/usr/bin/env bash
# Builds the Rust core and the SwiftUI app, then wraps it in dist/UniHub.app.
set -euo pipefail
cd "$(dirname "$0")/.."

./scripts/build-core.sh
(cd apple && swift build -c release --arch arm64)
BIN=$(cd apple && swift build -c release --arch arm64 --show-bin-path)

APP=dist/UniHub.app
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN/UniHub" "$APP/Contents/MacOS/UniHub"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>UniHub</string>
  <key>CFBundleDisplayName</key><string>UniHub</string>
  <key>CFBundleIdentifier</key><string>io.github.unihub.app</string>
  <key>CFBundleExecutable</key><string>UniHub</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

codesign --force --sign - "$APP" >/dev/null 2>&1
echo "Built $APP ($(du -sh "$APP" | cut -f1))"
