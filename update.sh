#!/bin/bash
set -euo pipefail

APP_NAME="Zeditor"
BUNDLE_DIR="target/${APP_NAME}.app"
BINARY_NAME="popup-editor"
INSTALL_DIR="$HOME/Applications"

echo "Building release..."
cargo build --release

echo "Killing existing Zeditor instance..."
pkill -x "$BINARY_NAME" 2>/dev/null || true

echo "Creating app bundle..."
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/Contents/MacOS"
mkdir -p "$BUNDLE_DIR/Contents/Resources"
cp "target/release/${BINARY_NAME}" "$BUNDLE_DIR/Contents/MacOS/${BINARY_NAME}"
cp Info.plist "$BUNDLE_DIR/Contents/Info.plist"
cp AppIcon.icns "$BUNDLE_DIR/Contents/Resources/AppIcon.icns"

echo "Installing to ${INSTALL_DIR}..."
rm -rf "${INSTALL_DIR}/${APP_NAME}.app"
cp -R "$BUNDLE_DIR" "${INSTALL_DIR}/${APP_NAME}.app"

# Sign with "Zeditor" certificate to preserve Accessibility permissions across rebuilds
echo "Signing app..."
codesign --force --sign "Zeditor" "${INSTALL_DIR}/${APP_NAME}.app"

echo "Launching ${APP_NAME}..."
open "${INSTALL_DIR}/${APP_NAME}.app"

echo "Done!"
