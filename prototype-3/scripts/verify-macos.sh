#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test "$(uname -s)" = Darwin || { printf '%s\n' 'macOS interactive desktop required' >&2; exit 1; }
unset DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH DYLD_INSERT_LIBRARIES
export RUSTC_WRAPPER="$PWD/scripts/yaml-rustc.py" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export WAKUI_MACOS_TIMEOUT_SECONDS="${WAKUI_MACOS_TIMEOUT_SECONDS:-45}" WAKUI_MACOS_METRICS=1
export WAKUI_ALLOW_CLIPBOARD_PROBE=1
python3 scripts/test-yaml-rustc.py

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
for marker in close-recovery event-namespace validation-scope typed-menu injected-field-editor native-menu viewport-contract readonly-viewport menu-transaction nested-cleanup dynamic-row-height keyed-edit-state keyed-rows reaction-chain weak-view; do
    printf '%s\n' "$output" | grep -q "^MACOS3 PASS $marker "
done
output=$(cargo run --locked -p wakui3-settings --no-default-features --features backend-macos --example controls)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^CONTROLS3 PASS checkbox readonly radio slider combo image tab dialog$'
printf '%s\n' "$output" | grep -q '^TABS3 PASS top bottom left right selection retained$'
output=$(cargo run --locked -p wakui3-macos --example multiwindow)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^MACOS3 PASS single-session repeated-attach foreign-attach-and-registration-rejected$'
printf '%s\n' "$output" | grep -q '^MACOS3 PASS closed-registration rejected-before-session-attach$'
printf '%s\n' "$output" | grep -q '^MACOS3 PASS active-close menu-handoff quit-veto-without-present$'
printf '%s\n' "$output" | grep -q '^MACOS3 PASS multiwindow shared-store reopen active-menu quit-veto confirm-cancel-drain-close$'
sh scripts/verify-tao-macos.sh
python3 scripts/bundle-macos.py --verify
python3 scripts/bundle-macos.py --package wakui3-library --verify
printf '%s\n' 'VERIFY3 MACOS PASS: core regressions, AppKit settings/library, injected input, bounded dispatch and native drawing'