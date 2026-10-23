#!/usr/bin/env bash
# Regression probes for e2e.sh shutdown detection (NER-327). Manual run:
#   bash packaging/android/test-e2e-wait.sh
# Wired into CI: check-contract.sh "shell probes" phase + the lint job's
# explicit shell-probes step in .github/workflows/ci.yml.
# NER-327: the same has_instrumentation_output / wait_until_gone /
# run_instrumentation gates below run against the real emulator in CI
# (.github/workflows/ci.yml `android-e2e` matrix API 28/35, which executes
# packaging/android/e2e.sh); the stub probes here prove the
# runner-missing shape fails the gate without needing a device.
# Stubs adb and drives the real wait/evidence helpers.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=e2e.sh
source "${SCRIPT_DIR}/e2e.sh"
trap - EXIT

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
PASS=0
# Shared stub matchers: every adb stub below matches the same
# instrumentation call shape (Sonar S1192: no repeated literal).
INSTRUMENT_CMD="am instrument"
PIDOF_MATCH="pidof"
FORCE_STOP_MATCH="am force-stop"
fail() {
    echo "FAIL: $1" >&2
    exit 1
}
pass() {
    PASS=$((PASS + 1))
    echo "ok: $1"
    return 0
}

# --- has_instrumentation_output ---
if has_instrumentation_output "INSTRUMENTATION_RESULT: shortMsg=Process crashed."; then
    pass "crash output counts as instrumentation output"
else
    fail "crash output not recognised"
fi
if has_instrumentation_output "INSTRUMENTATION_STATUS_CODE: 0"; then
    pass "status output counts"
else
    fail "status output not recognised"
fi
if has_instrumentation_output "error: device offline"; then
    fail "adb error counted as instrumentation output"
else
    pass "adb error is not instrumentation output"
fi
if has_instrumentation_output ""; then
    fail "empty output counted as instrumentation output"
else
    pass "empty output is not instrumentation output"
fi

# --- termination gate with stubbed adb: runner missing must fail ---
adb() {
    # shellcheck disable=SC2124
    case "$*" in
        *"${INSTRUMENT_CMD}"*) echo "error: device '(null)' not found" >&2; return 1 ;;
        *${FORCE_STOP_MATCH}*) return 0 ;;
        *${PIDOF_MATCH}*) echo "1234" ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
}
export -f adb
if expect_process_termination "io.github.Fake#missing" >/dev/null 2>&1; then
    fail "runner-missing termination passed the gate"
else
    pass "runner-missing termination fails the gate"
fi
unset -f adb

# --- termination gate with stubbed adb: genuine pass still passes ---
# Freshness sees a lingering process and force-stops it; the pre-request
# check relaunches it; the test runs; later pidofs report gone.
echo 0 >"${TMP}/genuine-pidof-calls"
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) printf 'INSTRUMENTATION_RESULT: shortMsg=Process crashed.\nINSTRUMENTATION_STATUS_CODE: 0\n' ;;
        *${FORCE_STOP_MATCH}*) return 0 ;;
        *monkey*) return 0 ;;
        *${PIDOF_MATCH}*)
            n=$(($(cat "${TMP}/genuine-pidof-calls") + 1))
            echo "${n}" >"${TMP}/genuine-pidof-calls"
            if ((n == 1 || n == 3)); then echo "1234"; else return 1; fi ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
    return 0
}
export -f adb
if expect_process_termination "io.github.Fake#gone" >/dev/null 2>&1; then
    pass "genuine termination still passes"
else
    fail "genuine termination no longer passes"
fi
unset -f adb

# --- adb failure inside the wait must fail, not pass ---
adb() {
    case "$*" in
        *${PIDOF_MATCH}*) echo "error: device offline" >&2; return 1 ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
}
export -f adb
if wait_until_gone_with_timeout 1 >/dev/null 2>&1; then
    fail "adb failure inside the wait passed"
else
    pass "adb failure inside the wait fails"
