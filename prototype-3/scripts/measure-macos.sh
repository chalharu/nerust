#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test "$(uname -s)" = Darwin || { printf '%s\n' 'macOS interactive desktop required' >&2; exit 1; }
unset DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH DYLD_INSERT_LIBRARIES
export RUSTC_WRAPPER= CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export WAKUI_MACOS_TIMEOUT_SECONDS="${WAKUI_MACOS_TIMEOUT_SECONDS:-45}" WAKUI_MACOS_METRICS=1
export WAKUI_ALLOW_CLIPBOARD_PROBE=1

cargo run --locked --release -p wakui3-macos --example conformance
cargo run --locked --release -p wakui3-library --no-default-features --features backend-macos -- --smoke
sh scripts/measure.sh
printf '%s\n' 'MEASURE3 MACOS PASS: release CPU budgets, 4-window image/edit/selection generation contracts and informational synthetic input'