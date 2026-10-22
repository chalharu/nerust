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

# Fail fast when an instrumentation test is never selected: an unlisted
# test would otherwise pass CI by not running.
python3 "${SCRIPT_DIR}/check-test-inventory.py" e2e

dump_logcat_on_failure() {
    local status=$?
    if [[ "${status}" -ne 0 ]] && command -v adb >/dev/null 2>&1; then
        echo "Android logcat tail after e2e failure:"
        adb logcat -d -t 500 || true
    fi
    exit "${status}"
}

trap dump_logcat_on_failure EXIT

run_instrumentation() {
    local tests="$1"
    local output
    output="$(adb shell am instrument -w -r -e class "${tests}" "${TEST_RUNNER}" 2>&1 || true)"
    printf '%s\n' "${output}"
    if instrumentation_ok <<< "${output}"; then
        return 0
    fi
    instrumentation_crashed_without_failure "${output}"
}

# An `OK (N tests)` summary line: the only passing shape.
instrumentation_ok() {
    grep -Eq '^OK \([0-9]+ tests?\)$'
}

# A hard failure: assertion text or an instrumentation error status.
instrumentation_failed() {
    local output="$1"
    grep -Eq 'FAILURES!!!|AssertionError|INSTRUMENTATION_STATUS_CODE: -2' <<< "${output}"
}

# A process crash with no recorded failure: infra flake, not a test failure.
instrumentation_crashed_without_failure() {
    local output="$1"
    grep -q 'INSTRUMENTATION_RESULT: shortMsg=Process crashed.' <<< "${output}" &&
        grep -q 'INSTRUMENTATION_STATUS_CODE: 0' <<< "${output}" &&
        ! instrumentation_failed "${output}"
}

# Poll for process exit with a deadline instead of a single snapshot.
wait_until_gone() {
    wait_until_gone_with_timeout 10
}

# Bounded variant with an explicit grace period in seconds.
wait_until_gone_with_timeout() {
    local deadline=$((SECONDS + $1))
    while ((SECONDS < deadline)); do
        if ! adb shell pidof "${APP_PACKAGE}" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.5
    done
    return 1
}

expect_process_termination() {
    local test="$1"
    local output
    output="$(adb shell am instrument -w -r -e class "${test}" "${TEST_RUNNER}" 2>&1 || true)"
    printf '%s\n' "${output}"
    if instrumentation_failed "${output}"; then
        # The assertion already failed, but still distinguish a slow shutdown
        # from a hung one: stacks are only captured while the process is alive.
        if wait_until_gone_with_timeout 30; then
            echo "process exited after test failure (slow shutdown, see timestamps above)"
        else
            dump_shutdown_threads
        fi
        return 1
    fi
    if wait_until_gone; then
        return 0
    fi
    dump_shutdown_threads
    return 1
}

# A lingering process means shutdown hung: capture thread stacks while alive.
# Failure path only; the SIGQUIT dump lands in logcat for the next triage.
dump_shutdown_threads() {
    local pid
    pid="$(adb shell pidof "${APP_PACKAGE}" 2>/dev/null | tr -d '\r' || true)"
    if [[ -n "${pid}" ]]; then
        echo "App process ${pid} survived termination; dumping threads:"
        adb shell kill -QUIT "${pid}" || true
        # Wait for the stacks to flush instead of a magic sleep.
        local deadline=$((SECONDS + 10))
        while ((SECONDS < deadline)); do
            if adb logcat -d 2>/dev/null | grep -q "Cmd line: ${APP_PACKAGE}"; then
                break
            fi
            sleep 0.5
        done
    fi
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
expect_process_termination "${TEST_CLASS}#backTerminatesApplicationProcess"
run_instrumentation "${TEST_CLASS}#appLaunchesAfterTermination"
expect_process_termination "${TEST_CLASS}#exitTerminatesApplicationProcess"
run_instrumentation "${TEST_CLASS}#appLaunchesAfterTermination"
run_instrumentation "${TEST_CLASS}#runtimeMeetsMinimumSupportedApi,${TEST_CLASS}#romPickerIntentUsesSafPersistableReadAccess,${TEST_CLASS}#directoryPickerIntentUsesPersistableReadWriteAccess,${TEST_CLASS}#pushedZonesParseToDrawableRects,${TEST_CLASS}#pushedJoystickParsesHomeGeometry,${TEST_CLASS}#delayedDismissCannotAcknowledgeReplacementDialog"
