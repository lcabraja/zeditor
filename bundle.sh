#!/bin/bash
set -euo pipefail

APP_NAME="Zeditor"
BUNDLE_DIR="target/${APP_NAME}.app"
BINARY_NAME="popup-editor"

# Build release
cargo build --release

# Create .app bundle structure
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/Contents/MacOS"
mkdir -p "$BUNDLE_DIR/Contents/Resources"

# Copy binary
cp "target/release/${BINARY_NAME}" "$BUNDLE_DIR/Contents/MacOS/${BINARY_NAME}"

# Copy Info.plist
cp Info.plist "$BUNDLE_DIR/Contents/Info.plist"

echo "Built ${BUNDLE_DIR}"
echo "Run with: open ${BUNDLE_DIR}"
