#!/bin/bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$PROJECT_DIR"

APP_NAME="Zeditor"
BUILD_DIR="${CARGO_TARGET_DIR:-$PROJECT_DIR/target}"
BUNDLE_DIR="${BUILD_DIR}/${APP_NAME}.app"
BINARY_NAME="zeditor"

# Build release
cargo build --locked --release --bin "$BINARY_NAME" --target-dir "$BUILD_DIR"

# Create .app bundle structure
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/Contents/MacOS"
mkdir -p "$BUNDLE_DIR/Contents/Resources"

# Copy binary
cp "$BUILD_DIR/release/${BINARY_NAME}" "$BUNDLE_DIR/Contents/MacOS/${BINARY_NAME}"

# Copy Info.plist
cp Info.plist "$BUNDLE_DIR/Contents/Info.plist"
cp AppIcon.icns "$BUNDLE_DIR/Contents/Resources/AppIcon.icns"

echo "Built ${BUNDLE_DIR}"
echo "Run with: open ${BUNDLE_DIR}"
