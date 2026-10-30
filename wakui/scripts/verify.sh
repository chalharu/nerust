#!/bin/sh
# Run from anywhere. GTK4 and Tao/GTK3 execute in separate processes.
set -eu
cd "$(dirname "$0")/.."

cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo clippy --locked -p wakui-demo --all-targets --no-default-features --features backend-tao -- -D warnings
GSK_RENDERER=cairo xvfb-run -a cargo test --locked --workspace -- --test-threads=1
cargo run --locked -p wakui-demo -- --headless
cargo build --locked -p wakui-demo
output=$(GSK_RENDERER=cairo timeout 45s xvfb-run -a cargo run --locked -p wakui-demo -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^SMOKE PASS backend=gtk '
cargo build --locked -p wakui-demo --no-default-features --features backend-tao
output=$(timeout 45s xvfb-run -a cargo run --locked -p wakui-demo --no-default-features --features backend-tao -- --smoke)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^SMOKE PASS backend=tao '
printf '%s\n' 'VERIFY PASS: unit/integration tests, GTK and Tao native smoke'