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
export CARGO_NDK_PLATFORM="${CARGO_NDK_PLATFORM:-26}"

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

cargo ndk \
    --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" \
    -t arm64-v8a \
    -o "${JNI_LIBS_DIR}" \
    build -p nerust_android --release

(
    cd "${SCRIPT_DIR}"
    ./gradlew --no-daemon assembleRelease
)

mkdir -p "${OUT_DIR}"
cp "${APK_SRC}" "${OUT_DIR}/${APK_NAME}"
sha256sum "${OUT_DIR}/${APK_NAME}" > "${OUT_DIR}/${APK_NAME}.sha256"
