# Review Follow-Up

This remains a standalone, pre-freeze prototype. Passing its gates is not approval
to replace the main application or debugger. No older prototype or application
module is changed by this follow-up.

## Main Application Requirements

Integration into nerust is an agreed objective, not an undecided use case.
The following are required migration gates. A standalone settings/library smoke
does not satisfy the application, image or multiwindow rows by itself.

| Existing application boundary | Required prototype behavior |
| --- | --- |
| gui/frontends/view-defs/src/menu.rs | File/Emulation/States/Help and nested Select/Save/Load/Delete slot menus; stable command IDs, typed arguments and reactive enablement. |
| gui/frontends/view-defs/src/application/auxiliary.rs | Settings, History, PPU and Debugger under one host/event loop; independent close/reopen, generation-checked work, shared model ownership and coordinated shutdown. Tao support is mandatory. |
| Widget settings and debugger composition | Reactive visibility/enablement, retained editing and selection, required layout/control properties and typed native extension points. |
| PPU display snapshots and debugger sessions | Shared RGBA frames, image-generation gating, retained surfaces and no unnecessary pixel comparison/upload on unchanged model updates. |
| Main-application performance | Representative image, editing, selection and auxiliary-window workload with executable Linux and macOS CPU budgets; no-change behavior is a separate assertion, not inferred from settings projection speed. |

Window creation and application/domain state remain typed host composition.
A multiwindow YAML graph is not required to preserve the existing application
ownership boundary. Unsupported mandatory capabilities block migration rather
than becoming optional backlog. External IME/accessibility and distribution
certification still require their named platform environments and credentials.

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
| Coordinated multiwindow/attach | GtkHost | TaoHost under one owned loop | MacHost |
| Native controls | GTK | Software iced controls | AppKit |
| Full external IME/accessibility certification | Not certified | Preedit/accessibility limitations | Not certified |

One document describes one window. Window creation, ownership, reopen and shutdown
remain typed host/application composition, not a global YAML graph. TaoHost keeps
one WindowId-routed loop, per-window painter/clipboard/surface state and a shared
scheduler. Close/reopen and coordinated veto/drain are native-tested on Linux/macOS.
Its legacy external-reactor entry still rejects instead of creating a nested loop.

The schema has 19 control/layout variants. CheckBox, Radio, Slider, Combo, Image,
Tab and Dialog are required by nerust and implemented on GTK, Tao and AppKit.
The shared controls example uses actual backend widgets and typed bindings;
Tao uses software iced widgets and a nonblocking modal overlay, not OS controls.
Reactive visibility/enablement, full Style bindings and recursive typed menus are
supported. Arbitrary CSS patches, dynamic Tab-page graphs, editable Combo and
arbitrary Grid tracks remain outside the required fixed-page subset. Grid measures row
heights; fixed virtual-item height is a separate constraint.

The mandatory integration example models the existing main/settings/PPU/debugger
boundaries with four native windows, eight shared NES-size images, generation
changes, retained editing/selection and auxiliary reopen. It directly checks native
image setter counts on no-change updates. Release budgets are p50 <= 5ms and
p95 <= 10ms per edit/selection/update cycle on both Linux and macOS, configurable
with WAKUI_INTEGRATION_P50_NS/WAKUI_INTEGRATION_P95_NS. This boundary workload does
not execute the emulator core and is not compositor/input-to-presentation latency.
The actual session/renderer bridge still needs its application migration tests;
the 16MiB scoped and settings/library workloads remain separate CPU baselines.

## Common Rejection Contract

The shared controls executable separates raw model data, display, acceptance and
diagnostics. Formal gates run the same probe with actual GTK, Tao and AppKit controls.

| Case | Model | Display | Result |
| --- | --- | --- | --- |
| Slider step normalization | 42.4 before input | 42.0 | Valid writes normalize before acceptance |
| Slider Closed/Capacity/Conversion | Unchanged 42.4 | Retained 42.0 | slide Err; try_set preserves typed WriteError |
| readonly/disabled/hidden | Unchanged | Retained, without remount | Err; enabled=true does not add a writer |
| unknown literal Tab page | Not constructed | Partial View closed | Parser error at selected property |
| unknown bound initial Tab page | Shared Store unchanged; consumed owner closed | Partial View closed | try_view/try_owned_view Err |
| empty Tab choice/readonly select | Empty remains empty; readonly unchanged | First page/last accepted page | readonly select Err |
| unknown Combo/Tab operation choice | Unchanged | Retained | Err before mutation |
| nested menu disabled parent | Unchanged | Retained tree | Leaf activation Err; typed argument preserved when enabled |
| Dialog readonly/hidden parent | Open flag unchanged false | No modal opened | set_open Err |
| Dialog open/child dismiss/reopen | Open flag true/false follows accepted operations | Native modal or Tao overlay | Commands and set_open accept; close then reopen verified |

