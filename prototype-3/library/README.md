# Standalone file library

`wakui3-library` is the second practical screen after settings. It is independent
of nerust: no emulator integration, ROM parsing, contents reads, persistence or
user-folder writes. Only the isolated `--smoke` / test Sandbox creates fixtures.

The folder and case-insensitive filename filter are editable while scanning.
Scan/rescan retries failures; Select next file cycles through metadata details.
The shared `view!` uses a virtualized `ItemsControl` with six visible rows and
persistent path-keyed row Stores. Matching rows retain identity across rescans.

Scans are non-recursive and skip symlinks and directories. Supported extensions
are `.nes`, `.gb`, `.gbc`, `.gba`, case-insensitively. A scan visits at most 8192
directory entries and retains at most 256 matches. A capped scan reports this
explicitly: refine the filter. The retained subset follows filesystem iteration
order, then sorts by path; this is not an exhaustive library index. Folder input
must be UTF-8 for the TextBox; child paths retain native path bytes in their keys.

## Common wiring evaluation

Both settings and `wiring::connect` use the public
`Store::watch(source, react)` API rather than direct `subscribe` callbacks
capturing a Store.
The runtime supplies `&Store` to each callback. There are three retained scopes:

1. Query reaction publishes a new generation-tagged pending request.
2. Pending reaction accepts a `Policy::Latest` effect. The BlockingPool receives
   work only when that accepted future is polled, never before acceptance.
3. Projection reaction rebuilds bounded keyed rows/status/details, with its own
   outputs excluded from its source to avoid feedback.

Separating these reactions leaves one ingress slot for the callback's update or
effect even with `Limits { ingress: 1, .. }`. `watch` must coalesce source values,
retry after saturation, and invalidate queued callbacks when tokens are dropped.
Generation **and current query** checks reject stale completions, including a
query edit before its reaction runs. Latest cancellation cannot interrupt a
filesystem call already running; fixed workers and bounded queue limit overlap.
Pool/task rejection is visible and recoverable by rescan. Empty/failed results
clear selection; previous successful rows remain visible only during scanning.

All tokens live in View. An explicit app owner closes Store on View close; the
closing reaction captures only WeakView. Pool joins happen after native-loop exit.

## Integration and verification

The crate is a workspace member. `scripts/verify.sh` includes its tests, both
backends and strict smoke markers. Run the individual checks from `prototype-3`:

```sh
export RUSTC_WRAPPER=
export CARGO_TARGET_DIR=/tmp/wakui3-library-target
export RUSTFLAGS='-C target-feature=-crt-static -C prefer-dynamic=no'
export CARGO_BUILD_JOBS=2
cargo test -p wakui3-library --no-default-features --features backend-gtk
cargo build -p wakui3-library --no-default-features --features backend-gtk
GDK_BACKEND=x11 timeout 45s xvfb-run -a /tmp/wakui3-library-target/debug/wakui3-library --smoke
cargo test -p wakui3-library --no-default-features --features backend-tao
cargo build -p wakui3-library --no-default-features --features backend-tao
GDK_BACKEND=x11 timeout 45s xvfb-run -a /tmp/wakui3-library-target/debug/wakui3-library --smoke
```

There are 14 deterministic display-free tests using LocalPool/oneshots, including
stale query/generation rejection, errors/recovery, ingress=1, task/pool rejection,
token lifetime, Store ownership, bounded rows and actual non-writing temp scans.
Smoke ignores `--folder` and uses only its owned temporary fixture directory.
It drives adapter/native-loop entry, button and close events, then prints
`LIBRARY3 gtk PASS ...` or `LIBRARY3 tao PASS ...` and exits zero. A missing PASS
or incomplete smoke returns failure. Normal usage: `wakui3-library --folder PATH`.
