#!/usr/bin/env bash
# Regression probes for e2e.sh shutdown detection (NER-327). Manual run:
#   bash packaging/android/test-e2e-wait.sh
# Stubs adb and drives the real wait/evidence helpers.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=e2e.sh
source "${SCRIPT_DIR}/e2e.sh"
trap - EXIT

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
PASS=0
# Shared stub matcher: every adb stub below matches the same
# instrumentation call shape (Sonar S1192: no repeated literal).
INSTRUMENT_CMD="am instrument"
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
        *pidof*) echo "1234" ;;
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
# First pidof (pre-request evidence) reports alive, later ones report gone.
echo 0 >"${TMP}/genuine-pidof-calls"
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) printf 'INSTRUMENTATION_RESULT: shortMsg=Process crashed.\nINSTRUMENTATION_STATUS_CODE: 0\n' ;;
        *pidof*)
            n=$(($(cat "${TMP}/genuine-pidof-calls") + 1))
            echo "${n}" >"${TMP}/genuine-pidof-calls"
            if ((n == 1)); then echo "1234"; else return 1; fi ;;
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
        *pidof*) echo "error: device offline" >&2; return 1 ;;
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

# --- run path: OK summary still passes ---
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) printf 'INSTRUMENTATION_CODE: 1\nOK (2 tests)\n' ;;
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
        *pidof*) echo "error: closed" >&2; return 1 ;;
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
# The gate relaunches once; an unstartable process still fails.
export E2E_ENSURE_PROCESS_TIMEOUT=2
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) printf 'INSTRUMENTATION_RESULT: shortMsg=Process crashed.\nINSTRUMENTATION_STATUS_CODE: 0\n' ;;
        *pidof*) return 1 ;;
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

# --- relaunch when dead: force-stop aftermath still terminates ---
# First pidof misses (post-instrumentation force-stop), monkey starts the
# app, the poll sees it, the test reports OK, then the process is gone.
echo 0 >"${TMP}/relaunch-pidof-calls"
adb() {
    case "$*" in
        *"${INSTRUMENT_CMD}"*) printf 'INSTRUMENTATION_CODE: -1\nOK (1 test)\n' ;;
        *monkey*) return 0 ;;
        *pidof*)
            n=$(($(cat "${TMP}/relaunch-pidof-calls") + 1))
            echo "${n}" >"${TMP}/relaunch-pidof-calls"
            if ((n == 1)); then return 1; fi
            if ((n == 2)); then echo "1234"; return 0; fi
            return 1 ;;
        *) echo "unexpected adb call: $*" >&2; return 1 ;;
    esac
    return 0
}
export -f adb
if expect_process_termination "io.github.Fake#Relaunched" >/dev/null 2>&1; then
    pass "relaunch-when-dead still terminates"
else
    fail "relaunch-when-dead broke the termination gate"
fi
unset -f adb

echo "e2e wait probes: OK (${PASS} scenarios)" 
