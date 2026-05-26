#!/usr/bin/env bash
# packaging/linux/package.sh
#
# Build a release tarball for the Nerust Tao frontend on Linux.
#
# Usage:
#   packaging/linux/package.sh <arch>
#
# Arguments:
#   arch   Target architecture label embedded in the tarball name, e.g.
#          "x86_64" or "aarch64".
#
# Environment:
#   BINARY   Path to the compiled nerust_tao binary.
#            Defaults to target/release/nerust_tao.
#   OUT_DIR  Directory where the tarball is written.
#            Defaults to target/dist.
#
# Output:
#   <OUT_DIR>/nerust-<tag>-linux-<arch>.tar.gz
#
# Contents of the tarball:
#   nerust_tao   - the release binary
#   README.md    - project README
#   LICENSE      - MPL-2.0 licence

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --manifest-path "${WORKSPACE_ROOT}/Cargo.toml" --format-version 1 --no-deps | perl -0ne 'print $1 if /"target_directory"\s*:\s*"([^"]+)"/')}"

ARCH="${1:?Usage: package.sh <arch>}"
BINARY="${BINARY:-${TARGET_DIR}/release/nerust_tao}"
OUT_DIR="${OUT_DIR:-${TARGET_DIR}/dist}"
TAG_NAME="${TAG_NAME:-$(git -C "${WORKSPACE_ROOT}" describe --tags --abbrev=0 2>/dev/null || echo "v0.1.0")}"

TARBALL_NAME="nerust-${TAG_NAME}-linux-${ARCH}"
STAGE_DIR="${OUT_DIR}/${TARBALL_NAME}"

echo "Packaging Linux tarball for arch=${ARCH}"
echo "  binary : ${BINARY}"
echo "  output : ${OUT_DIR}/${TARBALL_NAME}.tar.gz"

# Validate inputs
if [[ -z "${TARGET_DIR}" ]]; then
    echo "Error: failed to determine cargo target directory" >&2
    exit 1
fi

if [[ ! -f "${BINARY}" ]]; then
    echo "Error: binary not found at '${BINARY}'" >&2
    exit 1
fi

# Prepare staging directory
rm -rf "${STAGE_DIR}"
mkdir -p "${STAGE_DIR}"

cp "${BINARY}"   "${STAGE_DIR}/nerust_tao"
cp README.md     "${STAGE_DIR}/README.md"
cp LICENSE       "${STAGE_DIR}/LICENSE"

# Ensure the binary is executable
chmod +x "${STAGE_DIR}/nerust_tao"

# Create tarball (reproducible: sort entries, zero mtime)
mkdir -p "${OUT_DIR}"
tar \
    --create \
    --gzip \
    --file "${OUT_DIR}/${TARBALL_NAME}.tar.gz" \
    --directory "${OUT_DIR}" \
    --sort=name \
    --mtime='@0' \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    "${TARBALL_NAME}"

echo "Created ${OUT_DIR}/${TARBALL_NAME}.tar.gz"

# Clean up staging directory
rm -rf "${STAGE_DIR}"
