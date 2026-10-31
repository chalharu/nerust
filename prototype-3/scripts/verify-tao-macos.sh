#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test "$(uname -s)" = Darwin || { printf '%s\n' 'macOS interactive desktop required' >&2; exit 1; }
unset DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH DYLD_INSERT_LIBRARIES
export RUSTC_WRAPPER="$PWD/scripts/yaml-rustc.py" CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export WAKUI_ALLOW_CLIPBOARD_PROBE=1

native_run() {
    if output=$(python3 -c '
import os, signal, subprocess, sys
process = subprocess.Popen(sys.argv[1:], start_new_session=True)
try:
    seconds = float(os.environ.get("WAKUI_NATIVE_TIMEOUT_SECONDS", "45"))
    sys.exit(process.wait(timeout=seconds))
except subprocess.TimeoutExpired:
    os.killpg(process.pid, signal.SIGKILL)
    process.wait()
    print(f"Tao native run exceeded {seconds} seconds", file=sys.stderr)
    sys.exit(124)
' cargo run --locked "$@" 2>&1); then
        printf '%s\n' "$output"
    else
        printf '%s\n' "$output" >&2
        return 1
    fi
}

cargo clippy --locked -p wakui2-tao --all-targets -- -D warnings
for package in wakui2-demo wakui3-settings wakui3-library; do
    cargo clippy --locked -p "$package" --all-targets --no-default-features --features backend-tao -- -D warnings
    cargo build --locked -p "$package" --no-default-features --features backend-tao
done
cargo build --locked -p wakui2-tao --example native_smoke
native_run -p wakui2-tao --example native_smoke
printf '%s\n' "$output" | grep -q '^native-smoke: close callback OK'
native_run -p wakui3-settings --no-default-features --features backend-tao --example controls
printf '%s\n' "$output" | grep -q '^CONTROLS3 PASS checkbox readonly radio slider combo image tab dialog$'
native_run -p wakui2-demo --no-default-features --features backend-tao -- --smoke
printf '%s\n' "$output" | grep -q '^PROTOTYPE2 PASS '
native_run -p wakui3-settings --no-default-features --features backend-tao -- --smoke
for marker in 'entries readonly invalid-retention save-gating cancel-no-write' \
    'async-save wm-close-veto menu-quit-veto keep-editing cancel-close' \
    'busy-exit-waits newer-invalid-edit-reprompts' \
    'final-save-exit saved-json verified model-closed'; do
    printf '%s\n' "$output" | grep -q "^SETTINGS3 PASS $marker$"
done
native_run -p wakui3-library --no-default-features --features backend-tao -- --smoke
printf '%s\n' "$output" | grep -q '^LIBRARY3 tao PASS native-events scan filter selection invalid-folder recovery rescan$'
printf '%s\n' "$output" | grep -q '^LIBRARY3 tao PASS native-close model-closed$'
printf '%s\n' 'VERIFY3 TAO MACOS PASS: strict Clippy, native drawing/clipboard, gallery and settings/library automation'