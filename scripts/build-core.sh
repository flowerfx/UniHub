#!/usr/bin/env bash
# Builds the Rust core as an XCFramework and generates its Swift bindings for apple/.
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE="${PROFILE:-release}"
FLAG=$([ "$PROFILE" = release ] && echo --release || true)
TARGET=aarch64-apple-darwin
OUT=target/$TARGET/$PROFILE
GEN=target/uniffi-swift
XCF=apple/Frameworks/UniHubCoreFFI.xcframework

export MACOSX_DEPLOYMENT_TARGET=14.0
cargo build -p unihub-core $FLAG --target $TARGET

rm -rf "$GEN" && mkdir -p "$GEN/headers" apple/Frameworks apple/Sources/UniHubCore
cargo run -q -p uniffi-bindgen -- generate --library "$OUT/libunihub_core.dylib" \
  --language swift --out-dir "$GEN"

cp "$GEN/unihub_coreFFI.h" "$GEN/headers/"
cp "$GEN/unihub_coreFFI.modulemap" "$GEN/headers/module.modulemap"

rm -rf "$XCF"
xcodebuild -create-xcframework -library "$OUT/libunihub_core.a" -headers "$GEN/headers" \
  -output "$XCF" >/dev/null
cp "$GEN/unihub_core.swift" apple/Sources/UniHubCore/UniHubCore.swift

echo "Built $XCF and apple/Sources/UniHubCore/UniHubCore.swift"
