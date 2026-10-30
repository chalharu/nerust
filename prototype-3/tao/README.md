# wakui2-tao

Implements the frozen `../API.md` View API using software iced/tiny-skia controls,
a real Tao desktop window and native muda menus. It is not an OS-native control
toolkit or an accessibility implementation.

## Layout, templates, ownership

Stack, Grid (zero-based row/column and spans), wrapping Flex and Absolute have
distinct advanced-widget layout paths. Placement width/height, outer margins and
inner padding use logical pixels. Grid uses the supplied row/column, including
the default cell (0,0); it does not silently substitute a stack or auto-flow.
Grid columns and row indices/spans are safety-bounded to 4096. Invalid/nonfinite
spacing is rejected by `try_style` before mutation. A finite explicitly placed size is constrained by
the enclosing available size. Root styling uses `__root`; missing IDs never
silently become aliases. `style` records validation failures in diagnostics.

Common subset with GTK: Grid fills the available logical width with equal tracks
of `(available - (columns-1)*gap)/columns`; children without explicit widths fill
their track/span (minus margins), and spans include internal gaps. Explicit rows,
columns and row/column spans are honored; unused rows still contribute gaps.
Portable code supplies explicit placement, because GTK's unstyled children keep
auto-flow while Tao's default is cell 0,0. GTK native minimum sizes and integer
rounding can enlarge tracks or constrain window sizes; Tao bounds explicit sizes
to available space. Neither backend promises pixel identity. Flex's common subset
is only gap and horizontal wrapping, not MAUI-complete grow/shrink/alignment or
identical line breaking. `layout::tests::actual_grid_nodes_fill_equal_columns_with_gap_and_explicit_spans`
checks real iced child nodes at two available widths, not just a geometry helper.

Items and Content invoke arbitrary typed recursive templates with persistent
item Stores. Local IDs scope as `list/key/local`; explicitly prefixed IDs also
work. The same key AND Store retain a mount across reorder/overlapping scroll.
Replacing the Store remounts it. Duplicate source keys, empty keys and keys containing
`/` reject the whole incoming snapshot before changing live mounts and populate
`error()` and `diagnostics()`. Items and Content both validate public `Keyed` fields.

Lists use actual iced scroll events, fixed-height rows and before/after spacers.
`Viewport.count` must be 1..256, row height finite and 1..4096 logical pixels,
using the shared runtime validator (also used for static literals and GTK/AppKit).
Invalid updates report a diagnostic and preserve mounts; an invalid initial
viewport mounts nothing. A valid update recovers. Starts beyond the last full
page select that last page, using the common overflow-safe range calculation.
Fractional scrolling mounts at most one extra trailing row (maximum 257 mounts).
Only the mounted range owns widget bindings/commands. Eviction explicitly drops
subscriptions, including custom primitive tokens; source Stores are not closed.
`keep` called synchronously inside a template also belongs to that template mount.
Programmatic `viewport` and writable viewport bindings also move the real native
scroll widget. Read-only viewport bindings still permit local user scrolling.
Writable viewport bindings receive the effective normalized start, including
after a source shrink or an external out-of-range update.

`mount(parent,id,Rc<dyn Fn()->Element>,Vec<Subscription>)` is the application
primitive extension point. It accepts actual iced advanced widgets; factories
must be side-effect-free while rendering. Publish `Message::Custom(id)` and
register `on_custom` for callbacks, or use ordinary Edit/Click messages.
`meter` demonstrates a software primitive, not a platform-native progress widget.

## Menus and input boundaries

