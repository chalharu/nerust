#!/usr/bin/env bash
# packaging/macos/build-app.sh
#
# Assemble a macOS .app bundle for Nerust, ad-hoc sign it, and produce a zip
# suitable for attaching to a GitHub Release.
#
# This script creates the bundle structure manually, informed by how
# tauri-bundler lays out .app directories, but without requiring the Tauri
# framework itself.
#
# Usage (run from workspace root):
#   packaging/macos/build-app.sh
#
# Environment:
#   TAG_NAME  Release tag embedded in the zip filename.
#             Defaults to the value of `git describe --tags --abbrev=0` or
#             "v0.1.0" when no tags are present.
#   VERSION   Version string embedded in Info.plist.
#             Defaults to TAG_NAME with a leading `v` stripped.
#   BINARY   Path to the compiled nerust_tao binary.
#            Defaults to target/release/nerust_tao.
#   OUT_DIR  Directory where Nerust.app and the zip are written.
#            Defaults to target/dist.
#
# Output:
#   <OUT_DIR>/Nerust.app              - the app bundle
#   <OUT_DIR>/nerust-<tag>-macos-aarch64.app.zip   - zipped bundle
#
# Requirements on the build host:
#   codesign   (Xcode Command Line Tools)
#   iconutil   (Xcode Command Line Tools)
#   ditto      (built-in macOS utility, used for zip to preserve resource forks)

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" --format-version 1 --no-deps | sed -n 's#.*\"target_directory\":\"\\([^\"]*\\)\".*#\\1#p')}"

TAG_NAME="${TAG_NAME:-$(git -C "${WORKSPACE_ROOT}" describe --tags --abbrev=0 2>/dev/null || echo "v0.1.0")}"
VERSION="${VERSION:-${TAG_NAME#v}}"
BINARY="${BINARY:-${TARGET_DIR}/release/nerust_tao}"
OUT_DIR="${OUT_DIR:-${TARGET_DIR}/dist}"

APP_NAME="Nerust"
APP_DIR="${OUT_DIR}/${APP_NAME}.app"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"
ZIP_NAME="nerust-${TAG_NAME}-macos-aarch64.app.zip"

echo "Building ${APP_NAME}.app  version=${VERSION}"
echo "  binary : ${BINARY}"
echo "  output : ${OUT_DIR}/${ZIP_NAME}"

# ---------------------------------------------------------------------------
# Validate inputs
# ---------------------------------------------------------------------------

if [[ ! -f "${BINARY}" ]]; then
    echo "Error: binary not found at '${BINARY}'" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Build .app bundle structure
# ---------------------------------------------------------------------------

rm -rf "${APP_DIR}"
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"

# Executable
cp "${BINARY}" "${MACOS_DIR}/nerust_tao"
chmod +x "${MACOS_DIR}/nerust_tao"

# Info.plist (substitute version placeholder)
sed "s/__VERSION__/${VERSION}/g" \
    "${SCRIPT_DIR}/Info.plist" \
    > "${CONTENTS_DIR}/Info.plist"

# PkgInfo (type + creator signature)
printf 'APPL????' > "${CONTENTS_DIR}/PkgInfo"

# ---------------------------------------------------------------------------
# Icon
# ---------------------------------------------------------------------------

ICONS_SRC="${SCRIPT_DIR}/icons/icon_1024x1024.png"
ICONSET_DIR="${OUT_DIR}/AppIcon.iconset"

if [[ -f "${ICONS_SRC}" ]]; then
    echo "Generating AppIcon.icns from ${ICONS_SRC}…"
    rm -rf "${ICONSET_DIR}"
    mkdir -p "${ICONSET_DIR}"

    # Generate required icon sizes via sips (built-in on macOS)
    for SIZE in 16 32 64 128 256 512 1024; do
        sips -z "${SIZE}" "${SIZE}" "${ICONS_SRC}" \
            --out "${ICONSET_DIR}/icon_${SIZE}x${SIZE}.png" \
            > /dev/null 2>&1
    done
    # @2x variants (Retina)
    for SIZE in 16 32 64 128 256 512; do
        DOUBLE=$(( SIZE * 2 ))
        cp "${ICONSET_DIR}/icon_${DOUBLE}x${DOUBLE}.png" \
           "${ICONSET_DIR}/icon_${SIZE}x${SIZE}@2x.png"
    done

    iconutil --convert icns \
        --output "${RESOURCES_DIR}/AppIcon.icns" \
        "${ICONSET_DIR}"

    rm -rf "${ICONSET_DIR}"
    echo "AppIcon.icns written."
else
    echo "Warning: icon source not found at '${ICONS_SRC}'; bundle will use the system generic icon." >&2
fi

# ---------------------------------------------------------------------------
# Ad-hoc code signing
#
# Sign with identity "-" (ad-hoc). This keeps the bundle signed without
# requiring an Apple Developer account. The resulting archive is intentionally
# not notarized, so users may need to bypass Gatekeeper on first launch.
# ---------------------------------------------------------------------------

echo "Ad-hoc signing ${APP_NAME}.app…"
codesign \
    --force \
    --deep \
    --sign - \
    --entitlements "${SCRIPT_DIR}/Entitlements.plist" \
    --options runtime \
    "${APP_DIR}"

codesign --verify --deep --strict "${APP_DIR}"
echo "Signature verified."

# ---------------------------------------------------------------------------
# Zip the bundle
#
# Use `ditto` which preserves the resource fork and extended attributes that
# some macOS tools expect. Plain `zip` can corrupt .app bundles.
# ---------------------------------------------------------------------------

mkdir -p "${OUT_DIR}"
OUTPUT_ZIP="${OUT_DIR}/${ZIP_NAME}"
rm -f "${OUTPUT_ZIP}"

ditto -c -k --sequesterRsrc --keepParent "${APP_DIR}" "${OUTPUT_ZIP}"
echo "Created ${OUTPUT_ZIP}"
