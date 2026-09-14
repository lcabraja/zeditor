#!/bin/bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$PROJECT_DIR"

APP_NAME="Zeditor"
BUILD_DIR="${CARGO_TARGET_DIR:-$PROJECT_DIR/target}"
BUNDLE_DIR="${BUILD_DIR}/${APP_NAME}.app"
INSTALL_DIR="${ZEDITOR_INSTALL_DIR:-$HOME/Applications}"
SIGNING_IDENTITY="${ZEDITOR_SIGNING_IDENTITY:-Zeditor}"
INSTALLED_APP="${INSTALL_DIR}/${APP_NAME}.app"

./bundle.sh

# Prepare and validate the replacement before stopping the running app.
mkdir -p "$INSTALL_DIR"
STAGING_DIR="$(mktemp -d "${INSTALL_DIR}/.Zeditor-update.XXXXXX")"
cleanup() {
    if [[ -d "$STAGING_DIR/previous.app" && ! -e "$INSTALLED_APP" ]]; then
        mv "$STAGING_DIR/previous.app" "$INSTALLED_APP"
    fi
    rm -rf "$STAGING_DIR"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

cp -R "$BUNDLE_DIR" "$STAGING_DIR/${APP_NAME}.app"
echo "Signing app with ${SIGNING_IDENTITY}..."
# Keep the same signing identity to preserve Accessibility permissions.
codesign --force --sign "$SIGNING_IDENTITY" "$STAGING_DIR/${APP_NAME}.app"
codesign --verify --strict "$STAGING_DIR/${APP_NAME}.app"

echo "Stopping existing Zeditor instance..."
pkill -x zeditor 2>/dev/null || true

echo "Installing to ${INSTALL_DIR}..."
if [[ -e "$INSTALLED_APP" ]]; then
    mv "$INSTALLED_APP" "$STAGING_DIR/previous.app"
fi
mv "$STAGING_DIR/${APP_NAME}.app" "$INSTALLED_APP"

echo "Launching ${APP_NAME}..."
open "$INSTALLED_APP"
echo "Done!"
