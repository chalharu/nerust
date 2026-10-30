#!/bin/sh
# Release CPU regression budgets, not native frame/input latency guarantees.
set -eu
cd "$(dirname "$0")/.."

assert_budget() {
    marker=$1
    p50=$2
    p95=$3
    awk -v marker="$marker" -v max50="$p50" -v max95="$p95" '
        index($0, marker) {
            median = ""; tail = ""
            for (i = 1; i <= NF; i++) {
                split($i, pair, "=")
                if (pair[1] == "p50_ns") median = pair[2]
                if (pair[1] == "p95_ns") tail = pair[2]
            }
            if (median == "" || tail == "" || median !~ /^[0-9]+$/ || tail !~ /^[0-9]+$/ || median + 0 > max50 || tail + 0 > max95 || median + 0 > tail + 0) {
                print "MEASURE3 FAIL " marker " p50_ns=" median " p95_ns=" tail " budgets=" max50 "/" max95 > "/dev/stderr"
                failed = 1
            }
            found++
        }
        END { if (!found || failed) exit 1 }
    '
}

output=$(cargo run --locked --release -p wakui2-runtime --example scaling)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -q '^SCALING3 PASS: read clones=0, update clones=1, notification counts exact$'
printf '%s\n' "$output" | grep -q '^SCOPED3 PASS clone-free-replacement indexed-dependencies noop-generation$'
printf '%s\n' "$output" | assert_budget 'SCOPED3 subscribers=' "${WAKUI_SCOPED_P50_NS:-100000}" "${WAKUI_SCOPED_P95_NS:-500000}"

workload() {
    package=$1
    name=$2
    marker=$3
    p50=$4
    p95=$5
    if [ "$(uname -s)" = Darwin ]; then
        output=$(cargo test --locked --release -p "$package" --no-default-features --features backend-macos "$name" -- --exact --ignored --nocapture --test-threads=1)
    else
        output=$(cargo test --locked --release -p "$package" "$name" -- --exact --ignored --nocapture --test-threads=1)
    fi
    printf '%s\n' "$output"
    printf '%s\n' "$output" | grep -q '^test result: ok\. 1 passed; 0 failed;'
    # libtest prints the test name before uncaptured output on the same line.
    printf '%s\n' "$output" | grep -q "WORKLOAD3 $marker release=true "
    printf '%s\n' "$output" | assert_budget "WORKLOAD3 $marker release=true " "$p50" "$p95"
}
workload wakui3-settings model::tests::perf::settings_draft_projection settings "${WAKUI_SETTINGS_P50_NS:-1000000}" "${WAKUI_SETTINGS_P95_NS:-5000000}"
workload wakui3-library tests::perf::library_rescan_projection library "${WAKUI_LIBRARY_P50_NS:-5000000}" "${WAKUI_LIBRARY_P95_NS:-20000000}"
printf '%s\n' 'MEASURE3 PASS: release clone/projection contracts and p50/p95 CPU budgets'