Menu snapshots and titles are reactive. Shortcuts first parse shared runtime
`Shortcut` grammar: `Primary+S`, `Primary+Q`, `Ctrl+Shift+S`. Modifiers are
Primary/Ctrl/Cmd/Alt/Shift before one ASCII letter/digit, F1..F12, or
Enter/Tab/Space/Escape/Backspace/Delete/Insert/Home/End/PageUp/PageDown/Left/Right/Up/Down.
Tokens are case-insensitive and trimmed; Shift is explicit. Primary is Cmd on
macOS, Ctrl elsewhere. `to_gtk`/`to_muda` and the explicit-target `*_for` variants
produce backend syntax. If common parsing fails, existing muda accelerator input
(e.g. `Ctrl+KeyS`) is accepted as **legacy backend-specific syntax**; its broader
accepted vocabulary is not common grammar. GTK keyvals and muda Code accelerators
do not establish physical-key equivalence for all keyboard layouts.
Normal items dispatch their frozen
string argument. Automation accepts `menu/item` or an unambiguous item ID.
Native remove/append failures restore the old items, order and accelerators before
returning an error; action and native-ID registries commit only after native success.
An offscreen Submenu/root insertion is not an atomic swap: muda can fail during
descendant attachment before registering the new child. Rollback uses the existing
submenu (preserving its reactive title). If restoration itself fails, diagnostics
report both errors; the old registry remains, but native state may be degraded.
Windows uses Tao's message hook and the current muda HACCEL table with
`TranslateAcceleratorW`; Linux uses the native GTK accelerator group.
Copy/Cut/Paste/SelectAll/About/Separator use actual muda predefined roles,
not aliases to the application dispatcher. Native roles cannot be activated by
`activate_menu`; that returns an explicit error. muda has no public enabled setter
for predefined items: disabled predefined actions and custom accelerators on
them are rejected. **Quit is the deliberate exception:** all platforms use an
ordinary native muda item with a weak-owner closure calling `View::request_close`,
never the application dispatcher. It supports enabled state, `activate_menu` and
custom shortcuts such as `Primary+Q`. The unsafe-to-veto predefined Quit is never
installed: muda 0.21.2 directly calls PostQuitMessage on Windows and uses AppKit
`terminate:` on macOS, bypassing a reliably cancellable application MenuEvent.
Windows/macOS normal-item event/accelerator delivery is **unverified natively**;
Linux tests must not be interpreted as guarantees for those systems. OS global
termination/application-menu quit and forced process exit are outside this API.
GTK3 predefined editing actions send GTK key sequences toward the focused native
window; their propagation into software iced inputs is not a verified conformance
claim. Native About receives metadata so it opens muda's real OS dialog.

`iced_winit::Clipboard` takes a winit window and cannot accept a Tao window. This
adapter connects the same `window_clipboard` implementation to the Tao display
handle, retaining its owner for safety. Connection failure is diagnosed rather
than replaced by an application-local clipboard. Clipboard service availability
and primary-selection support depend on the platform/display server.

Tao 0.37 exposes `ReceivedImeText` but **no public preedit event**. Committed text
is forwarded (without duplicating Linux key text); IME candidate positioning uses
the iced cursor. In-place preedit, physical-key identity, complete nonediting key
coverage and OS accessibility are not claimed. `input_capabilities()` reports
these limits; it describes adapter support, not clipboard-service availability.

## Loop and verification

`run` owns one UI-local future scheduler and one native loop. Worker wakeups cross
the event proxy as coalesced numeric IDs, with a one-shot GTK idle nudge on Linux.
Idle waits are event-driven; the only deadlines come from actual iced redraw
requests (such as caret blinking), not a runtime polling timer. Close/drop owns
task cancellation, subscriptions, close callbacks and native menu detachment.
`View::downgrade() -> WeakView` is non-owning; `upgrade() -> Option<View>` returns
None after close/drop. `on_close_request(Rc<dyn Fn()>)` replaces one app handler.
`request_close()` invokes it or force-closes by default. Native CloseRequested and
ordinary Quit items use that same entry point; Destroyed/error/owner teardown use
forceful `close()`. Nested synchronous requests inside a handler are ignored, not
queued; later requests retry. Panics are diagnosed and release the request gate.
`close()` stays idempotent, bypasses the request handler, and runs existing final
`on_close` cleanup once. Register cleanup before closure (Tao retains its existing
immediate invocation for a late on_close registration).

Settings pattern: on_close_request captures only a Store/Command which starts
confirmation/save and eventually publishes close_ready. Subscribe using WeakView,
upgrade and call close only when ready; retain that token with `View::keep`.
An on_close callback closes the Store when the View actually ends. Never capture
View strongly in a callback/token stored on that same View. keep ownership and
template-scoped token eviction are unchanged.
`run_async` returns an unsupported-external-host error without constructing a View.
Call `run` on the main thread, at most once per process. Do not initialize GTK4
in the same process, or install a competing process-global muda event handler.

