#!/usr/bin/env bash
# Verify generated constants, then execute the real host-side codecs/locks.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
python3 "${SCRIPT_DIR}/generate-contract.py" --check
python3 "${SCRIPT_DIR}/test-contract.py"
cargo test --manifest-path "${SCRIPT_DIR}/../../Cargo.toml" -p nerust_android --lib
echo "Android bridge contract: OK"
