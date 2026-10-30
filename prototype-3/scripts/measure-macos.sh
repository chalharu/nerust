#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test "$(uname -s)" = Darwin || { printf '%s\n' 'macOS interactive desktop required' >&2; exit 1; }
unset DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH DYLD_INSERT_LIBRARIES
export RUSTC_WRAPPER= CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export WAKUI_MACOS_TIMEOUT_SECONDS="${WAKUI_MACOS_TIMEOUT_SECONDS:-45}" WAKUI_MACOS_METRICS=1

cargo run --locked --release -p wakui3-macos --example conformance
cargo run --locked --release -p wakui3-library --no-default-features --features backend-macos -- --smoke
cargo test --locked --release -p wakui3-settings --no-default-features --features backend-macos \
    settings_draft_projection -- --ignored --nocapture
cargo test --locked --release -p wakui3-library --no-default-features --features backend-macos \
    library_rescan_projection -- --ignored --nocapture
printf '%s\n' 'MEASURE3 MACOS PASS: informational CPU drawing, synthetic input, sandbox scan bursts and projection workloads (no timing thresholds)'