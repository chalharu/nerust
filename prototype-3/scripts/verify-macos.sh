#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test "$(uname -s)" = Darwin || { printf '%s\n' 'macOS interactive desktop required' >&2; exit 1; }
unset DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH DYLD_INSERT_LIBRARIES
export RUSTC_WRAPPER= CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export WAKUI_MACOS_TIMEOUT_SECONDS="${WAKUI_MACOS_TIMEOUT_SECONDS:-45}" WAKUI_MACOS_METRICS=1

cargo fmt --all -- --check
cargo clippy --locked -p wakui2-runtime -p wakui2-markup -p wakui2-macros \
    -p wakui3-macos -p wakui3-settings -p wakui3-library \
    --all-targets --no-default-features --features backend-macos -- -D warnings
cargo test --locked -p wakui2-runtime -p wakui2-markup -p wakui2-macros \
    -p wakui3-macos -p wakui3-settings -p wakui3-library \
    --no-default-features --features backend-macos
output=$(cargo run --locked -p wakui3-settings --no-default-features --features backend-macos -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS final-save-exit saved-json verified model-closed$'
output=$(cargo run --locked -p wakui3-library --no-default-features --features backend-macos -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^LIBRARY3 macos PASS latest-query-bursts samples=20 injected-edits=60 '
printf '%s\n' "$output" | grep -q '^LIBRARY3 macos PASS native-close model-closed$'
output=$(cargo run --locked -p wakui3-macos --example conformance)
printf '%s\n' "$output"
for marker in injected-field-editor native-menu viewport-contract keyed-rows reaction-chain weak-view; do
    printf '%s\n' "$output" | grep -q "^MACOS3 PASS $marker "
done
printf '%s\n' 'VERIFY3 MACOS PASS: core regressions, AppKit settings/library, injected input, bounded dispatch and native drawing'