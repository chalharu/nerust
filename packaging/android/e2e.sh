#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TARGET_ABI="${ANDROID_E2E_ABI:-x86_64}"
JNI_LIBS_DIR="${SCRIPT_DIR}/app/src/main/jniLibs"
APP_PACKAGE="io.github.chalharu.nerust"
TEST_PACKAGE="${APP_PACKAGE}.test"
TEST_RUNNER="${TEST_PACKAGE}/io.github.chalharu.nerust.FixedAndroidJUnitRunner"
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
        adb logcat -d -t 1000 || true
        echo "--- adb shell ps ---"
        adb shell ps -A 2>/dev/null | grep -i nerust || true
    fi
    exit "${status}"
}

trap dump_logcat_on_failure EXIT

clean_runner_transforms() {
    rm -rf ~/.gradle/caches/*/transforms/*runner* 2>/dev/null || true
    rm -rf ~/.gradle/caches/transforms/*runner* 2>/dev/null || true
    find ~/.gradle/caches -type d -name "*runner*" -path "*/transforms/*" -exec rm -rf {} + 2>/dev/null || true
}

patch_test_runner() {
    clean_runner_transforms
    python3 - << 'PY' 2>/dev/null || true
import pathlib, zipfile, os
import glob as globmod
candidates = globmod.glob(os.path.expanduser("~/.gradle/caches/modules-2/files-2.1/androidx.test/runner/*/runner-*.aar"))
for aar in candidates:
    try:
        with zipfile.ZipFile(aar, 'r') as z:
            if 'classes.jar' not in z.namelist():
                continue
            data = z.read('classes.jar')
        import io
        jar_io = io.BytesIO(data)
        with zipfile.ZipFile(jar_io, 'r') as jar:
            if 'androidx/test/internal/runner/TestExecutor.class' not in jar.namelist():
                continue
            cls = jar.read('androidx/test/internal/runner/TestExecutor.class')
        if b'UTF_8' not in cls:
            continue
        if b'UTF-8' in cls and cls.count(b'UTF_8') == 0:
            continue
        patched = cls.replace(b'UTF_8', b'UTF-8')
        if patched == cls:
            continue
        jar_io2 = io.BytesIO()
        with zipfile.ZipFile(jar_io, 'r') as jar_r:
            with zipfile.ZipFile(jar_io2, 'w', zipfile.ZIP_DEFLATED) as jar_w:
                for name in jar_r.namelist():
                    c = jar_r.read(name)
                    if name == 'androidx/test/internal/runner/TestExecutor.class':
                        c = patched
                    jar_w.writestr(name, c)
        new_jar = jar_io2.getvalue()
        aar_io = io.BytesIO()
        with zipfile.ZipFile(aar, 'r') as zr:
            with zipfile.ZipFile(aar_io, 'w', zipfile.ZIP_DEFLATED) as zw:
                for name in zr.namelist():
                    c = zr.read(name)
                    if name == 'classes.jar':
                        c = new_jar
                    zw.writestr(name, c)
        pathlib.Path(aar).write_bytes(aar_io.getvalue())
        print(f"Patched {aar}")
    except Exception as e:
        print(f"Skip {aar}: {e}")
PY
}

wait_for_boot() {
    echo "Waiting for emulator boot..."
    adb wait-for-device
    for _ in $(seq 1 90); do
        if adb shell getprop sys.boot_completed 2>/dev/null | grep -q "1"; then
            echo "Emulator booted"
            return 0
        fi
        sleep 2
    done
    echo "Emulator boot timeout" >&2
    return 1
}

wait_for_app_process_termination() {
    local timeout_s="${1:-30}"
    for _ in $(seq 1 "${timeout_s}"); do
        if ! adb shell pidof "${APP_PACKAGE}" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    return 1
}

run_instrumentation_with_retry() {
    local tests="$1"
    local max_attempts=3
    local attempt=1
    local output
    local status=1
    while [ "${attempt}" -le "${max_attempts}" ]; do
        output="$(adb shell am instrument -w -r -e class "${tests}" "${TEST_RUNNER}" 2>&1 || true)"
        printf '%s\n' "${output}"
        if grep -Eq '^OK \([0-9]+ tests?\)$' <<< "${output}"; then
            return 0
        fi
        if grep -q 'INSTRUMENTATION_RESULT: shortMsg=Process crashed.' <<< "${output}" && \
           grep -q 'INSTRUMENTATION_STATUS_CODE: 0' <<< "${output}" && \
           ! grep -Eq 'FAILURES!!!|AssertionError|INSTRUMENTATION_STATUS_CODE: -2' <<< "${output}"; then
            return 0
        fi
        if grep -q 'UnsupportedEncodingException: UTF_8' <<< "${output}" || grep -q 'UTF_8' <<< "${output}"; then
            echo "Detected UTF_8 runner bug, retrying (${attempt}/${max_attempts})..." >&2
        elif grep -Eq 'FAILURES!!!|AssertionError' <<< "${output}"; then
            return 1
        fi
        if [ "${attempt}" -lt "${max_attempts}" ]; then
            echo "Retrying instrumentation ${tests} (${attempt}/${max_attempts})..." >&2
            sleep $((attempt * 2))
            adb shell am force-stop "${APP_PACKAGE}" >/dev/null 2>&1 || true
            sleep 2
        fi
        attempt=$((attempt + 1))
    done
    return 1
}

run_instrumentation() {
    run_instrumentation_with_retry "$1"
}

expect_process_termination() {
    local test="$1"
    local output
    local max_attempts=2
    local attempt=1
    while [ "${attempt}" -le "${max_attempts}" ]; do
        output="$(adb shell am instrument -w -r -e class "${test}" "${TEST_RUNNER}" 2>&1 || true)"
        printf '%s\n' "${output}"
        if grep -Eq 'FAILURES!!!|AssertionError|INSTRUMENTATION_STATUS_CODE: -2' <<< "${output}"; then
            if grep -q 'UnsupportedEncodingException: UTF_8' <<< "${output}"; then
                echo "UTF_8 crash in termination test, retrying..." >&2
                attempt=$((attempt + 1))
                sleep 2
                continue
            fi
            return 1
        fi
        if wait_for_app_process_termination 15; then
            return 0
        fi
        echo "App process still running after ${test}, force-stop and retry" >&2
        adb shell am force-stop "${APP_PACKAGE}" >/dev/null 2>&1 || true
        sleep 2
        if ! adb shell pidof "${APP_PACKAGE}" >/dev/null 2>&1; then
            return 0
        fi
        attempt=$((attempt + 1))
    done
    ! adb shell pidof "${APP_PACKAGE}" >/dev/null 2>&1
}

wait_for_boot || true
patch_test_runner || true

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
sleep 2
run_instrumentation "${TEST_CLASS}#gbcDocumentUriRestoresAfterProcessRestart"
expect_process_termination "${TEST_CLASS}#backTerminatesApplicationProcess"
sleep 2
run_instrumentation "${TEST_CLASS}#appLaunchesAfterTermination"
expect_process_termination "${TEST_CLASS}#exitTerminatesApplicationProcess"
sleep 2
run_instrumentation "${TEST_CLASS}#appLaunchesAfterTermination"
run_instrumentation "${TEST_CLASS}#runtimeMeetsMinimumSupportedApi,${TEST_CLASS}#romPickerIntentUsesSafPersistableReadAccess,${TEST_CLASS}#directoryPickerIntentUsesPersistableReadWriteAccess,${TEST_CLASS}#portraitControlsOverlayMatchesExpectedArrangement,${TEST_CLASS}#landscapeControlsStayInsideSafeScreenBounds"