Dialog open/close is an explicit model operation; GTK's hidden placeholder is not
a user visibility request. Native modal appearance is not required to match Tao's
overlay. Operation acceptance is not a queued write's application receipt. View
error strings and native appearance are not promised byte-identical across backends.

Shutdown confirm/cancel/drain/close panic injection verifies all remaining windows
close and failures remain observable. Forced close stays idempotent. Library scan
keeps the lowest 256 Paths independently of enumeration order within the 8192
visit budget; capped reports do not promise a complete folder snapshot. Linux tests
real non-UTF8 filenames; Unix also tests lossless selection keys independently of
filesystem support. Metadata-only scans preserve content and mtime.

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

## Integration Follow-Up Results

The reviewed follow-up passed verify-docker.sh and verify-macos.sh, including
strict Clippy, required native executables, both relocated/ad-hoc signed bundles
and release CPU budgets. Expected panic-injection messages are followed by the
recovery PASS markers; no formal gate failed on this final revision.

| Suite | Linux | macOS |
| --- | --- | --- |
| Runtime | 72 passed, 0 ignored | 72 passed, 0 ignored |
| Macro unit / integration / Self | 14 / 12 / 1 passed | 14 / 12 / 1 passed |
| Downstream Cargo UI | 13 passed, 0 ignored | 13 passed, 0 ignored |
| Markup unit / parser | 3 / 16 passed | 3 / 16 passed |
| Tao library | 34 passed, 4 ignored; native cases run separately | Main-thread native executables, not worker-thread UI tests |
| Settings model | 30 passed; GTK 3/Tao 1 ignored for separate native/perf execution | 30 passed, 1 separately executed perf case |
| Library model | 16 passed, 1 separately executed perf case | 15 passed, 1 separately executed perf case |

The same controls executable requires CONTRACT3, MENUS3, DIALOG3 and TABS3 markers
on all backends. GTK also checks native separator sections. Tao checks real drawn
multiwindow close/reopen and coordinated Quit, plus retained hide/show widget state
and zero-size hidden layout. AppKit conformance covers its native editor/menu paths.

| Four-window/eight-image release workload | p50 ns | p95 ns |
| --- | --- | --- |
| Linux GTK | 125542 | 140000 |
| Linux Tao | 10625 | 13084 |
| macOS AppKit | 2617834 | 2990042 |
| macOS Tao | 9542 | 13000 |

Each run checks eight initial native image acceptances, generation replacement,
no-change setter suppression, editing/selection and auxiliary reopen. Budgets are
5000000/10000000 ns; these are CPU cycles, not image-upload or physical-input SLOs.
Hosted CI is a separate post-push observation. External IME/accessibility, Windows,
Developer ID/notarization and actual main-application migration remain unclaimed.

## Prior Baseline Results

The follow-up passed these local gates on macOS Apple Silicon and Linux Docker:

- `sh prototype-3/scripts/verify-docker.sh`: image COPY/build path, strict Clippy,
  core/compile-fail suites, isolated GTK/Tao native gates and release CPU budgets.
- `sh prototype-3/scripts/verify-macos.sh`: core/compile-fail suites, strict Clippy,
  AppKit/Tao native gates, editable keyed reorder and both relocated signed bundles.
- `sh prototype-3/scripts/measure-macos.sh`: release drawing/projection workloads;
  these were informational CPU/synthetic-input observations on that baseline.
  The current runner additionally enforces shared CPU budgets and native integration contracts.
- Runtime: 67 passed. Macro integration: 12 passed. Cargo UI: 11 passed.
  Diagnostic wrapper: six passed. Persistence: eight passed, including explicit
  unlock while a duplicated file description remains open and killed-writer recovery.
- Linux Tao unit suite after the layout follow-up: 31 passed, four ignored; the formal runner separately
  invokes display-dependent tests in isolated processes. The added GTK native
  Style/late-close contract test executed and passed, rather than matching zero tests.
- Dedicated GitHub workflow: actionlint 1.7.7 passed. Hosted CI must still be checked
  after push; these results describe local execution only.

The final Docker COPY-path run exposed a lock lifetime race when process spawning
overlapped file closure. Explicit unlock and a deterministic duplicated-descriptor
regression fixed it; the complete Linux and macOS formal gates then passed again.

## Layout Follow-Up

