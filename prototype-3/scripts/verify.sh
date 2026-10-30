#!/bin/sh
# GTK4 and GTK3/Tao must run in separate processes, including each GTK test.
set -eu
cd "$(dirname "$0")/.."
export GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals

native_test() {
    output=$(xvfb-run -a cargo test --locked "$@")
    printf '%s\n' "$output"
    printf '%s\n' "$output" | grep -q '^test result: ok\. 1 passed; 0 failed;'
}

cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p wakui2-demo --all-targets --no-default-features --features backend-tao -- -D warnings
cargo clippy --locked -p wakui3-settings --all-targets --no-default-features --features backend-tao -- -D warnings
cargo test --locked -p wakui2-runtime -p wakui2-markup -p wakui2-macros
output=$(cargo run --locked -p wakui2-runtime --example scaling)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^SCALING3 PASS: read clones=0, update clones=1, notification counts exact$'
cargo test --locked -p wakui2-tao --lib
cargo test --locked -p wakui3-settings
cargo test --locked -p wakui3-settings --no-default-features --features backend-tao
for name in native_contract native_executor_completion native_invalid_keyed_snapshots \
    native_root_style native_menu_namespaces native_coordinated_multiwindow_close \
    native_coordinated_close_timeout native_coordinated_close_cancel \
    native_coordinated_close_weak_teardown native_entry_converted_replacement \
    native_entry_incremental_edits_and_readonly native_close_requests_weak_and_reentrant \
    native_close_ready_subscription_retains_model_without_view_cycle \
    native_close_request_handler_precedes_coordinator_and_menu_falls_back \
    native_canonical_shortcut_semantics native_grid_equal_columns_geometry_and_spans; do
    native_test -p wakui2-gtk --lib "tests::$name" -- --exact --test-threads=1
done
GDK_SCALE=2 native_test -p wakui2-gtk --lib \
    tests::native_grid_equal_columns_geometry_and_spans -- --exact --test-threads=1
native_test -p wakui2-tao --lib \
    tests::native_menu_append_failure_restores_accelerators_registry_and_quit -- --exact --ignored --test-threads=1
cargo build --locked -p wakui2-demo --example minimal
cargo build --locked -p wakui2-demo
output=$(timeout 45s xvfb-run -a cargo run --locked -p wakui2-demo -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^PROTOTYPE2 PASS '
cargo build --locked -p wakui2-demo --example minimal --no-default-features --features backend-tao
cargo build --locked -p wakui2-demo --no-default-features --features backend-tao
output=$(timeout 45s xvfb-run -a cargo run --locked -p wakui2-demo --no-default-features --features backend-tao -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^PROTOTYPE2 PASS '
cargo build --locked -p wakui2-tao --example native_smoke
output=$(timeout 45s xvfb-run -a cargo run --locked -p wakui2-tao --example native_smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^native-smoke: close callback OK'
settings_smoke() {
    output=$(timeout 45s xvfb-run -a cargo run --locked -p wakui3-settings "$@" -- --smoke)
    printf '%s\n' "$output"
    printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS entries readonly invalid-retention save-gating cancel-no-write$'
    printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS async-save wm-close-veto menu-quit-veto keep-editing cancel-close$'
    printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS busy-exit-waits newer-invalid-edit-reprompts$'
    printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS final-save-exit saved-json verified model-closed$'
}
settings_smoke
settings_smoke --no-default-features --features backend-tao
printf '%s\n' 'VERIFY3 PASS: inherited regressions, shared ownership, typed commands, common UI contracts and settings GTK/Tao'