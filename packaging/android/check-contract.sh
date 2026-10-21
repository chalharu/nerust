#!/usr/bin/env bash
# Verify constants AND actual event/JNI structures before host codecs/locks.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
python3 "${SCRIPT_DIR}/generate-contract.py" --check
python3 "${SCRIPT_DIR}/verify-contract.py"
python3 "${SCRIPT_DIR}/test-contract.py"
python3 "${SCRIPT_DIR}/test-decoder-contract.py"
cargo test --manifest-path "${SCRIPT_DIR}/../../Cargo.toml" -p nerust_android --lib
echo "Android bridge contract: OK"
