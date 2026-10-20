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

# Menu actions: BridgeContract.MenuAction is the single Kotlin source.
# Every Rust ACTION_* literal must resolve to it; no raw literals elsewhere.
rust_actions="$( (grep -oE 'ACTION_[A-Z_]+: &str = "[a-z_]+"' "${RUST_DIR}/src/bridge_events.rs" | grep -oE '"[a-z_]+"' | tr -d '"'; echo controller) | sort -u)"
kt_actions="$(sed -n '/object MenuAction/,/^}/p' "${KT_DIR}/BridgeContract.kt" | grep -oE 'const val [A-Z_]+ = "[a-z_]+"' | grep -oE '"[a-z_]+"' | tr -d '"' | sort -u)"
[[ "${rust_actions}" == "${kt_actions}" ]] \
    || fail "menu actions differ rust=[${rust_actions}] kotlin=[${kt_actions}]"
grep -q "MenuAction\." "${KT_DIR}/ChromeManager.kt" "${KT_DIR}/MainActivity.kt" \
    || fail "Kotlin senders must resolve menu actions through MenuAction"
if grep -rn --include='*.kt' -E 'menuEventJson\("|dispatchMenuAction\("' "${KT_DIR}" | grep -q .; then
    grep -rn --include='*.kt' -E 'menuEventJson\("|dispatchMenuAction\("' "${KT_DIR}" >&2
    fail "menu senders must resolve actions through MenuAction, never raw literals"
fi

# Controller keys: Rust table, JSON enum, and Kotlin ControllerKey stay identical.
rust_keys="$(grep -oE '"(button[1265]|start|select|up|down|left|right)"' "${RUST_DIR}/src/bridge_events.rs" | tr -d '"' | sort -u)"
json_keys="$(python3 -c 'import json,sys; print("\n".join(json.load(open(sys.argv[1]))["definitions"]["menuEvent"]["properties"]["key"]["enum"]))' "${RUST_DIR}/bridge_events.v1.json" | sort -u)"
kt_keys="$(grep -oE 'const val [A-Z0-9_]+ = "(button[1265]|start|select|up|down|left|right)"' "${KT_DIR}/BridgeContract.kt" | grep -oE '"[a-z0-9_]+"' | tr -d '"' | sort -u)"
[[ "${rust_keys}" == "${json_keys}" && "${json_keys}" == "${kt_keys}" ]] \
    || fail "controller keys differ rust=[${rust_keys}] json=[${json_keys}] kotlin=[${kt_keys}]"

# Ranged fields: Rust RangeSpec shape and Kotlin native Slider stay in sync.
grep -q '"kind": "choice"' "${RUST_DIR}/src/android/settings.rs" \
    || fail "Rust settings payload must tag choice fields with kind"
grep -q 'field\["kind"\] = serde_json::json!("range")' "${RUST_DIR}/src/android/settings.rs" \
    || fail "Rust settings payload must tag ranged fields with kind"
grep -q 'field\["min"\]' "${RUST_DIR}/src/android/settings.rs" \
    || fail "Rust settings payload must carry min/max/unit for ranged fields"
grep -q 'kind == "range"' "${KT_DIR}/MainActivity.kt" \
    || fail "Kotlin must branch on the pushed field kind"
grep -q 'field.getInt("min")' "${KT_DIR}/MainActivity.kt" \
    || fail "Kotlin must read the pushed min/max shape"
grep -q 'field.getInt("max")' "${KT_DIR}/MainActivity.kt" \
    || fail "Kotlin must read the pushed min/max shape"
grep -q 'Slider(' "${KT_DIR}/SettingsDialog.kt" \
    || fail "Kotlin must render ranged fields natively with a Slider"

# Settings sections: Rust owns id->label; Kotlin renders the pushed label.
rust_sections="$(sed -n '/pub(crate) const SETTINGS_SECTION_IDS/,/];/p' "${RUST_DIR}/src/bridge_events.rs" | grep -oE '"[a-z.]+"' | tr -d '"' | sort -u)"
for section in ${rust_sections}; do
    grep -q "\"${section}\"" "${KT_DIR}/BridgeContract.kt" \
        || fail "section ${section} missing from Kotlin fallback labels"
done
grep -q 'section.optString("label")' "${KT_DIR}/MainActivity.kt" \
    || fail "Kotlin must render the Rust-pushed section label"

echo "Android bridge contract: OK (bridge v${bridge_rs}, settings v${settings_rs}, chunk ${chunk_rs})"
