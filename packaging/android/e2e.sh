#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TARGET_ABI="${ANDROID_E2E_ABI:-x86_64}"
JNI_LIBS_DIR="${SCRIPT_DIR}/app/src/main/jniLibs"
APP_PACKAGE="io.github.chalharu.nerust"
TEST_PACKAGE="${APP_PACKAGE}.test"
TEST_RUNNER="${TEST_PACKAGE}/androidx.test.runner.AndroidJUnitRunner"
TEST_CLASS="io.github.chalharu.nerust.MainActivityE2eTest"
APP_APK="${SCRIPT_DIR}/app/build/outputs/apk/debug/app-debug.apk"
TEST_APK="${SCRIPT_DIR}/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk"

export CARGO_NDK_PLATFORM="${CARGO_NDK_PLATFORM:-28}"
export ANDROID_ABI_FILTERS="${ANDROID_ABI_FILTERS:-${TARGET_ABI}}"

echo "Running Android e2e tests"
echo "  abi: ${TARGET_ABI}"

dump_logcat_on_failure() {
    local status=$?
    if [ "${status}" -ne 0 ] && command -v adb >/dev/null 2>&1; then
        echo "Android logcat tail after e2e failure:"
        adb logcat -d -t 500 || true
    fi
    exit "${status}"
}

trap dump_logcat_on_failure EXIT

run_instrumentation() {
    local tests="$1"
    local output
    output="$(adb shell am instrument -w -r -e class "${tests}" "${TEST_RUNNER}")"
    printf '%s\n' "${output}"
    grep -Eq '^OK \([0-9]+ tests?\)$' <<< "${output}"
}

rm -rf "${JNI_LIBS_DIR}"
mkdir -p "${JNI_LIBS_DIR}"

cargo ndk \
    --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" \
    -t "${TARGET_ABI}" \
    -o "${JNI_LIBS_DIR}" \
    build -p nerust_android

(
    cd "${SCRIPT_DIR}"
    ./gradlew --no-daemon :app:assembleDebug :app:assembleDebugAndroidTest
)

adb uninstall "${TEST_PACKAGE}" >/dev/null 2>&1 || true
adb uninstall "${APP_PACKAGE}" >/dev/null 2>&1 || true
adb install "${APP_APK}"
adb install "${TEST_APK}"

run_instrumentation "${TEST_CLASS}#appLoadsGbcAndSupportsDrawerDialogsAndMenuActions"
adb shell am force-stop "${APP_PACKAGE}"
run_instrumentation "${TEST_CLASS}#gbcDocumentUriRestoresAfterProcessRestart"
run_instrumentation "${TEST_CLASS}#runtimeMeetsMinimumSupportedApi,${TEST_CLASS}#romPickerIntentUsesSafPersistableReadAccess,${TEST_CLASS}#directoryPickerIntentUsesPersistableReadWriteAccess,${TEST_CLASS}#portraitControlsOverlayMatchesExpectedArrangement,${TEST_CLASS}#landscapeControlsStayInsideSafeScreenBounds"
