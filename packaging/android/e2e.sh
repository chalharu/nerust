#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TARGET_ABI="${ANDROID_E2E_ABI:-x86_64}"
JNI_LIBS_DIR="${SCRIPT_DIR}/app/src/main/jniLibs"

export CARGO_NDK_PLATFORM="${CARGO_NDK_PLATFORM:-26}"
export ANDROID_ABI_FILTERS="${ANDROID_ABI_FILTERS:-${TARGET_ABI}}"

echo "Running Android e2e tests"
echo "  abi: ${TARGET_ABI}"

rm -rf "${JNI_LIBS_DIR}"
mkdir -p "${JNI_LIBS_DIR}"

cargo ndk \
    --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" \
    -t "${TARGET_ABI}" \
    -o "${JNI_LIBS_DIR}" \
    build -p nerust_android

(
    cd "${SCRIPT_DIR}"
    ./gradlew --no-daemon connectedDebugAndroidTest
)