fi
unset -f adb

# --- run path: crash-only output is not a pass ---
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) printf 'INSTRUMENTATION_RESULT: shortMsg=Process crashed.\nINSTRUMENTATION_STATUS_CODE: 0\n' ;;
        *${FORCE_STOP_MATCH}*) return 0 ;;
        *${PIDOF_MATCH}*) return 1 ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
    return 0
}
export -f adb
if run_instrumentation "io.github.Fake#CrashOnly" >/dev/null 2>&1; then
    fail "crash-only run_instrumentation passed"
else
    pass "crash-only run_instrumentation fails"
fi
unset -f adb

# --- run path: runner-missing output fails, never converts to a pass ---
# NER-327 stub-path proof: the instrumentation call emits only adb's
# runner-missing error (no INSTRUMENTATION_* block), so the output gate
# must fail. The same gate runs against the real emulator in CI
# (`android-e2e` jobs), where a missing runner likewise fails e2e.sh.
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) echo "error: device '(null)' not found" >&2; return 1 ;;
        *${FORCE_STOP_MATCH}*) return 0 ;;
        *${PIDOF_MATCH}*) return 1 ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
}
export -f adb
if run_instrumentation "io.github.Fake#RunnerMissing" >/dev/null 2>&1; then
    fail "runner-missing run_instrumentation passed"
else
    pass "runner-missing run_instrumentation fails"
fi
unset -f adb

# --- run path: OK summary still passes ---
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) printf 'INSTRUMENTATION_CODE: 1\nOK (2 tests)\n' ;;
        *${FORCE_STOP_MATCH}*) return 0 ;;
        *${PIDOF_MATCH}*) return 1 ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
    return 0
}
export -f adb
if run_instrumentation "io.github.Fake#Ok" >/dev/null 2>&1; then
    pass "OK run_instrumentation still passes"
else
    fail "OK run_instrumentation no longer passes"
fi
unset -f adb

# --- run path: empty output fails, never converts to a pass ---
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) return 1 ;;
        *${FORCE_STOP_MATCH}*) return 0 ;;
        *${PIDOF_MATCH}*) return 1 ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
}
export -f adb
if run_instrumentation "io.github.Fake#Empty" >/dev/null 2>&1; then
    fail "empty run_instrumentation passed"
else
    pass "empty run_instrumentation fails"
fi
unset -f adb

# --- unknown adb failure text never converts to gone ---
adb() {
    case "$*" in
        *${PIDOF_MATCH}*) echo "error: closed" >&2; return 1 ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
}
export -f adb
if wait_until_gone_with_timeout 1 >/dev/null 2>&1; then
    fail "unknown adb failure passed as gone"
else
    pass "unknown adb failure text fails the wait"
fi
unset -f adb

# --- pre-dead process: a pre-existing crash plus absence proves nothing ---
# Freshness skips force-stop when nothing runs; the relaunch fails; the
# crash output without progress still fails the gate.
export E2E_ENSURE_PROCESS_TIMEOUT=2
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) printf 'INSTRUMENTATION_RESULT: shortMsg=Process crashed.\nINSTRUMENTATION_STATUS_CODE: 0\n' ;;
        *${FORCE_STOP_MATCH}*) return 0 ;;
        *monkey*) return 1 ;;
        *${PIDOF_MATCH}*) return 1 ;;
        *logcat*) printf 'no finished marker here\n' ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
    return 0
}
export -f adb
if expect_process_termination "io.github.Fake#PreDead" >/dev/null 2>&1; then
    fail "pre-dead termination passed without causing the exit"
else
    pass "pre-dead process fails the termination gate"
fi
unset -f adb
unset E2E_ENSURE_PROCESS_TIMEOUT

