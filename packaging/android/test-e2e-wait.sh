#!/usr/bin/env bash
# Regression probes for e2e.sh shutdown detection (NER-327). Manual run:
#   bash packaging/android/test-e2e-wait.sh
# Stubs adb and drives the real wait/evidence helpers.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=e2e.sh
source "${SCRIPT_DIR}/e2e.sh"
trap - EXIT

PASS=0
fail() {
    echo "FAIL: $1" >&2
    exit 1
}
pass() {
    PASS=$((PASS + 1))
    echo "ok: $1"
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
        *"am instrument"*) echo "error: device '(null)' not found" >&2; return 1 ;;
        *pidof*) return 1 ;;
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
adb() {
    case "$*" in
        *"am instrument"*) printf 'INSTRUMENTATION_RESULT: shortMsg=Process crashed.\nINSTRUMENTATION_STATUS_CODE: 0\n' ;;
        *pidof*) return 1 ;;
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
    esac
    return 0
}
export -f adb
if wait_until_gone_with_timeout 1 >/dev/null 2>&1; then
    fail "adb failure inside the wait passed"
else
    pass "adb failure inside the wait fails"
fi
unset -f adb

echo "e2e wait probes: OK (${PASS} scenarios)"
