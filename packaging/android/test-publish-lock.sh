#!/usr/bin/env bash
# Regression probes for publish-lock.sh (NER-313). Manual run:
#   bash packaging/android/test-publish-lock.sh
# Not wired into CI (no shell runner); results are recorded on the ticket.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=publish-lock.sh
source "${SCRIPT_DIR}/publish-lock.sh"

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
PASS=0
fail() {
    echo "FAIL: $1" >&2
    exit 1
}
pass() {
    PASS=$((PASS + 1))
    echo "ok: $1"
}

# AC-1: a refused competitor must not delete the live owner's lock.
# NOTE: start the holder directly; a holder spawned inside $() dies with it.
sleep 30 &
HOLDER=$!
LOCK1="${TMP}/pub.lock"
mkdir "${LOCK1}"
echo "${HOLDER}" >"${LOCK1}/pid"
if acquire_publish_lock "${LOCK1}"; then
    kill "${HOLDER}" 2>/dev/null || true
    fail "competitor entered a live owner's lock"
fi
[[ -d "${LOCK1}" ]] || fail "live owner's lock was deleted by the refused competitor"
[[ "$(cat "${LOCK1}/pid")" == "${HOLDER}" ]] || fail "live owner's pid was overwritten"
kill "${HOLDER}" 2>/dev/null || true
wait "${HOLDER}" 2>/dev/null || true
# The refused competitor owns nothing: its cleanup must not delete either.
publish_lock_cleanup
[[ -d "${LOCK1}" ]] || fail "cleanup deleted a lock it never owned"
rm -rf "${LOCK1}"
pass "AC-1 refused competitor keeps the live lock"

# AC-2: mkdir-to-pid-record window never admits two owners.
LOCK2="${TMP}/win.lock"
mkdir "${LOCK2}"
( sleep 0.5; echo "$$" >"${LOCK2}/pid" ) &
WRITER=$!
if acquire_publish_lock "${LOCK2}"; then
    kill "${WRITER}" 2>/dev/null || true
    fail "competitor stole the lock during the pid-write window"
fi
wait "${WRITER}" 2>/dev/null || true
[[ -d "${LOCK2}" ]] || fail "window owner's lock was deleted while waiting"
rm -rf "${LOCK2}"
pass "AC-2 pid-write window admits no second owner"

# AC-3a: stale lock with a dead pid is recovered promptly.
LOCK3="${TMP}/stale.lock"
mkdir "${LOCK3}"
echo "999999999" >"${LOCK3}/pid"
START="$(date +%s)"
if ! acquire_publish_lock "${LOCK3}"; then
    fail "dead-pid lock was not recovered"
fi
ELAPSED=$(( $(date +%s) - START ))
[[ "${ELAPSED}" -lt 4 ]] || fail "dead-pid recovery waited the unknown grace (${ELAPSED}s)"
[[ "$(cat "${LOCK3}/pid")" == "$$" ]] || fail "recovered lock names the wrong owner"
pass "AC-3a dead-pid lock recovered without the grace wait"

# AC-3b: lock with no pid ever (crashed before recording) is recovered
# after the grace wait, then released normally.
LOCK4="${TMP}/nopid.lock"
mkdir "${LOCK4}"
if ! acquire_publish_lock "${LOCK4}"; then
    fail "pid-less lock was not recovered"
fi
[[ "$(cat "${LOCK4}/pid")" == "$$" ]] || fail "recovered lock names the wrong owner"
release_publish_lock
[[ ! -e "${LOCK4}" ]] || fail "normal release left the lock behind"
pass "AC-3b pid-less lock recovered after grace and released"

# AC-3c: release and cleanup are idempotent and ownership-scoped.
LOCK5="${TMP}/rel.lock"
acquire_publish_lock "${LOCK5}" >/dev/null
release_publish_lock
release_publish_lock
publish_lock_cleanup
[[ ! -e "${LOCK5}" ]] || fail "double release left residue"
PUBLISH_LOCK_DIR="${TMP}/foreign.lock"
mkdir "${PUBLISH_LOCK_DIR}"
PUBLISH_LOCK_OWNED=""
publish_lock_cleanup
[[ -d "${TMP}/foreign.lock" ]] || fail "cleanup deleted a foreign lock"
rm -rf "${TMP}/foreign.lock"
PUBLISH_LOCK_DIR=""
pass "AC-3c release and cleanup stay ownership-scoped"

echo "publish-lock probes: OK (${PASS} scenarios)"
