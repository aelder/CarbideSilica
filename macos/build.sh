#!/bin/bash
# Builds the Rust bridge, generates its Swift bindings, builds the SwiftUI app
# and assembles "Carbide Silica.app" in this folder.
#
#   ./build.sh          build everything
#   ./build.sh bridge   rebuild only the bridge and bindings (then use Xcode or `swift build`)
set -euo pipefail
cd "$(dirname "$0")"

LIB=bridge/target/release
# Matches Package.swift and Info.plist. Homebrew's Rust standard library targets macOS 15.
export MACOSX_DEPLOYMENT_TARGET=15.0
GEN=build/generated

echo "==> Rust bridge"
cargo build --release --manifest-path bridge/Cargo.toml --lib

echo "==> Swift bindings"
rm -rf "$GEN" build/headers CarbideFFI.xcframework
mkdir -p "$GEN" build/headers
cargo run --quiet --release --manifest-path bridge/Cargo.toml --bin uniffi-bindgen -- \
    generate --library "$LIB/libcarbide_bridge.dylib" --language swift --out-dir "$GEN"
cp "$GEN/carbide_bridgeFFI.h" build/headers/
cp "$GEN/carbide_bridgeFFI.modulemap" build/headers/module.modulemap
xcodebuild -create-xcframework \
    -library "$LIB/libcarbide_bridge.a" -headers build/headers \
    -output CarbideFFI.xcframework >/dev/null
mkdir -p Sources/CarbideCore
cp "$GEN/carbide_bridge.swift" Sources/CarbideCore/

[ "${1:-}" = "bridge" ] && exit 0

echo "==> SwiftUI app"
swift build -c release
BIN="$(swift build -c release --show-bin-path)/CarbideSilica"
APP="Carbide Silica.app"

echo "==> $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BIN" "$APP/Contents/MacOS/CarbideSilica"
cp Info.plist "$APP/Contents/Info.plist"
# The sounds of the keys, with their licence.
mkdir -p "$APP/Contents/Resources"
cp -R Resources/Sounds "$APP/Contents/Resources/Sounds"
codesign --force --sign - "$APP" >/dev/null 2>&1
echo "Built $(pwd)/$APP"
