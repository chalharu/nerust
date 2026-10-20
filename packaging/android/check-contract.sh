#!/usr/bin/env bash
# Assert the Android bridge contract stays single-sourced across languages.
# Rust owns the numbers; Kotlin mirrors them from BridgeContract.kt.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="${SCRIPT_DIR}/../../gui/frontends/android"
KT_DIR="${SCRIPT_DIR}/app/src/main/kotlin/io/github/chalharu/nerust"

fail() {
    echo "contract mismatch: $1" >&2
    exit 1
}

rust_const() {
    local name="$1"
    local file="$2"
    grep -oE "const ${name}[^=]*= *[0-9]+" "${file}" | grep -oE '[0-9]+$'
}

bridge_rs="$(rust_const BRIDGE_EVENT_VERSION "${RUST_DIR}/src/bridge_events.rs")"
bridge_kt="$(grep -oE 'const val BRIDGE_EVENT_VERSION = [0-9]+' "${KT_DIR}/BridgeContract.kt" | grep -oE '[0-9]+')"
bridge_json="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["definitions"]["menuEvent"]["properties"]["v"]["const"])' "${RUST_DIR}/bridge_events.v1.json")"
[[ "${bridge_rs}" == "${bridge_kt}" && "${bridge_kt}" == "${bridge_json}" ]] \
    || fail "BRIDGE_EVENT_VERSION rust=${bridge_rs} kotlin=${bridge_kt} json=${bridge_json}"

settings_rs="$(rust_const SETTINGS_SCHEMA_VERSION "${RUST_DIR}/src/android/settings.rs")"
settings_kt="$(grep -oE 'const val SETTINGS_SCHEMA_VERSION = [0-9]+' "${KT_DIR}/BridgeContract.kt" | grep -oE '[0-9]+')"
[[ "${settings_rs}" == "${settings_kt}" ]] \
    || fail "SETTINGS_SCHEMA_VERSION rust=${settings_rs} kotlin=${settings_kt}"

chunk_rs="$(grep -oE 'SAF_CHUNK_BYTES: usize = [0-9 ]+\* *[0-9]+' "${RUST_DIR}/src/android/saf.rs" | python3 -c 'import sys,re; print(eval(re.search(r"[0-9 ]+\* *[0-9]+", sys.stdin.read()).group()))')"
chunk_kt="$(grep -oE 'SAF_CHUNK_BYTES = [0-9 ]+\* *[0-9]+' "${KT_DIR}/SafRepository.kt" | python3 -c 'import sys,re; print(eval(re.search(r"[0-9 ]+\* *[0-9]+", sys.stdin.read()).group()))')"
[[ "${chunk_rs}" == "${chunk_kt}" ]] \
    || fail "SAF_CHUNK_BYTES rust=${chunk_rs} kotlin=${chunk_kt}"

# Menu action names: every Rust ACTION_* literal must appear in a Kotlin sender.
while IFS= read -r action; do
    grep -q "\"${action}\"" "${KT_DIR}/ChromeManager.kt" "${KT_DIR}/MainActivity.kt" \
        || fail "menu action ${action} missing from Kotlin senders"
done < <(grep -oE 'ACTION_[A-Z_]+: &str = "[a-z_]+"' "${RUST_DIR}/src/android/menu.rs" | grep -oE '"[a-z_]+"' | tr -d '"')

echo "Android bridge contract: OK (bridge v${bridge_rs}, settings v${settings_rs}, chunk ${chunk_rs})"
