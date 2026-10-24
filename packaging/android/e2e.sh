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
# Abnormal SAF paths run against a stub provider on the emulator.
FAULT_CLASS="io.github.chalharu.nerust.SafAbnormalProviderTest"
# Marker keeps the process alive across teardown so the summary is delivered.
HARNESS_TEARDOWN_MARKER="/data/local/tmp/nerust-harness-teardown"
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
    local output rc=0
    ensure_fresh_process
    output="$(adb shell am instrument -w -r -e class "${tests}" "${TEST_RUNNER}" 2>&1)" || rc=$?
    printf '%s\n' "${output}"
    if ! has_instrumentation_output "${output}"; then
        echo "no instrumentation output: runner never started or adb offline (am exit ${rc})" >&2
        return 1
    fi
    if instrumentation_ok <<< "${output}"; then
        return 0
    fi
    # A hard failure fails immediately; a crash never excuses it.
    if instrumentation_failed "${output}"; then
        return 1
    fi
    # Retry once when the summary lost the race with process exit.
    if ! crashed_after_progress "${tests}"; then
        return 1
    fi
    echo "result delivery raced process exit; retrying once" >&2
    ensure_fresh_process
    output="$(adb shell am instrument -w -r -e class "${tests}" "${TEST_RUNNER}" 2>&1)" || rc=$?
    printf '%s\n' "${output}"
    if ! has_instrumentation_output "${output}"; then
        echo "no instrumentation output on retry: runner never started or adb offline (am exit ${rc})" >&2
        return 1
    fi
    instrumentation_ok <<< "${output}"
}