Diagnostics/automation: `error`, `diagnostics`, `frame_count`, `mount_identity`,
`subscription_count`, and the frozen edit/click/menu/text/keys/count/viewport API.
Automation routes through the same message handlers as iced/native menu commands;
it is not a claim that OS pointer/key events were synthesized.

Unit tests cover geometry, bounded ranges, typed recursive templates, mount
identity, source replacement, eviction cleanup, canonical readonly text,
shortcut validation, scheduler worker wake/turn bounds, and explicit limits.
Native-View tests construct muda menus and cannot run on macOS libtest worker
threads; compile checks and pure range tests are not a substitute for their
Linux/X11 execution. AppKit has its own main-thread conformance runner.
The pure transaction test injects failures before/after every remove/append.
The Linux display test also provokes an actual GTK append error (`Ctrl+Fn`),
checks old native IDs/order/hotkeys/actions and title preservation, then activates
the native Quit shim. It is ignored by default to isolate GTK thread ownership;
from `prototype-3`, run it separately with
`xvfb-run -a cargo test -p wakui2-tao --lib tests::native_menu_append_failure_restores_accelerators_registry_and_quit -- --exact --ignored --test-threads=1`.
`G_DEBUG=fatal-criticals` can additionally enforce clean GTK diagnostics.
`examples/native_smoke.rs` has a real `main` calling `run`; it opens/draws a native
window and menu, mounts an application-defined advanced widget that roundtrips
the system clipboard, awaits a worker completion, exercises automation and closes.
On Linux it also activates the real native accelerator group and observes/closes
the actual predefined About dialog. It sends a real native window close request
(vetoed), activates the native Quit item (vetoed), then activates the native
Primary+Q accelerator to publish close_ready and close exactly once. Save uses
Primary+S. This is not synthesized physical keyboard input; Windows/macOS paths
use API automation and are not native verification of close/accelerator delivery.
Run it in a separate process. For headless Linux, explicitly select X11:
`timeout 45 xvfb-run -a env GDK_BACKEND=x11 G_DEBUG=fatal-criticals /tmp/wakui2-target/debug/examples/native_smoke`
(after building the example with that target directory). `xvfb-run` sets `DISPLAY`
but does not disable an inherited Wayland connection: GTK may still choose
`GdkWaylandDisplay`, bypassing Xvfb entirely. In that case the real Wayland
clipboard requires compositor-provided seat events and keyboard focus; without
them, reads can fail with `no events received on any seat` or
`client doesn't have focus`, stopping the smoke before menus and Quit. This is
not a passing or skipped clipboard test. The adapter does not replace it with a
local cache or force all applications onto X11. A complete X11 smoke pass does
not verify Wayland clipboard access in an unfocused session. Other desktop
platforms require their own native verification; a Linux pass is not proof of
Windows/macOS IME or menu conformance.

## macOS verification

From the prototype workspace root, run `sh scripts/verify-tao-macos.sh` on an
interactive macOS desktop. It removes inherited DYLD search/injection variables
only for its processes, checks strict Clippy without lint exclusions, and runs
the main-thread native smoke, gallery, settings and library in separate processes.
Native runs have a 45-second deadline; a failed run prints its captured output
and fails the gate. `scripts/verify-macos.sh` includes this Tao gate after AppKit.
View-creating libtest tests still require Linux/X11: a single libtest worker is
not the macOS main thread.

On 2026-10-31, Apple Silicon/macOS 15.7.5/Rust 1.99 passed
`VERIFY3 TAO MACOS PASS`: a real Tao window drew software iced controls, an
advanced widget roundtripped the system clipboard, a worker woke the UI, and
gallery/settings/library automation completed including persistence and close.
The non-Linux Proxy path now passes strict Clippy; Linux retains its one-shot
GTK wake only after successful event submission. The same source also passed
the Docker/Linux X11 native input/menu/close suite and release CPU budgets.

The macOS smoke's close/menu requests are API automation, not synthesized OS
keys or native accelerator/WM-delivery conformance. External IME, preedit,
VoiceOver, Full Keyboard Access, multiwindow and compositor latency remain
unverified/unsupported as documented above. A passing native draw/clipboard
smoke does not remove those boundaries.