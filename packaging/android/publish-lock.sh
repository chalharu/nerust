#!/usr/bin/env bash
# Publish lock with ownership for package.sh staging swaps.
# Only the acquirer releases, and an unwritten pid is never proof of death.
# Sourced by package.sh; the EXIT trap calls publish_lock_cleanup.
# shellcheck disable=SC2148
PUBLISH_LOCK_OWNED=""

# EXIT-trap safe: removes the lock only when this process acquired it.
publish_lock_cleanup() {
    if [[ "${PUBLISH_LOCK_OWNED:-}" == "1" && -n "${PUBLISH_LOCK_DIR:-}" && -d "${PUBLISH_LOCK_DIR}" ]]; then
        rm -rf "${PUBLISH_LOCK_DIR}"
    fi
    PUBLISH_LOCK_OWNED=""
    return 0
}

# Acquire the publish lock dir, recording this process as owner.
# A live owner refuses; a dead or never-recorded owner is taken over.
# Returns 0 on ownership, 1 on refusal or failure.
acquire_publish_lock() {
    PUBLISH_LOCK_DIR="$1"
    PUBLISH_LOCK_OWNED=""
    if mkdir "${PUBLISH_LOCK_DIR}" 2>/dev/null; then
        echo "$$" >"${PUBLISH_LOCK_DIR}/pid"
        PUBLISH_LOCK_OWNED=1
        return 0
    fi
    # A live owner may sit between mkdir and its pid write: wait for the
    # owner record instead of treating a missing pid as dead proof.
    local owner="unknown"
    for _ in $(seq 1 50); do
        owner="$(cat "${PUBLISH_LOCK_DIR}/pid" 2>/dev/null || echo unknown)"
        [[ "${owner}" != "unknown" ]] && break
        sleep 0.1
    done
    if [[ "${owner}" != "unknown" ]] && kill -0 "${owner}" 2>/dev/null; then
        echo "another publish is running (pid ${owner}); refusing concurrent publish" >&2
        return 1
    fi
    echo "removing stale publish lock from dead pid ${owner}" >&2
    rm -rf "${PUBLISH_LOCK_DIR}"
    if mkdir "${PUBLISH_LOCK_DIR}" 2>/dev/null; then
        echo "$$" >"${PUBLISH_LOCK_DIR}/pid"
        PUBLISH_LOCK_OWNED=1
        return 0
    fi
    echo "cannot acquire publish lock" >&2
    return 1
}

# Explicit release on the success path; failures rely on the EXIT trap.
release_publish_lock() {
    if [[ "${PUBLISH_LOCK_OWNED:-}" == "1" && -n "${PUBLISH_LOCK_DIR:-}" ]]; then
        rm -rf "${PUBLISH_LOCK_DIR}"
    fi
    PUBLISH_LOCK_DIR=""
    PUBLISH_LOCK_OWNED=""
    return 0
}
