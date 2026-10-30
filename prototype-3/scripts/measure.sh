#!/bin/sh
# CPU observations only: no timing thresholds, native frames or input latency.
set -eu
cd "$(dirname "$0")/.."

output=$(cargo run --locked --release -p wakui2-runtime --example scaling)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^SCALING3 PASS: read clones=0, update clones=1, notification counts exact$'

workload() {
    package=$1
    name=$2
    marker=$3
    output=$(cargo test --locked --release -p "$package" "$name" -- --exact --ignored --nocapture --test-threads=1)
    printf '%s\n' "$output"
    printf '%s\n' "$output" | grep -q '^test result: ok\. 1 passed; 0 failed;'
    # libtest prints the test name before uncaptured output on the same line.
    printf '%s\n' "$output" | grep -q "WORKLOAD3 $marker release=true "
}
workload wakui3-settings model::tests::perf::settings_draft_projection settings
workload wakui3-library tests::perf::library_rescan_projection library
printf '%s\n' 'MEASURE3 PASS: release clone contracts and actual settings/library CPU workloads; no timing thresholds'