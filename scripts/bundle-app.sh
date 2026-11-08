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
cp apple/Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

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
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Sign with a fixed identity when one is set: $UNIHUB_SIGN_IDENTITY, or the SHA-1 or name
# of a code-signing certificate in .signing-identity (not committed; see README). macOS
# keys privacy permissions (Documents access…) to the signer, so with a fixed identity
# they survive updates. Without one the app is ad-hoc signed and macOS asks again after
# every update.
IDENTITY="${UNIHUB_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ] && [ -f .signing-identity ]; then
  IDENTITY=$(grep -v '^[[:space:]]*#' .signing-identity | head -1 | tr -d '[:space:]')
fi
if [ -n "$IDENTITY" ] && ! security find-identity -v -p codesigning | grep -qF "$IDENTITY"; then
  echo "warning: signing identity $IDENTITY is not in the keychain; signing ad-hoc instead" >&2
  IDENTITY=""
fi
if [ -n "$IDENTITY" ]; then
  codesign --force --sign "$IDENTITY" --timestamp=none "$APP"
  echo "Signed with $(codesign -dvv "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1)"
else
  echo "warning: no signing identity (.signing-identity); signing ad-hoc" >&2
  codesign --force --sign - "$APP" >/dev/null 2>&1
fi
echo "Built $APP ($(du -sh "$APP" | cut -f1))"
