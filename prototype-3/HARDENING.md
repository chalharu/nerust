# Review Follow-Up

This remains a standalone, pre-freeze prototype. Passing its gates is not approval
to replace the main application or debugger. No older prototype or application
module is changed by this follow-up.

## Correctness Changes

| Review concern | Resolution |
| --- | --- |
| Dirty-field omissions | Handwritten bind_scoped/update_scoped are conservative. Explicit optimization APIs have separate names and documented proof obligations. Generated field readers, writers and dependency names share one parsed path. Regression reproduces an omitted field without stale notifications. |
| Owning/shared model confusion | owned_view consumes a non-Clone StoreOwner. Acquisition fails if a shared Store handle already exists; compile-fail and runtime tests cover both boundaries. Sharing from an existing owner explicitly delegates final closure to it. |
| Write rejection ambiguity | try_set returns ReadOnly/Closed/Capacity/Conversion. Acceptance is not an application receipt. Acceptance bools are must_use; existing intentional discards are explicit. |
| Dropped continuation | Store fails closed and exposes dispatch_failed, rather than silently retaining a stranded FIFO. Hosts must still poll retained futures and avoid blocking callbacks. |
| Projection duplicates | map keeps its non-PartialEq contract; distinct is an explicit opt-in. |
| Backend Style errors | try_style validates spacing, finite dimensions and parent placement before mutation. Missing IDs reject, never alias root. style records errors; invalid GTK placement no longer panics after partial mutation. |
| Late cleanup | on_close invokes immediately after closure on GTK, Tao and AppKit. |
| Viewport writeback | Writable models receive effective last-page start; readonly models scroll locally. Fractional Tao overscan remains bounded at count+1. |
| Diagnostic locations | JSON also has source spans. YAML unknown fields use spanned keys. Wrapper selects primary expansion markers structurally, preserves CRLF byte positions and leaves ambiguous diagnostics unchanged. |
| AppKit editable rows | Native frames move without remounting retained key/Store pairs. Native regression preserves field-editor identity, marked text and selection through reorder. |
| AppKit distribution | @rpath bridge plus reproducible signed bundle script; verification relocates the bundle and runs smoke without DYLD overrides or absolute build rpaths. |
| Clipboard interference | Mutable clipboard probes require explicit opt-in and an isolated desktop. Synthetic input is not external IME certification. |
| Smoke readiness | Tao waits for the probe's actual draw notification, not a fixed 800ms delay. Formal runners retain a configurable hang watchdog. |
| Persistence | Cooperative writer lock with explicit RAII unlock, Unix directory sync, rejection/recovery and SIGKILL subprocess regressions. A duplicated/inherited file description cannot retain the lock after normal guard drop. Post-rename sync failure can mean publication already happened. |

## Supported Scope

| Capability | GTK/Linux | Tao | AppKit |
| --- | --- | --- | --- |
| Settings/library screens | Supported | Supported | Supported |
| Stack | Supported | Supported | Supported |
| Grid/Flex/Absolute | Supported with documented geometry limits | Supported with documented geometry limits | Unsupported |
| Items keyed retention | Supported | Supported | Supported, including editable reorder |
| Optional Content template | Supported | Supported | Unsupported |
| Coordinated multiwindow/attach | GtkHost | Unsupported | MacHost |
| Native controls | GTK | Software iced controls | AppKit |
| Full external IME/accessibility certification | Not certified | Preedit/accessibility limitations | Not certified |

One document describes one window. Window creation, ownership, reopen and shutdown
remain typed host/application composition, not a global YAML graph. Tao's unsupported
external-host entry rejects without constructing a window. Applications needing
multiple Tao windows must not select it until that capability is implemented.

The schema still has 12 control/layout variants. CheckBox, Radio, Slider, Combo,
Image, Tab, Dialog, arbitrary conditional/dynamic Style properties, recursive
menus and arbitrary Grid tracks are not added as speculative framework surface.
Native factories are explicit procedural extensions, not portable declarative
support. Select the next real screen and implement its required controls before
claiming that screen is portable. Grid already measures row heights; fixed
virtual-item height is a separate constraint.

Debugger/image migration is blocked on a representative main-application benchmark
preserving existing image-generation gates, surface lifetime, selection, editing,
auxiliary windows and no-change behavior. The 16MiB shared-payload scoped example
and settings/library projection workloads are CPU baselines, not that benchmark
and not compositor/input-to-presentation measurements.

## Deliberate Contracts