The subsequent visual review found real issues, not just transient test windows:

- GTK settings stretched horizontal button frames, while Tao retained text widths.
  Tao now shares surplus width and forwards frame constraints to the control body.
- Fill entries/meters could crush a sibling button in gallery rows. Their initial
  measurement is now bounded; a real-widget regression covers three widths.
- Short GTK/Tao windows had no outer content scroller. Both now keep menus fixed
  and scroll the content. GTK labels and Tao labels/buttons wrap long words.
- Tao 0.37.2's Linux discrete wheel path reads smooth deltas, producing zero movement
  for XTest wheel buttons. A GTK3 boundary bridge preserves discrete directions;
  a native regression checks the displayed content position, not just row data.
- Gallery virtual rows now reserve 64 logical pixels for wrapped controls. AppKit's
  editable conformance row had a label plus editor in 32 pixels; it now uses 56,
  with a 56/72/56 no-remount regression. Its old capture really did clip labels.

Verification on this revision:

- Actual settings YAML: four projection states at 440/660/960 logical widths,
  16 controls each, frame/body width checks, horizontal bounds and pairwise
  nonintersection. GTK uses native allocations and verifies bottom reachability
  at height 360. Tao uses real iced widget nodes, not geometry-helper mocks.
- XTest settings captures: initial, invalid and exit-confirmation states at
  660x520, 960x520 and 440x360 on both backends, plus the six short-window bottoms.
  OCR waits for the expected state; repeated identical pixels alone can still be
  an old frame. OCR is a readiness aid, not proof that every glyph is visible.
- Gallery/library: both backends, the same three sizes, top and bottom captures;
  gallery bottom waits for its Absolute caption. AppKit 640/960 conformance images
  were also inspected after the editable-row correction. No unintended control
  overlap or inaccessible bottom control remained in these checked cases.
- `verify-docker.sh` and `verify-macos.sh` passed, including strict Clippy, native
  regressions, input paths, release CPU gates on Linux and relocated macOS bundles.
  One direct AppKit conformance invocation hit its reaction-chain input-starvation
  assertion; the subsequent complete formal macOS run passed. This remains a test
  timing caveat, not evidence that arbitrary input latency is certified.

Regenerate settings images on an isolated X11 desktop with xdotool, ImageMagick
and Tesseract installed: `python3 scripts/review-settings-layout.py` from this
workspace. Images and `captures.json` are under ignored `target/layout-review/`.
The native geometry/wheel regressions are mandatory in `scripts/verify.sh` and do
not depend on OCR. Native metrics, narrow-width line breaking and theme colors
can differ; row structure, width distribution and reachability are the shared
contract. Arbitrary explicit Absolute overlap or undersized custom templates are
not automatically repaired by these layout rules.

## Required Controls Follow-Up

- CheckBox bool, Radio stable key, stepped Slider f64, Combo Choice IDs, shared
  RGBA ImageFrame, retained Tab pages and nonblocking Dialog content are declarative.
  Display-only TwoWay properties reject at their markup mode location.
- Layout values are finite and bounded at 1,000,000 logical pixels; Grid columns
  are 1..=4096. Combined margin/padding overflow rejects before native conversion.
  try_layout rejects unavailable/foreign parents and duplicate IDs before mutation;
  AppKit rejects unsupported layouts instead of returning a dangling Parent.
- Shutdown cancellation aborts pending window waits before another work poll.
  AppKit close-request flags recover after panic, preserve the payload, and
  accepted no-op converter writes read back the canonical display value.
- Formal Linux/macOS gates execute the same seven-control example, including
  readonly rejection, slider normalization, invalid choices, Tab construction,
  foreign/duplicate layouts and closed-view rejection. AppKit Tab registries are
  released on row eviction; GTK Combo avoids same-value notify cycles.
- Persistence injects PermissionDenied, StorageFull and Other at five IO stages
  (15 conditions), checking complete old/new JSON, owned-temp cleanup and recovery.
  These are fault injections, not real disk exhaustion or hardware power-loss proof.
- XTest captures cover GTK/Tao 640x660, 960x660 and 440x360, top/bottom and a
  physically opened modal. Resize capture waits for Tao's actual draw marker;
  unchanged old pixels alone are not readiness. OCR is supplemented by image
  inspection and modal pixels. Native minimum sizes and image allocation differ
  from Tao's fixed frames; checked controls remain reachable and do not overlap.
- AppKit 640/960 RGBA/control captures and the native control probe were checked.
  Full external IME, accessibility, Developer ID/notarization, Windows replacement
  and hardware power-loss gates remain manual pre-release requirements.