#!/usr/bin/env bash
# Regression probes for publish-lock.sh; also wired into CI.
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

# A refused competitor keeps the live lock.
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
pass "refused competitor keeps the live lock"

# The pid-write window admits no second owner.
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
pass "pid-write window admits no second owner"

# A dead-pid lock is recovered promptly.
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
pass "dead-pid lock recovered without the grace wait"

# A pid-less lock is recovered after grace, then released.
LOCK4="${TMP}/nopid.lock"
mkdir "${LOCK4}"
if ! acquire_publish_lock "${LOCK4}"; then
    fail "pid-less lock was not recovered"
fi
[[ "$(cat "${LOCK4}/pid")" == "$$" ]] || fail "recovered lock names the wrong owner"
release_publish_lock
[[ ! -e "${LOCK4}" ]] || fail "normal release left the lock behind"
pass "pid-less lock recovered after grace and released"

# Release and cleanup stay idempotent and ownership-scoped.
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
pass "release and cleanup stay ownership-scoped"

# A stalled owner is never double-owned.
STALLOCK="${TMP}/stall.lock"
mkdir "${STALLOCK}"
( sleep 7; printf '%s' "$$" >"${STALLOCK}/pid.tmp" && mv "${STALLOCK}/pid.tmp" "${STALLOCK}/pid" ) &
STALLED_WRITER=$!
STALLED_ACQUIRED=0
if acquire_publish_lock "${STALLOCK}"; then
    STALLED_ACQUIRED=1
fi
wait "${STALLED_WRITER}" 2>/dev/null || true
if ((STALLED_ACQUIRED == 1)); then
    [[ "${PUBLISH_LOCK_OWNED}" == "1" ]] || fail "claimed without ownership flag"
    [[ "$(cat "${STALLOCK}/pid")" == "$$" ]] || fail "our pid record lost after stalled-owner race"
    release_publish_lock
    pass "B1 stalled owner: single ownership, our claim intact"
else
    [[ "${PUBLISH_LOCK_OWNED:-}" != "1" ]] || fail "refused but flagged owned"
    [[ -d "${STALLOCK}" ]] || fail "stalled owner's dir vanished after our refusal"
    rm -rf "${STALLOCK}"
    pass "B1 stalled owner: refused without double ownership"
fi

# Dual recoverers converge to one winner.
DUELLOCK="${TMP}/duel.lock"
mkdir "${DUELLOCK}"
echo "999999999" >"${DUELLOCK}/pid"
( rc=0; acquire_publish_lock "${DUELLOCK}" >/dev/null 2>&1 || rc=$?; echo "rc=${rc} owned=${PUBLISH_LOCK_OWNED:-0}" >"${TMP}/duel1" ) &
DP1=$!
( rc=0; acquire_publish_lock "${DUELLOCK}" >/dev/null 2>&1 || rc=$?; echo "rc=${rc} owned=${PUBLISH_LOCK_OWNED:-0}" >"${TMP}/duel2" ) &
DP2=$!
wait "$DP1" 2>/dev/null || true
wait "$DP2" 2>/dev/null || true
DUEL_WINNERS=0
[[ "$(cat "${TMP}/duel1")" == "rc=0 owned=1" ]] && DUEL_WINNERS=$((DUEL_WINNERS + 1))
[[ "$(cat "${TMP}/duel2")" == "rc=0 owned=1" ]] && DUEL_WINNERS=$((DUEL_WINNERS + 1))
[[ "${DUEL_WINNERS}" -eq 1 ]] || fail "dual recovery produced ${DUEL_WINNERS} owners"
SURVIVOR="$(cat "${DUELLOCK}/pid" 2>/dev/null || echo MISSING)"
[[ "${SURVIVOR}" =~ ^[0-9]+$ ]] || fail "no single surviving owner record"
pass "B2 dual recoverers converge to a single winner"

# A steal on a just-completed record yields back and refuses.
YIELDLOCK="${TMP}/yield.lock"
mkdir "${YIELDLOCK}"
sleep 30 &
SLEEPER=$!
mv() {
    if [[ "${1:-}" == "${YIELDLOCK}" && "${2:-}" == *.stale.* ]]; then
        printf '%s' "${SLEEPER}" >"${YIELDLOCK}/pid"
    fi
    command mv "$@"
}
if acquire_publish_lock "${YIELDLOCK}"; then
    unset -f mv
    kill "${SLEEPER}" 2>/dev/null || true
    fail "takeover stole a just-completed live record"
fi
unset -f mv
[[ -d "${YIELDLOCK}" ]] || fail "yield-back removed the live owner's dir"
[[ "$(cat "${YIELDLOCK}/pid")" == "${SLEEPER}" ]] || fail "victim pid record lost in yield-back"
[[ "${PUBLISH_LOCK_OWNED:-}" != "1" ]] || fail "refused but flagged owned"
[[ -z "$(ls -d "${YIELDLOCK}".stale.* 2>/dev/null)" ]] || fail "stale copy left behind"
kill "${SLEEPER}" 2>/dev/null || true
wait "${SLEEPER}" 2>/dev/null || true
pass "B4 completed record during takeover yields back and refuses"

# A directory replacement blocks the ownership claim.
VERIFYLOCK="${TMP}/verify.lock"
STAT_CALLS_FILE="${TMP}/statcalls"
echo 0 >"${STAT_CALLS_FILE}"
ORIG_DIR_ID="$(declare -f publish_lock_dir_id)"
publish_lock_dir_id() {
    declare -i n
    n=$(($(cat "${STAT_CALLS_FILE}") + 1))
    echo "$n" >"${STAT_CALLS_FILE}"
    if ((n >= 2)); then
        echo "replaced-dir-id"
    else
        local target="$1"
        command stat -c '%d:%i' "${target}" 2>/dev/null || echo "nostat"
    fi
}
if acquire_publish_lock "${VERIFYLOCK}"; then
    eval "$ORIG_DIR_ID"
    fail "claim survived a directory replacement"
fi
eval "$ORIG_DIR_ID"
[[ "${PUBLISH_LOCK_OWNED:-}" != "1" ]] || fail "refused but flagged owned"
pass "B5 replaced directory blocks the ownership claim"

echo "publish-lock probes: OK (${PASS} scenarios)"
echo "--- exclusion spans publish through consume ---"
# Holder publishes then keeps the lock while a consumer reads the tree.
SHARED="${TMP}/shared.lock"
SHARED_TREE="${TMP}/tree.txt"
echo "BUILD-A" >"${SHARED_TREE}"
acquire_publish_lock "${SHARED}" >/dev/null
# Simulate publish: the live tree now carries this run's libs.
echo "BUILD-A" >"${SHARED_TREE}"
# A concurrent publisher starts while the first run still consumes.
( acquire_publish_lock "${SHARED}" >/dev/null 2>&1 ) && fail "second publisher entered during consume"
[[ "$(cat "${SHARED_TREE}")" == "BUILD-A" ]] || fail "live tree changed under the consumer"
# The refusal keeps each consume on its own publish.
release_publish_lock
# After the full run, the next publisher proceeds normally.
acquire_publish_lock "${SHARED}" >/dev/null || fail "lock not reusable after release"
release_publish_lock
echo "ok: exclusion spans publish through consume"
echo "publish-lock probes: ALL OK"
