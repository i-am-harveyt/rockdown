#!/usr/bin/env bash
set -euo pipefail

# Determine architecture
ARCH=$(uname -m)
APP_NAME="Rockdown.app"
BUNDLE_DIR="target/bundle/osx"
APP_DIR="${BUNDLE_DIR}/${APP_NAME}"

echo "==> Building release binary..."
cargo build --release

echo "==> Packaging ${APP_NAME} (${ARCH})..."
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/Contents/MacOS"
mkdir -p "${APP_DIR}/Contents/Resources"

# Copy binary
cp "target/release/rockdown" "${APP_DIR}/Contents/MacOS/rockdown"
chmod +x "${APP_DIR}/Contents/MacOS/rockdown"

# Copy Info.plist
cp "extra/macos/Info.plist" "${APP_DIR}/Contents/Info.plist"

# Optional icon
if [ -f "extra/macos/Rockdown.icns" ]; then
    cp "extra/macos/Rockdown.icns" "${APP_DIR}/Contents/Resources/Rockdown.icns"
fi

# Create zip distribution
ZIP_NAME="rockdown-macos-${ARCH}.zip"
echo "==> Creating ${ZIP_NAME}..."
(cd "${BUNDLE_DIR}" && rm -f "${ZIP_NAME}" && zip -q -r "${ZIP_NAME}" "${APP_NAME}")

echo "==> Done! App bundle created at:"
echo "    ${APP_DIR}"
echo "    ${BUNDLE_DIR}/${ZIP_NAME}"
