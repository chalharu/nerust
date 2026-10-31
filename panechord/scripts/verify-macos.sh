#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test "$(uname -s)" = Darwin || { printf '%s\n' 'macOS interactive desktop required' >&2; exit 1; }
unset DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH DYLD_INSERT_LIBRARIES
export RUSTC_WRAPPER="$PWD/scripts/yaml-rustc.py" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export PANECHORD_MACOS_TIMEOUT_SECONDS="${PANECHORD_MACOS_TIMEOUT_SECONDS:-45}" PANECHORD_MACOS_METRICS=1
export PANECHORD_ALLOW_CLIPBOARD_PROBE=1
python3 scripts/test-yaml-rustc.py
python3 ../gui/frontends/check_frontend_policy.py

cargo fmt -p panechord -p panechord-runtime -p panechord-markup -p panechord-macros \
    -p panechord-gtk -p panechord-tao -p panechord-macos -p panechord-demo \
    -p panechord-settings -p panechord-library -p nerust_frontend_viewdefs -- --check
cargo clippy --locked -p nerust_frontend_viewdefs --features panechord --all-targets -- -D warnings
cargo test --locked -p nerust_frontend_viewdefs --features panechord
cargo clippy --locked -p panechord -p panechord-runtime -p panechord-markup -p panechord-macros \
    -p panechord-macos -p panechord-settings -p panechord-library \
    --all-targets --no-default-features --features backend-macos -- -D warnings
cargo test --locked -p panechord-runtime -p panechord-markup -p panechord-macros \
    -p panechord-macos -p panechord-settings -p panechord-library \
    --no-default-features --features backend-macos
output=$(cargo run --locked -p panechord-settings --no-default-features --features backend-macos -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS final-save-exit saved-json verified model-closed$'
output=$(cargo run --locked -p panechord-library --no-default-features --features backend-macos -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^LIBRARY3 macos PASS latest-query-bursts samples=20 injected-edits=60 '
printf '%s\n' "$output" | grep -q '^LIBRARY3 macos PASS native-close model-closed$'
output=$(cargo run --locked -p panechord-macos --example conformance)
printf '%s\n' "$output"
for marker in close-recovery event-namespace validation-scope typed-menu injected-field-editor native-menu viewport-contract readonly-viewport menu-transaction nested-cleanup dynamic-row-height keyed-edit-state keyed-rows reaction-chain weak-view; do
    printf '%s\n' "$output" | grep -q "^MACOS3 PASS $marker "
done
output=$(cargo run --locked -p panechord-settings --no-default-features --features backend-macos --example controls)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^CONTROLS3 PASS checkbox readonly radio slider combo image tab dialog$'
printf '%s\n' "$output" | grep -q '^CONTRACT3 PASS slider closed capacity conversion readonly hidden disabled normalized-display$'
printf '%s\n' "$output" | grep -q '^TABS3 PASS top bottom left right selection retained$'
printf '%s\n' "$output" | grep -q '^MENUS3 PASS nested typed-argument parent-enablement$'
printf '%s\n' "$output" | grep -q '^DIALOG3 PASS readonly hidden-parent close-reopen$'
output=$(cargo run --locked -p panechord-macos --example multiwindow)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^MACOS3 PASS single-session repeated-attach foreign-attach-and-registration-rejected$'
printf '%s\n' "$output" | grep -q '^MACOS3 PASS closed-registration rejected-before-session-attach$'
printf '%s\n' "$output" | grep -q '^MACOS3 PASS active-close menu-handoff quit-veto-without-present$'
printf '%s\n' "$output" | grep -q '^MACOS3 PASS multiwindow shared-store reopen active-menu quit-veto confirm-cancel-drain-close$'
sh scripts/verify-tao-macos.sh
python3 scripts/bundle-macos.py --verify
python3 scripts/bundle-macos.py --package panechord-library --verify
sh scripts/measure-macos.sh
printf '%s\n' 'VERIFY3 MACOS PASS: core regressions, AppKit settings/library, injected input, bounded dispatch and native drawing'