#!/usr/bin/env bash

set -euo pipefail

# renovate: datasource=crate depName=cargo-ndk
readonly DEFAULT_CARGO_NDK_VERSION="4.1.2"

CARGO_NDK_VERSION="${CARGO_NDK_VERSION:-$DEFAULT_CARGO_NDK_VERSION}"
CARGO_HOME_DIR="${CARGO_HOME:-${HOME}/.cargo}"
INSTALL_DIR="${CARGO_HOME_DIR}/bin"
ARCHIVE_SHA256=""

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
        target="x86_64-unknown-linux-musl"
        ARCHIVE_SHA256="dc148bebbfcd7d5355e9858530d5a69c20ce69cf3268c54b9f63c60cb4c9a966"
        ;;
    Linux-arm64 | Linux-aarch64)
        target="aarch64-unknown-linux-musl"
        ARCHIVE_SHA256="2d9f4f28f797ee98a97a5b760197e4e64f2ce2967d716cf57c69ad01f4ed72ba"
        ;;
    Darwin-arm64 | Darwin-aarch64)
        target="aarch64-apple-darwin"
        ARCHIVE_SHA256="76d87f5f1bbbbe3980b979f7d1bdef00849dde11c669be243223942b296f5006"
        ;;
    Darwin-x86_64)
        target="x86_64-apple-darwin"
        ARCHIVE_SHA256="ea30f5e7898ce0b1c7af89e0a2dd2db61c494d81ad1b36284f74f1ab5fd85956"
        ;;
    *)
        echo "unsupported host for cargo-ndk binary install: $(uname -s)-$(uname -m)" >&2
        exit 1
        ;;
esac

archive="cargo-ndk-${target}-v${CARGO_NDK_VERSION}.tgz"
url="https://github.com/bbqsrc/cargo-ndk/releases/download/v${CARGO_NDK_VERSION}/${archive}"
tmp_dir="$(mktemp -d)"

cleanup() {
    rm -rf "${tmp_dir}"
}
trap cleanup EXIT

verify_sha256() {
    local actual

    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "${tmp_dir}/${archive}" | awk '{ print $1 }')"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "${tmp_dir}/${archive}" | awk '{ print $1 }')"
    else
        echo "sha256sum or shasum is required to verify the cargo-ndk archive" >&2
        exit 1
    fi

    if [[ "${actual}" != "${ARCHIVE_SHA256}" ]]; then
        echo "cargo-ndk checksum mismatch for ${archive}" >&2
        exit 1
    fi
}

mkdir -p "${INSTALL_DIR}"

curl --fail --location --silent --show-error "${url}" -o "${tmp_dir}/${archive}"
verify_sha256
tar -xzf "${tmp_dir}/${archive}" -C "${tmp_dir}"

archive_root="${tmp_dir}/cargo-ndk-${target}-v${CARGO_NDK_VERSION}"
install -m 0755 "${archive_root}/cargo-ndk" "${INSTALL_DIR}/cargo-ndk"
install -m 0755 "${archive_root}/cargo-ndk-env" "${INSTALL_DIR}/cargo-ndk-env"
install -m 0755 "${archive_root}/cargo-ndk-test" "${INSTALL_DIR}/cargo-ndk-test"
