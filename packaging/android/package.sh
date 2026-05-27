#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" --format-version 1 --no-deps | perl -0ne 'print $1 if /"target_directory"\s*:\s*"([^"]+)"/')}"

TAG_NAME="${TAG_NAME:-$(git -C "${WORKSPACE_ROOT}" describe --tags --abbrev=0 2>/dev/null || echo "v0.1.0")}"
OUT_DIR="${OUT_DIR:-${TARGET_DIR}/dist}"
JNI_LIBS_DIR="${SCRIPT_DIR}/app/src/main/jniLibs"
APK_SRC="${SCRIPT_DIR}/app/build/outputs/apk/release/app-release.apk"
APK_NAME="nerust-${TAG_NAME}-android-arm64-v8a.apk"

echo "Packaging Android APK"
echo "  output : ${OUT_DIR}/${APK_NAME}"

rm -rf "${JNI_LIBS_DIR}"
mkdir -p "${JNI_LIBS_DIR}"

cargo ndk \
    --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" \
    -t arm64-v8a \
    -o "${JNI_LIBS_DIR}" \
    build -p nerust_android --release

(
    cd "${SCRIPT_DIR}"
    ./gradlew --no-daemon assembleRelease
)

mkdir -p "${OUT_DIR}"
cp "${APK_SRC}" "${OUT_DIR}/${APK_NAME}"
sha256sum "${OUT_DIR}/${APK_NAME}" > "${OUT_DIR}/${APK_NAME}.sha256"
