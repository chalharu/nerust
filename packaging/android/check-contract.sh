#!/usr/bin/env bash
# Verify constants AND actual event/JNI structures before host codecs/locks.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PYTHONDONTWRITEBYTECODE=1

phase() {
    echo "--- contract: $1"
}

if [[ "${1:-}" == "--slow-only" ]]; then
    phase "decoder probes"
    python3 "${SCRIPT_DIR}/test-decoder-contract.py"
    phase "host codec tests"
    cargo test --locked --manifest-path "${SCRIPT_DIR}/../../Cargo.toml" -p nerust_android --lib
    echo "Android bridge contract (slow): OK"
    exit 0
fi

phase "generate --check"
python3 "${SCRIPT_DIR}/generate-contract.py" --check
phase "verify"
python3 "${SCRIPT_DIR}/verify-contract.py"
phase "contract tests"
python3 "${SCRIPT_DIR}/test-contract.py"
phase "inventory gate self-tests"
python3 "${SCRIPT_DIR}/test-inventory.py"
if [[ "${1:-}" == "--fast-only" ]]; then
    echo "Android bridge contract (fast): OK"
    exit 0
fi
phase "decoder probes"
python3 "${SCRIPT_DIR}/test-decoder-contract.py"
phase "host codec tests"
cargo test --locked --manifest-path "${SCRIPT_DIR}/../../Cargo.toml" -p nerust_android --lib
echo "Android bridge contract: OK"
