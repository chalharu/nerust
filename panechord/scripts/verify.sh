#!/bin/sh
# GTK4 and GTK3/Tao must run in separate processes, including each GTK test.
set -eu
cd "$(dirname "$0")/.."
export GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals
export RUSTC_WRAPPER="$PWD/scripts/yaml-rustc.py"
export PANECHORD_ALLOW_CLIPBOARD_PROBE=1
native_timeout="${PANECHORD_NATIVE_TIMEOUT_SECONDS:-45}s"
python3 scripts/test-yaml-rustc.py
python3 ../gui/frontends/check_frontend_policy.py

native_test() {
    if output=$(timeout "$native_timeout" xvfb-run -a cargo test --locked "$@" 2>&1); then
        printf '%s\n' "$output"
    else
        printf '%s\n' "$output" >&2
        return 1
    fi
    printf '%s\n' "$output" | grep -q '^test result: ok\. 1 passed; 0 failed;'
}

cargo fmt -p panechord -p panechord-runtime -p panechord-markup -p panechord-macros \
    -p panechord-gtk -p panechord-tao -p panechord-macos -p panechord-demo \
    -p panechord-settings -p panechord-library -p nerust_frontend_viewdefs -- --check
cargo clippy --locked -p nerust_frontend_viewdefs --features panechord --all-targets -- -D warnings
cargo test --locked -p nerust_frontend_viewdefs --features panechord
cargo clippy --locked -p panechord -p panechord-runtime -p panechord-markup -p panechord-macros \
    -p panechord-gtk -p panechord-tao -p panechord-macos -p panechord-demo \
    -p panechord-settings -p panechord-library --all-targets -- -D warnings
cargo clippy --locked -p panechord-demo --all-targets --no-default-features --features backend-tao -- -D warnings
cargo clippy --locked -p panechord-settings --all-targets --no-default-features --features backend-tao -- -D warnings
cargo clippy --locked -p panechord-library --all-targets --no-default-features --features backend-tao -- -D warnings
cargo test --locked -p panechord-runtime -p panechord-markup -p panechord-macros
output=$(cargo run --locked -p panechord-runtime --example scaling)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^SCALING3 PASS: read clones=0, update clones=1, notification counts exact$'
cargo test --locked -p panechord-tao --lib
cargo test --locked -p panechord-gtk --lib --no-run
native_test -p panechord-gtk --lib tests::native_style_late_close_contract -- --exact --ignored --test-threads=1
cargo test --locked -p panechord-settings
cargo test --locked -p panechord-settings --no-default-features --features backend-tao
cargo test --locked -p panechord-library
cargo test --locked -p panechord-library --no-default-features --features backend-tao
for backend in gtk tao; do
    output=$(timeout "$native_timeout" xvfb-run -a cargo run --locked -p panechord-settings --no-default-features --features "backend-$backend" --example controls)
    printf '%s\n' "$output"
    printf '%s\n' "$output" | grep -q '^CONTROLS3 PASS checkbox readonly radio slider combo image tab dialog$'
    printf '%s\n' "$output" | grep -q '^CONTRACT3 PASS slider closed capacity conversion readonly hidden disabled normalized-display$'
    printf '%s\n' "$output" | grep -q '^MENUS3 PASS nested typed-argument parent-enablement$'
    printf '%s\n' "$output" | grep -q '^DIALOG3 PASS readonly hidden-parent close-reopen$'
    printf '%s\n' "$output" | grep -q '^TABS3 PASS top bottom left right selection retained$'
done
native_test -p panechord-settings \
    native_tests::native_generated_shared_windows_survive_peer_close -- --exact --ignored --test-threads=1
native_test -p panechord-settings \
    native_tests::native_settings_layout_is_disjoint_and_scrolls_when_short -- --exact --ignored --test-threads=1
