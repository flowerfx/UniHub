#!/usr/bin/env bash
# Renders apple/Resources/AppIcon.svg into apple/Resources/AppIcon.icns.
# Run it after editing the SVG; the .icns is committed so normal builds skip this step.
set -euo pipefail
cd "$(dirname "$0")/.."

SVG=apple/Resources/AppIcon.svg
ICNS=apple/Resources/AppIcon.icns
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
SET="$WORK/AppIcon.iconset"
mkdir -p "$SET"

# Render the largest size once, then let sips scale down: one WebKit pass is enough.
swift scripts/render-svg.swift "$SVG" "$WORK/1024.png" 1024
for size in 16 32 128 256 512; do
  sips -z $size $size "$WORK/1024.png" --out "$SET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double "$WORK/1024.png" --out "$SET/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "$SET" -o "$ICNS"
cp "$WORK/1024.png" apple/Resources/AppIcon-1024.png
echo "Built $ICNS ($(du -h "$ICNS" | cut -f1))"
