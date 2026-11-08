#!/usr/bin/env bash
# Builds dist/UniHub.app and packs it into release/UniHub-<version>.dmg, with a
# release/UniHub-<version>.dmg.sha256 checksum next to it.
#
# Publish both files as assets of a GitHub release tagged v<version>: UniHub's updater
# reads the latest release, and refuses a DMG without its .sha256 file.
# The version comes from Cargo.toml ([workspace.package] version); bump it first.
set -euo pipefail
cd "$(dirname "$0")/.."

./scripts/bundle-app.sh

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="UniHub-$VERSION.dmg"
mkdir -p release
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT

# The window shows the app and a link to /Applications to drag it onto.
ditto dist/UniHub.app "$STAGE/UniHub.app"
ln -s /Applications "$STAGE/Applications"

rm -f "release/$NAME" "release/$NAME.sha256"
hdiutil create -volname "UniHub $VERSION" -srcfolder "$STAGE" -fs HFS+ -format UDZO \
  -imagekey zlib-level=9 -ov "release/$NAME" >/dev/null
(cd release && shasum -a 256 "$NAME" > "$NAME.sha256")

echo "Built release/$NAME ($(du -h "release/$NAME" | cut -f1))"
echo "Checksum: $(cut -d' ' -f1 "release/$NAME.sha256")"
