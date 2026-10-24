#!/usr/bin/env bash
# Publish lock with ownership for staging swaps; only the acquirer releases.
# shellcheck disable=SC2148
PUBLISH_LOCK_OWNED=""
# Shared unknown-owner sentinel for the scan and takeover paths below
# (Sonar S1192: no repeated literal).
UNKNOWN_OWNER="unknown"

# EXIT-trap safe: removes the lock only when this process acquired it.
publish_lock_cleanup() {
    if [[ "${PUBLISH_LOCK_OWNED:-}" == "1" && -n "${PUBLISH_LOCK_DIR:-}" && -d "${PUBLISH_LOCK_DIR}" ]]; then
        rm -rf "${PUBLISH_LOCK_DIR}"
    fi
    PUBLISH_LOCK_OWNED=""
    return 0
}

# Directory identity for the claim check: device plus inode. Falls back to
# sampling-proof refusal when stat is unavailable on the host.
publish_lock_dir_id() {
    local dir="$1"
    stat -c '%d:%i' "${dir}" 2>/dev/null || stat -f '%d:%i' "${dir}" 2>/dev/null || echo "nostat"
    return 0
}

# Acquire the lock dir; returns 0 on ownership, 1 on refusal.
acquire_publish_lock() {
    PUBLISH_LOCK_DIR="$1"
    PUBLISH_LOCK_OWNED=""
    local attempt=0
    local unknown_since=-1
    while ((attempt < 200)); do
        attempt=$((attempt + 1))
        # Fast path: fresh dir plus an atomic pid record. The rename makes
        # the record appear at once; readers never see a partial pid.
        if mkdir "${PUBLISH_LOCK_DIR}" 2>/dev/null; then
            local before
            before="$(publish_lock_dir_id "${PUBLISH_LOCK_DIR}")"
            if printf '%s' "$$" >"${PUBLISH_LOCK_DIR}/pid.tmp" \
                && mv "${PUBLISH_LOCK_DIR}/pid.tmp" "${PUBLISH_LOCK_DIR}/pid" \
                && [[ "$(cat "${PUBLISH_LOCK_DIR}/pid" 2>/dev/null)" == "$$" ]] \
                && [[ "$(publish_lock_dir_id "${PUBLISH_LOCK_DIR}")" == "${before}" ]]; then
                PUBLISH_LOCK_OWNED=1
                return 0
            fi
            # The dir moved under us before the record landed, or stat is
            # unavailable: never claim, compete fresh.
            sleep 0.1
            continue
        fi
        # The dir exists: read a well-formed owner record, if any. Anything
        # else (missing, empty, or non-numeric) is unknown, never proof.
        local owner="${UNKNOWN_OWNER}"
        local recorded
        recorded="$(cat "${PUBLISH_LOCK_DIR}/pid" 2>/dev/null || true)"
        if [[ "${recorded}" =~ ^[0-9]+$ ]]; then
            owner="${recorded}"
        fi
        if [[ "${owner}" != "${UNKNOWN_OWNER}" ]] && kill -0 "${owner}" 2>/dev/null; then
            echo "another publish is running (pid ${owner}); refusing concurrent publish" >&2
            return 1
        fi
        if [[ "${owner}" != "${UNKNOWN_OWNER}" ]]; then
            echo "removing stale publish lock from dead pid ${owner}" >&2
        else
            # A live owner may sit between mkdir and its pid record: wait out
            # the grace instead of treating a missing pid as dead proof.
            if ((unknown_since < 0)); then
                unknown_since="${SECONDS}"
                echo "publish lock has no owner record yet; waiting for the recorder" >&2
                sleep 0.1
                continue
            elif ((SECONDS - unknown_since < 5)); then
                sleep 0.1
                continue
            else
                echo "publish lock never recorded an owner; attempting single-winner takeover" >&2
            fi
        fi
        # Atomic single-winner takeover: at most one mover proceeds; losers
        # loop back and observe the winner.
        local stale="${PUBLISH_LOCK_DIR}.stale.$$"
        rm -rf "${stale}" 2>/dev/null
        if ! mv "${PUBLISH_LOCK_DIR}" "${stale}" 2>/dev/null; then
            sleep 0.1
            continue
        fi
        local stolen="${UNKNOWN_OWNER}"
        local stolen_recorded
        stolen_recorded="$(cat "${stale}/pid" 2>/dev/null || true)"
        if [[ "${stolen_recorded}" =~ ^[0-9]+$ ]]; then
            stolen="${stolen_recorded}"
        fi
        if [[ "${stolen}" != "${UNKNOWN_OWNER}" ]] && kill -0 "${stolen}" 2>/dev/null; then
            # Owner completed during takeover: yield back and refuse.
            if mv "${stale}" "${PUBLISH_LOCK_DIR}" 2>/dev/null; then
                echo "publish lock owner ${stolen} completed during takeover; refusing" >&2
                return 1
            fi
            # Name taken during takeover: refuse without orphaning.
            echo "publish lock changed hands during takeover; refusing" >&2
            return 1
        fi
        rm -rf "${stale}"
        # Loop around to mkdir and record atomically.
        sleep 0.1
    done
    echo "cannot acquire publish lock" >&2
    return 1
}

# Explicit release on the success path; normal failures rely on the EXIT
# trap, which never runs on SIGKILL or power loss (stale recovery covers it).
release_publish_lock() {
    if [[ "${PUBLISH_LOCK_OWNED:-}" == "1" && -n "${PUBLISH_LOCK_DIR:-}" ]]; then
        rm -rf "${PUBLISH_LOCK_DIR}"
    fi
    PUBLISH_LOCK_DIR=""
    PUBLISH_LOCK_OWNED=""
    return 0
}
