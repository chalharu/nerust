#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 is required (cargo metadata target_directory parsing)" >&2
    exit 1
fi
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" --format-version 1 --no-deps 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"

if [[ -z "${TAG_NAME:-}" ]]; then
    TAG_NAME="$(git -C "${WORKSPACE_ROOT}" describe --tags --abbrev=0 2>/dev/null || true)"
    if [[ -z "${TAG_NAME}" ]]; then
        # Only real releases need a reproducible version: tag builds
        # export TAG_NAME explicitly, PR/CI snapshots use the fallback.
        if [[ "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
            echo "release builds must export TAG_NAME (no git tag found)" >&2
            exit 1
        fi
        # Non-reproducible dev fallback: release builds must export TAG_NAME.
        echo "warning: no git tag found; using non-reproducible version v0.1.0" >&2
        TAG_NAME="v0.1.0"
    fi
fi
if [[ -z "${TARGET_DIR}" ]]; then
    echo "cannot resolve target directory: cargo metadata failed and CARGO_TARGET_DIR is unset" >&2
    exit 1
fi
OUT_DIR="${OUT_DIR:-${TARGET_DIR}/dist}"
JNI_LIBS_DIR="${SCRIPT_DIR}/app/src/main/jniLibs"
APK_SRC="${SCRIPT_DIR}/app/build/outputs/apk/release/app-release.apk"
APK_NAME="nerust-${TAG_NAME}-android-arm64-v8a.apk"
SIGNING_TEMP_DIR=""
export CARGO_NDK_PLATFORM="${CARGO_NDK_PLATFORM:-28}"

cleanup() {
    if [[ -n "${SIGNING_TEMP_DIR:-}" && -d "${SIGNING_TEMP_DIR}" ]]; then
        rm -rf "${SIGNING_TEMP_DIR}"
    fi
}

cleanup_staging() {
    if [[ -n "${STAGE_JNI_LIBS:-}" && -d "${STAGE_JNI_LIBS}" ]]; then
        rm -rf "${STAGE_JNI_LIBS}"
    fi
    cleanup
}
trap cleanup_staging EXIT

prepare_signing_keystore() {
    if [[ -n "${ANDROID_KEY_STORE_PATH:-}" ]]; then
        return
    fi

    if [[ -z "${ANDROID_CERTIFICATE:-}" && -z "${ANDROID_PRIVATE_KEY:-}" ]]; then
        return
    fi

    if [[ -z "${ANDROID_CERTIFICATE:-}" || -z "${ANDROID_PRIVATE_KEY:-}" ]]; then
        echo "ANDROID_CERTIFICATE and ANDROID_PRIVATE_KEY must be set together" >&2
        exit 1
    fi

    command -v openssl >/dev/null 2>&1 || {
        echo "openssl is required to generate a temporary Android signing keystore" >&2
        exit 1
    }
    command -v keytool >/dev/null 2>&1 || {
        echo "keytool is required to generate a temporary Android signing keystore" >&2
        exit 1
    }

    SIGNING_TEMP_DIR="$(mktemp -d)"
    local alias="${ANDROID_KEY_ALIAS:-nerust-upload}"
    local store_password="${ANDROID_KEY_STORE_PASSWORD:-$(openssl rand -hex 24)}"
    local key_password="${ANDROID_KEY_PASSWORD:-$store_password}"
    local certificate_file="${SIGNING_TEMP_DIR}/certificate.pem"
    local private_key_file="${SIGNING_TEMP_DIR}/private-key.pem"
    local pkcs12_file="${SIGNING_TEMP_DIR}/signing.p12"
    local jks_file="${SIGNING_TEMP_DIR}/signing.jks"
    local -a passin_args=()

    printf '%s\n' "${ANDROID_CERTIFICATE}" > "${certificate_file}"
    printf '%s\n' "${ANDROID_PRIVATE_KEY}" > "${private_key_file}"

    if [[ -n "${ANDROID_PRIVATE_KEY_PASSWORD:-}" ]]; then
        passin_args=(-passin "env:ANDROID_PRIVATE_KEY_PASSWORD")
    fi

    openssl pkcs12 \
        -export \
        -name "${alias}" \
        -inkey "${private_key_file}" \
        -in "${certificate_file}" \
        -out "${pkcs12_file}" \
        -passout "pass:${store_password}" \
        "${passin_args[@]}"

    keytool \
        -importkeystore \
        -srckeystore "${pkcs12_file}" \
        -srcstoretype PKCS12 \
        -srcstorepass "${store_password}" \
        -destkeystore "${jks_file}" \
        -deststoretype JKS \
        -deststorepass "${store_password}" \
        -destkeypass "${key_password}" \
        -alias "${alias}" \
        -noprompt \
        >/dev/null

    export ANDROID_KEY_STORE_PATH="${jks_file}"
    export ANDROID_KEY_ALIAS="${alias}"
    export ANDROID_KEY_STORE_PASSWORD="${store_password}"
    export ANDROID_KEY_PASSWORD="${key_password}"
}

echo "Packaging Android APK"
echo "  output : ${OUT_DIR}/${APK_NAME}"

# Stage native libs outside the tree: a failed build must not leave
# a half-populated jniLibs behind (stale .so would ship silently).
STAGE_JNI_LIBS="$(mktemp -d)"
# Single EXIT trap (cleanup_staging covers both temp dirs).
mkdir -p "${STAGE_JNI_LIBS}"

prepare_signing_keystore

# AGP's strip task silently skips without a resolved NDK ("Unable to
# strip ... packaging them as they are" -> 156MB APK incident). Resolve
# the same side-by-side NDK cargo-ndk uses and fail loudly when absent.
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
    if [[ -n "${sdk_root}" && -d "${sdk_root}/ndk" ]]; then
        latest_ndk="$(find "${sdk_root}/ndk" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; 2>/dev/null | sort -V 2>/dev/null | tail -n 1)"
        if [[ -z "${latest_ndk}" ]]; then
            latest_ndk="$(find "${sdk_root}/ndk" -mindepth 1 -maxdepth 1 -type d -exec basename {} \; 2>/dev/null | sort | tail -n 1)"
        fi
        if [[ -n "${latest_ndk}" ]]; then
            export ANDROID_NDK_HOME="${sdk_root}/ndk/${latest_ndk}"
        fi
    fi
fi
if [[ -z "${ANDROID_NDK_HOME:-}" || ! -d "${ANDROID_NDK_HOME}" ]]; then
    echo "Android NDK not found: set ANDROID_NDK_HOME or install a side-by-side NDK" >&2
    exit 1
fi
echo "Using Android NDK: ${ANDROID_NDK_HOME}"

cargo ndk \
    --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" \
    -t arm64-v8a \
    -o "${STAGE_JNI_LIBS}" \
    build -p nerust_android --release

# Loud gate against silent native bloat: DWARF must be gone and each
# release .so must stay within its per-ABI budget (thin-LTO DWARF
# incident: 133MB). Resolves llvm-readelf from the NDK first, then PATH;
# warns and skips the gate when no reader exists off CI, fails hard when
# CI needs it. Gates the *staged* tree; the in-tree jniLibs update only
# after every .so passes.
# Per-ABI native library budget in bytes. Only arm64-v8a ships today;
# extend the table (not the gate logic) when new ABIs land.
native_budget_bytes() {
    printf '%s' "${BUDGET_BYTES:-41943040}"
}
readelf_bin=""
# Array expansion keeps NDK paths with spaces intact; the unquoted
# glob below would word-split them.
shopt -s nullglob
ndk_readelfs=("${ANDROID_NDK_HOME}"/toolchains/llvm/prebuilt/*/bin/llvm-readelf)
shopt -u nullglob
for cand in ${ndk_readelfs[@]+"${ndk_readelfs[@]}"}; do
    if [[ -x "${cand}" ]]; then
        readelf_bin="${cand}"
        break
    fi
done
if [[ -z "${readelf_bin}" ]] && command -v llvm-readelf >/dev/null 2>&1; then
    readelf_bin="llvm-readelf"
fi
if [[ -z "${readelf_bin}" ]] && command -v readelf >/dev/null 2>&1; then
    readelf_bin="readelf"
fi
if [[ -n "${SKIP_NATIVE_GATE:-}" ]]; then
    echo "warning: native DWARF/size gate skipped by request; a bloated .so will ship silently" >&2
elif [[ -z "${readelf_bin}" ]]; then
    # The gate exists because of the 156MB incident: a missing reader
    # off CI is a hole, not a pass. Require the explicit bypass there too.
    echo "no llvm-readelf/readelf found: cannot verify DWARF strip (set SKIP_NATIVE_GATE=1 to bypass explicitly)" >&2
    exit 1
else
    shopt -s nullglob
    so_files=("${STAGE_JNI_LIBS}"/*/libmain.so)
    shopt -u nullglob
    if (( ${#so_files[@]} == 0 )); then
        echo "no libmain.so produced under staging dir; refusing to ship stale jniLibs" >&2
        exit 1
    fi
    for main_so in "${so_files[@]}"; do
        abi="$(basename "$(dirname "${main_so}")")"
        budget="$(native_budget_bytes)"
        if "${readelf_bin}" -S "${main_so}" | grep -Eq '\.debug_(info|abbrev|line|str|ranges|addr|loclists?)|\.zdebug_'; then
            echo "${abi}/libmain.so contains DWARF sections; release strip is not effective" >&2
            exit 1
        fi
        so_size="$(wc -c < "${main_so}" | tr -d '[:space:]')"
        if (( so_size > budget )); then
            echo "${abi}/libmain.so is ${so_size} bytes, over the ${budget} budget (${abi})" >&2
            exit 1
        fi
        echo "${abi}/libmain.so: ${so_size} bytes, no DWARF"
    done
fi
# Gate passed (or deliberately skipped): publish the staged libs.
# Swap via a temp dir so an interrupted copy never leaves a
# half-populated jniLibs tree behind.
stage_publish_tmp="${JNI_LIBS_DIR}.new"
rm -rf "${stage_publish_tmp}"
mkdir -p "${stage_publish_tmp}"
cp -r "${STAGE_JNI_LIBS}/." "${stage_publish_tmp}/"
rm -rf "${JNI_LIBS_DIR}"
mv "${stage_publish_tmp}" "${JNI_LIBS_DIR}"

(
    cd "${SCRIPT_DIR}"
    ./gradlew --no-daemon assembleRelease
)


mkdir -p "${OUT_DIR}"
cp "${APK_SRC}" "${OUT_DIR}/${APK_NAME}"
sha256sum "${OUT_DIR}/${APK_NAME}" > "${OUT_DIR}/${APK_NAME}.sha256"
