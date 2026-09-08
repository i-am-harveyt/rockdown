#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
case "${TARGET}" in
    aarch64-apple-darwin) ARCH="aarch64"; MACH_ARCH="arm64" ;;
    x86_64-apple-darwin) ARCH="x86_64"; MACH_ARCH="x86_64" ;;
    *) echo "Unsupported macOS target: ${TARGET}" >&2; exit 1 ;;
esac
export MACOSX_DEPLOYMENT_TARGET=12.0
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}"
APP_NAME="Rockdown.app"
BUNDLE_DIR="${CARGO_TARGET_DIR}/bundle/${TARGET}"
APP_DIR="${BUNDLE_DIR}/${APP_NAME}"
DIST_DIR="${DIST_DIR:-dist}"
SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:--}"

echo "==> Building release binary..."
cargo build --locked --release --target "${TARGET}"

echo "==> Packaging ${APP_NAME} (${ARCH})..."
rm -rf "${APP_DIR}"
mkdir -p "${APP_DIR}/Contents/MacOS"
mkdir -p "${APP_DIR}/Contents/Resources"

# Copy binary
cp "${CARGO_TARGET_DIR}/${TARGET}/release/rockdown" "${APP_DIR}/Contents/MacOS/rockdown"
chmod +x "${APP_DIR}/Contents/MacOS/rockdown"
lipo "${APP_DIR}/Contents/MacOS/rockdown" -verify_arch "${MACH_ARCH}"

# Copy Info.plist
cp "extra/macos/Info.plist" "${APP_DIR}/Contents/Info.plist"
VERSION=$(cargo metadata --locked --no-deps --format-version 1 | /usr/bin/plutil -extract packages.0.version raw -o - -)
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString ${VERSION}" "${APP_DIR}/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${GITHUB_RUN_NUMBER:-1}" "${APP_DIR}/Contents/Info.plist"
plutil -lint "${APP_DIR}/Contents/Info.plist"

# Optional icon
if [ -f "extra/macos/Rockdown.icns" ]; then
    cp "extra/macos/Rockdown.icns" "${APP_DIR}/Contents/Resources/Rockdown.icns"
fi

# Sign the complete bundle, not just the executable. Ad-hoc signing is local-only.
if [ "${SIGNING_IDENTITY}" = "-" ]; then
    echo "WARNING: Ad-hoc signing only; downloaded apps are not Gatekeeper-approved." >&2
    codesign --force --sign - "${APP_DIR}"
else
    codesign --force --options runtime --timestamp --sign "${SIGNING_IDENTITY}" "${APP_DIR}"
fi
codesign --verify --deep --strict --verbose=2 "${APP_DIR}"

mkdir -p "${DIST_DIR}"
if [ -n "${APPLE_NOTARY_PROFILE:-}" ]; then
    NOTARY_ARGS=(--keychain-profile "${APPLE_NOTARY_PROFILE}")
    if [ -n "${APPLE_NOTARY_KEYCHAIN:-}" ]; then
        NOTARY_ARGS+=(--keychain "${APPLE_NOTARY_KEYCHAIN}")
    fi
    if [ "${SIGNING_IDENTITY}" = "-" ]; then
        echo "Notarization requires a Developer ID signing identity." >&2
        exit 1
    fi
    ditto -c -k --sequesterRsrc --keepParent "${APP_DIR}" "${BUNDLE_DIR}/notarize.zip"
    xcrun notarytool submit "${BUNDLE_DIR}/notarize.zip" "${NOTARY_ARGS[@]}" --wait
    xcrun stapler staple "${APP_DIR}"
    xcrun stapler validate "${APP_DIR}"
    spctl --assess --type execute --verbose=2 "${APP_DIR}"
fi

# Archive only after signing and stapling so both downloads contain the same app.
ZIP_NAME="rockdown-macos-${ARCH}.zip"
echo "==> Creating ${ZIP_NAME}..."
rm -f "${DIST_DIR}/${ZIP_NAME}"
ditto -c -k --sequesterRsrc --keepParent "${APP_DIR}" "${DIST_DIR}/${ZIP_NAME}"

DMG_STAGE="${BUNDLE_DIR}/dmg"
rm -rf "${DMG_STAGE}"
mkdir -p "${DMG_STAGE}"
ditto "${APP_DIR}" "${DMG_STAGE}/${APP_NAME}"
ln -s /Applications "${DMG_STAGE}/Applications"
DMG_PATH="${DIST_DIR}/rockdown-macos-${ARCH}.dmg"
hdiutil create -ov -volname Rockdown -srcfolder "${DMG_STAGE}" -format UDZO "${DMG_PATH}"
if [ "${SIGNING_IDENTITY}" != "-" ]; then
    codesign --force --timestamp --sign "${SIGNING_IDENTITY}" "${DMG_PATH}"
fi
if [ -n "${APPLE_NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "${DMG_PATH}" "${NOTARY_ARGS[@]}" --wait
    xcrun stapler staple "${DMG_PATH}"
    xcrun stapler validate "${DMG_PATH}"
fi
hdiutil verify "${DMG_PATH}"

echo "==> Done! App bundle created at:"
echo "    ${APP_DIR}"
echo "    ${DIST_DIR}/${ZIP_NAME}"
echo "    ${DMG_PATH}"