# --- freshness force-stops a lingering process before the test ---
# A dead event loop would be reused otherwise: pidof reports alive once,
# exactly one force-stop runs, the pre-request check relaunches, the
# test reports OK, then it is gone.
echo 0 >"${TMP}/fresh-force-stop-calls"
echo 0 >"${TMP}/fresh-pidof-calls"
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) printf 'INSTRUMENTATION_CODE: -1\nOK (1 test)\n' ;;
        *${FORCE_STOP_MATCH}*)
            n=$(($(cat "${TMP}/fresh-force-stop-calls") + 1))
            echo "${n}" >"${TMP}/fresh-force-stop-calls"
            return 0 ;;
        *monkey*) return 0 ;;
        *${PIDOF_MATCH}*)
            n=$(($(cat "${TMP}/fresh-pidof-calls") + 1))
            echo "${n}" >"${TMP}/fresh-pidof-calls"
            if ((n == 1 || n == 3)); then echo "1234"; else return 1; fi ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
    return 0
}
export -f adb
if expect_process_termination "io.github.Fake#Fresh" >/dev/null 2>&1; then
    pass "lingering process is cleared before the test"
else
    fail "freshness gate broke the termination path"
fi
if [[ $(cat "${TMP}/fresh-force-stop-calls") -eq 1 ]]; then
    pass "exactly one force-stop precedes the test"
else
    fail "force-stop count wrong: $(cat "${TMP}/fresh-force-stop-calls")"
fi
unset -f adb

# --- retry recovers a lost summary after proven progress ---
echo 0 >"${TMP}/retry-instrument-calls"
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*)
            n=$(($(cat "${TMP}/retry-instrument-calls") + 1))
            echo "${n}" >"${TMP}/retry-instrument-calls"
            if ((n == 1)); then
                printf 'INSTRUMENTATION_STATUS: test=FakeRetry\nINSTRUMENTATION_STATUS_CODE: 0\nINSTRUMENTATION_RESULT: shortMsg=Process crashed.\nINSTRUMENTATION_CODE: 0\n'
            else
                printf 'INSTRUMENTATION_CODE: -1\nOK (1 test)\n'
            fi ;;
        *logcat*) printf 'TestRunner: finished: FakeRetry(io.github.Fake)\n' ;;
        *${FORCE_STOP_MATCH}*) return 0 ;;
        *${PIDOF_MATCH}*) return 1 ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
    return 0
}
export -f adb
if run_instrumentation "io.github.Fake#FakeRetry" >/dev/null 2>&1; then
    pass "lost summary after progress recovers on retry"
else
    fail "lost summary after progress did not recover"
fi
if [[ $(cat "${TMP}/retry-instrument-calls") -eq 2 ]]; then
    pass "retry ran exactly once"
else
    fail "retry count wrong: $(cat "${TMP}/retry-instrument-calls")"
fi
unset -f adb

# --- crash without progress never retries ---
echo 0 >"${TMP}/noprogress-instrument-calls"
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*)
            n=$(($(cat "${TMP}/noprogress-instrument-calls") + 1))
            echo "${n}" >"${TMP}/noprogress-instrument-calls"
            printf 'INSTRUMENTATION_STATUS: test=FakeCrash\nINSTRUMENTATION_STATUS_CODE: 1\nINSTRUMENTATION_RESULT: shortMsg=Process crashed.\nINSTRUMENTATION_CODE: 0\n' ;;
        *logcat*) printf 'some unrelated log line\n' ;;
        *${FORCE_STOP_MATCH}*) return 0 ;;
        *${PIDOF_MATCH}*) return 1 ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
    return 0
}
export -f adb
if run_instrumentation "io.github.Fake#FakeCrash" >/dev/null 2>&1; then
    fail "crash without progress passed"
else
    pass "crash without progress fails"
fi
if [[ $(cat "${TMP}/noprogress-instrument-calls") -eq 1 ]]; then
    pass "no retry without progress"
else
    fail "retried without progress: $(cat "${TMP}/noprogress-instrument-calls")"
fi
unset -f adb

echo "e2e wait probes: OK (${PASS} scenarios)" 