for name in native_contract native_executor_completion native_invalid_keyed_snapshots \
    native_root_style native_menu_namespaces native_coordinated_multiwindow_close \
    native_coordinated_close_timeout native_coordinated_close_cancel \
    native_coordinated_close_weak_teardown native_entry_converted_replacement \
    native_entry_incremental_edits_and_readonly native_close_requests_weak_and_reentrant \
    native_close_ready_subscription_retains_model_without_view_cycle \
    native_close_request_handler_precedes_coordinator_and_menu_falls_back \
    native_canonical_shortcut_semantics native_grid_equal_columns_geometry_and_spans; do
    native_test -p panechord-gtk --lib "tests::$name" -- --exact --test-threads=1
done
GDK_SCALE=2 native_test -p panechord-gtk --lib \
    tests::native_grid_equal_columns_geometry_and_spans -- --exact --test-threads=1
native_test -p panechord-tao --lib \
    tests::native_menu_append_failure_restores_accelerators_registry_and_quit -- --exact --ignored --test-threads=1
# Isolated X11 physical input tests must not silently skip missing XTest support.
GTK_IM_MODULE=simple PANECHORD_REQUIRE_XTEST=1 native_test -p panechord-gtk --lib \
    tests::native_x11_japanese_compose_focus_clipboard_and_rejection -- --exact --ignored --test-threads=1
GTK_IM_MODULE=simple PANECHORD_REQUIRE_XTEST=1 native_test -p panechord-tao --lib \
    driver::input_tests::native_x11_keyboard_ime_signal_iced_fields_and_clipboard -- --exact --ignored --test-threads=1
native_test -p panechord-tao --lib \
    driver::input_tests::native_backend_focus_loss_clears_iced_modifiers -- --exact --ignored --test-threads=1
native_test -p panechord-tao --lib \
    driver::input_tests::native_x11_discrete_wheel_scrolls_window_content -- --exact --ignored --test-threads=1
cargo build --locked -p panechord-demo --example minimal
cargo build --locked -p panechord-demo
output=$(timeout "$native_timeout" xvfb-run -a cargo run --locked -p panechord-demo -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^PROTOTYPE2 PASS '
cargo build --locked -p panechord-demo --example minimal --no-default-features --features backend-tao
cargo build --locked -p panechord-demo --no-default-features --features backend-tao
output=$(timeout "$native_timeout" xvfb-run -a cargo run --locked -p panechord-demo --no-default-features --features backend-tao -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^PROTOTYPE2 PASS '
cargo build --locked -p panechord-tao --example native_smoke
output=$(timeout "$native_timeout" xvfb-run -a cargo run --locked -p panechord-tao --example native_smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^native-smoke: close callback OK'
cargo build --locked -p panechord-tao --example multiwindow
output=$(timeout "$native_timeout" xvfb-run -a cargo run --locked -p panechord-tao --example multiwindow)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^TAO3 PASS multiwindow drawn shared-store primary-close reopen foreign-and-closed-rejected veto-drain-close recoverable-auxiliary-error-isolated$'
settings_smoke() {
    output=$(timeout "$native_timeout" xvfb-run -a cargo run --locked -p panechord-settings "$@" -- --smoke)
    printf '%s\n' "$output"
    printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS entries readonly invalid-retention save-gating cancel-no-write$'
    printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS async-save wm-close-veto menu-quit-veto keep-editing cancel-close$'
    printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS busy-exit-waits newer-invalid-edit-reprompts$'
    printf '%s\n' "$output" | grep -q '^SETTINGS3 PASS final-save-exit saved-json verified model-closed$'
}
settings_smoke
settings_smoke --no-default-features --features backend-tao
library_smoke() {
    backend=$1
    shift
    output=$(timeout "$native_timeout" xvfb-run -a cargo run --locked -p panechord-library "$@" -- --smoke)
    printf '%s\n' "$output"
    printf '%s\n' "$output" | grep -q "^LIBRARY3 $backend PASS native-events scan filter selection invalid-folder recovery rescan$"
    printf '%s\n' "$output" | grep -q "^LIBRARY3 $backend PASS native-close model-closed$"
}
library_smoke gtk
library_smoke tao --no-default-features --features backend-tao
printf '%s\n' 'VERIFY3 PASS: inherited regressions, scoped reactions, Self arguments, Linux input and settings/library GTK/Tao'