# True when logcat proves at least one requested test body finished:
# the summary loss, not the test, is what the retry recovers.
crashed_after_progress() {
    local tests="$1"
    local log=""
    # adbd can hiccup right after a process death; a single empty read
    # must not decide. Bounded refetch keeps the gate deterministic.
    local attempt
    for attempt in 1 2 3; do
        log="$(adb logcat -d -t 2000 2>/dev/null || true)"
        if [[ -n "${log}" ]]; then
            break
        fi
        sleep "${attempt}"
    done
    echo "logcat lines for progress check: $(printf '%s' "${log}" | wc -l)" >&2
    [[ -n "${log}" ]] || return 1
    local rest="${tests}" entry method
    while [[ -n "${rest}" ]]; do
        entry="${rest%%,*}"
        method="${entry##*#}"
        if grep -Fq "TestRunner: finished: ${method}(" <<< "${log}"; then
            echo "progress proven by ${method}; summary was lost" >&2
            return 0
        fi
        [[ "${rest}" == "${entry}" ]] && break
        rest="${rest#*,}"
    done
    return 1
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

# Poll for process exit with a deadline instead of a single snapshot.
wait_until_gone() {
    wait_until_gone_with_timeout 10
}

# Bounded variant with an explicit grace period in seconds.
wait_until_gone_with_timeout() {
    local grace_secs="$1"
    local deadline=$((SECONDS + grace_secs))
    while ((SECONDS < deadline)); do
        local out
        if ! out=$(adb shell pidof "${APP_PACKAGE}" 2>&1); then
            # Only a confirmed absence passes; adb failure text never means gone.
            if [[ -z "${out//[[:space:]]/}" ]]; then
                return 0
            fi
            echo "adb pidof failed while waiting for exit: ${out}" >&2
            return 1
        fi
        sleep 0.5
    done
    return 1
}

# Instrumentation output proves the runner started; absence proves nothing.
has_instrumentation_output() {
    local output="$1"
    grep -Eq 'INSTRUMENTATION_(RESULT|STATUS_CODE|CODE)' <<< "${output}"
}

# Fresh process per test: force-stop first so no dead loop is reused.
ensure_fresh_process() {
    local before
    if before="$(adb shell pidof "${APP_PACKAGE}" 2>&1)" && [[ -n "${before//[[:space:]]/}" ]]; then
        adb shell am force-stop "${APP_PACKAGE}" >/dev/null 2>&1 || true
    fi
}

# Start the process when missing; only a start failure proves a device problem.
ensure_process_running() {
    local before
    if before="$(adb shell pidof "${APP_PACKAGE}" 2>&1)" && [[ -n "${before//[[:space:]]/}" ]]; then
        return 0
    fi
    adb shell monkey -p "${APP_PACKAGE}" -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true
    local deadline=$((SECONDS + ${E2E_ENSURE_PROCESS_TIMEOUT:-30}))
    while ((SECONDS < deadline)); do
        if before="$(adb shell pidof "${APP_PACKAGE}" 2>&1)" && [[ -n "${before//[[:space:]]/}" ]]; then
            return 0
        fi
        sleep 0.5
    done
    echo "cannot confirm the process runs before the termination test" >&2
    return 1
}

expect_process_termination() {
    local test="$1"
    local output rc=0
    ensure_fresh_process
    ensure_process_running || return 1
    output="$(adb shell am instrument -w -r -e class "${test}" "${TEST_RUNNER}" 2>&1)" || rc=$?
    printf '%s\n' "${output}"
    if ! has_instrumentation_output "${output}"; then
        echo "no instrumentation output: runner never started or adb offline (am exit ${rc})" >&2
        return 1
    fi
    if instrumentation_failed "${output}"; then
        # Dump first: teardown force-stops the package as soon as the test
        # run ends, so a lingering process may already be gone below.
        dump_shutdown_threads
        if wait_until_gone_with_timeout 30; then
            echo "process gone after test failure (see force-stop note above)"
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

# Unit-probe hook: sourcing this file defines helpers without running the suite.
if [[ "${BASH_SOURCE[0]:-}" != "${0}" ]]; then
    return 0
fi

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

# Leave the teardown marker; missing marker fails fast instead of flaking.
adb shell "touch ${HARNESS_TEARDOWN_MARKER:?}" >/dev/null 2>&1
if ! adb shell "test -f ${HARNESS_TEARDOWN_MARKER}" >/dev/null 2>&1; then
    echo "cannot place the harness teardown marker; refusing flaky run" >&2
    exit 1
fi

run_instrumentation "${TEST_CLASS}#appLoadsGbcAndSupportsDrawerDialogsAndMenuActions"
adb shell am force-stop "${APP_PACKAGE}"
run_instrumentation "${TEST_CLASS}#gbcDocumentUriRestoresAfterProcessRestart"
expect_process_termination "${TEST_CLASS}#backTerminatesApplicationProcess"
run_instrumentation "${TEST_CLASS}#appLaunchesAfterTermination"
expect_process_termination "${TEST_CLASS}#exitTerminatesApplicationProcess"
run_instrumentation "${TEST_CLASS}#appLaunchesAfterTermination"
# One test per run: separate runs keep per-test isolation.
run_instrumentation "${TEST_CLASS}#runtimeMeetsMinimumSupportedApi"
run_instrumentation "${TEST_CLASS}#romPickerIntentUsesSafPersistableReadAccess"
run_instrumentation "${TEST_CLASS}#directoryPickerIntentUsesPersistableReadWriteAccess"
run_instrumentation "${TEST_CLASS}#pushedZonesParseToDrawableRects"
run_instrumentation "${TEST_CLASS}#pushedJoystickParsesHomeGeometry"
run_instrumentation "${TEST_CLASS}#delayedDismissCannotAcknowledgeReplacementDialog"
run_instrumentation "${FAULT_CLASS}#transientOutageIsScanErrorNeverMissing"
run_instrumentation "${FAULT_CLASS}#corruptDocumentReadsBackExactlyForQuarantine"
run_instrumentation "${FAULT_CLASS}#oversizedSidecarSummaryIsCappedOutOfListing"