- is_writable reports writer capability, not current capacity/liveness. Closed
  bindings reject writes and may deliver one initial frozen value to a subscriber.
- Serial orders one slot; Latest cancellation wrappers consume bounded permits
  until polled/dropped. These are not globally serialized UI work or unbounded leaks.
- Human confirmation has no destructive automatic timeout. cancel_shutdown,
  force-close and drain timeout are separate operations; hooks must be observable
  and nonblocking. u64 exhaustion remains fail-stop rather than unsafe ID reuse.
- Keys are opaque nonempty IDs excluding slash. Constructor bypass is revalidated
  at backend boundaries. Whitespace/backslash are not filesystem traversal.
- Shortcut grammar supports multiple modifiers but not all punctuation/media keys,
  chords or global conflict resolution. Add keys and focus-scoped conflicts needed
  by an actual debugger before migrating its shortcuts.
- watch delivers initially synchronously and returns an RAII must_use token.
  Validation status delivery guarantees still require the same owning Store and
  positive ingress; cross-Store retry is not claimed to be independent of its owner.
- force-close, dropped registration tokens and leaked application captures remain
  explicit lifecycle operations. Session weak registration does not guarantee
  cleanup after application-created strong cycles or mem::forget.

## Verification

- Core/compile-fail: runtime, markup, macros, model and persistence suites. Diagnostic
  wrapper has six independent structured tests, including Unicode/CRLF/tab and ambiguity.
- Linux formal gate: scripts/verify.sh, separate Xvfb GTK4/GTK3 processes, mandatory
  XTest paths, explicit ignored/native tests, each requiring an executed test.
  The dedicated Prototype 3 GitHub workflow invokes verify-docker.sh on relevant
  pushes/PRs. Its hosted result must be checked after push; local success is not
  evidence that a remote workflow has already completed.
- macOS formal gate: scripts/verify-macos.sh, main-thread AppKit/Tao executables,
  settings/library smoke, editable reorder and both relocated bundles.
- Release CPU gates: scripts/measure.sh and scripts/measure-macos.sh. Existing
  environment budget overrides remain available; watchdogs are not latency SLOs.
- Bundle build: python3 scripts/bundle-macos.py --verify (or --package wakui3-library).
  Ad-hoc signing is not Developer ID signing, notarization or oldest-OS certification.
- Clipboard native probes require WAKUI_ALLOW_CLIPBOARD_PROBE=1. Formal native
  scripts opt in; run them only on an isolated desktop with no concurrent copy/paste.
- WAKUI_NATIVE_TIMEOUT_SECONDS defaults to 45 for native runners; AppKit retains
  WAKUI_MACOS_TIMEOUT_SECONDS. A timeout is a failed/hung test, not a skipped success.

Report per-suite executed/failed/ignored counts. Do not equate plain cargo test
with completion of native, RPA, release or distribution gates. Real external IME,
Full Keyboard Access, VoiceOver, hardware power loss, Windows replacement and
all-platform packaging remain explicit pre-release checks, not inferred successes.

## Verified Results

The follow-up passed these local gates on macOS Apple Silicon and Linux Docker:

- `sh prototype-3/scripts/verify-docker.sh`: image COPY/build path, strict Clippy,
  core/compile-fail suites, isolated GTK/Tao native gates and release CPU budgets.
- `sh prototype-3/scripts/verify-macos.sh`: core/compile-fail suites, strict Clippy,
  AppKit/Tao native gates, editable keyed reorder and both relocated signed bundles.
- `sh prototype-3/scripts/measure-macos.sh`: release drawing/projection workloads;
  these are informational CPU/synthetic-input observations, not physical latency SLOs.
- Runtime: 67 passed. Macro integration: 12 passed. Cargo UI: 11 passed.
  Diagnostic wrapper: six passed. Persistence: eight passed, including explicit
  unlock while a duplicated file description remains open and killed-writer recovery.
- Linux Tao unit suite: 28 passed, three ignored; the formal runner separately
  invokes display-dependent tests in isolated processes. The added GTK native
  Style/late-close contract test executed and passed, rather than matching zero tests.
- Dedicated GitHub workflow: actionlint 1.7.7 passed. Hosted CI must still be checked
  after push; these results describe local execution only.

The final Docker COPY-path run exposed a lock lifetime race when process spawning
overlapped file closure. Explicit unlock and a deterministic duplicated-descriptor
regression fixed it; the complete Linux and macOS formal gates then passed again.