#!/bin/sh
# GTK4 and GTK3/Tao must run in separate processes, including each GTK test.
set -eu
cd "$(dirname "$0")/.."
export GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals

cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p wakui2-demo --all-targets --no-default-features --features backend-tao -- -D warnings
cargo test --locked -p wakui2-runtime -p wakui2-markup -p wakui2-macros
cargo test --locked -p wakui2-tao --lib
for name in native_contract native_executor_completion native_invalid_keyed_snapshots \
    native_root_style native_menu_namespaces native_coordinated_multiwindow_close \
    native_coordinated_close_timeout native_coordinated_close_cancel \
    native_coordinated_close_weak_teardown native_entry_converted_replacement \
    native_entry_incremental_edits_and_readonly; do
    xvfb-run -a cargo test --locked -p wakui2-gtk --lib "tests::$name" -- --exact --test-threads=1
done
xvfb-run -a cargo test --locked -p wakui2-tao --lib \
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
printf '%s\n' 'VERIFY2 PASS: core, compiler, native GTK/Tao, gallery and minimal builds'