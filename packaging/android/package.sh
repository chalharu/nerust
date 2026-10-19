#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" --format-version 1 --no-deps | perl -0ne 'print $1 if /"target_directory"\s*:\s*"([^"]+)"/')}"

TAG_NAME="${TAG_NAME:-$(git -C "${WORKSPACE_ROOT}" describe --tags --abbrev=0 2>/dev/null || echo "v0.1.0")}"
OUT_DIR="${OUT_DIR:-${TARGET_DIR}/dist}"
JNI_LIBS_DIR="${SCRIPT_DIR}/app/src/main/jniLibs"
APK_SRC="${SCRIPT_DIR}/app/build/outputs/apk/release/app-release.apk"
APK_NAME="nerust-${TAG_NAME}-android-arm64-v8a.apk"
SIGNING_TEMP_DIR=""
export CARGO_NDK_PLATFORM="${CARGO_NDK_PLATFORM:-28}"

cleanup() {
    if [[ -n "${SIGNING_TEMP_DIR}" && -d "${SIGNING_TEMP_DIR}" ]]; then
        rm -rf "${SIGNING_TEMP_DIR}"
    fi
}
trap cleanup EXIT

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

rm -rf "${JNI_LIBS_DIR}"
mkdir -p "${JNI_LIBS_DIR}"

prepare_signing_keystore

# AGP's strip task silently skips without a resolved NDK ("Unable to
# strip ... packaging them as they are" -> 156MB APK incident). Resolve
# the same side-by-side NDK cargo-ndk uses and fail loudly when absent.
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    sdk_root="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
    if [[ -n "${sdk_root}" && -d "${sdk_root}/ndk" ]]; then
        latest_ndk="$(ls -1 "${sdk_root}/ndk" 2>/dev/null | sort | tail -n 1)"
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
    -o "${JNI_LIBS_DIR}" \
    build -p nerust_android --release

(
    cd "${SCRIPT_DIR}"
    ./gradlew --no-daemon assembleRelease
)

# Loud gate against silent native bloat: DWARF must be gone and each
# release .so must stay within budget (thin-LTO DWARF incident: 133MB).
# Resolves llvm-readelf from the NDK first, then PATH; warns and skips
# the gate when no reader exists off CI, fails hard when CI needs it.
BUDGET_BYTES="${BUDGET_BYTES:-41943040}"
readelf_bin=""
for cand in "${ANDROID_NDK_HOME}"/toolchains/llvm/prebuilt/*/bin/llvm-readelf; do
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
    echo "Skipping native DWARF/size gate (SKIP_NATIVE_GATE is set)" >&2
elif [[ -z "${readelf_bin}" ]]; then
    if [[ -n "${CI:-}" ]]; then
        echo "no llvm-readelf/readelf found: cannot verify DWARF strip" >&2
        exit 1
    fi
    echo "warning: no llvm-readelf/readelf found; skipping DWARF/size gate" >&2
else
    shopt -s nullglob
    so_files=("${JNI_LIBS_DIR}"/*/libmain.so)
    shopt -u nullglob
    if (( ${#so_files[@]} == 0 )); then
        if [[ -n "${CI:-}" ]]; then
            echo "no libmain.so found under ${JNI_LIBS_DIR}" >&2
            exit 1
        fi
        echo "warning: no libmain.so found under ${JNI_LIBS_DIR}; skipping gate" >&2
    else
        for main_so in "${so_files[@]}"; do
            abi="$(basename "$(dirname "${main_so}")")"
            if "${readelf_bin}" -S "${main_so}" | grep -q '\.debug_'; then
                echo "${abi}/libmain.so contains DWARF sections; release strip is not effective" >&2
                exit 1
            fi
            so_size="$(wc -c < "${main_so}" | tr -d '[:space:]')"
            if (( so_size > BUDGET_BYTES )); then
                echo "${abi}/libmain.so is ${so_size} bytes, over the ${BUDGET_BYTES} budget (${abi})" >&2
                exit 1
            fi
            echo "${abi}/libmain.so: ${so_size} bytes, no DWARF"
        done
    fi
fi

mkdir -p "${OUT_DIR}"
cp "${APK_SRC}" "${OUT_DIR}/${APK_NAME}"
sha256sum "${OUT_DIR}/${APK_NAME}" > "${OUT_DIR}/${APK_NAME}.sha256"
