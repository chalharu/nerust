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
spacing is sanitized to zero. A finite explicitly placed size is constrained by
the enclosing available size. Root styling uses `__root`; the first style on an
otherwise empty View also registers the generated Window ID as a root alias.

Items and Content invoke arbitrary typed recursive templates with persistent
item Stores. Local IDs scope as `list/key/local`; explicitly prefixed IDs also
work. The same key AND Store retain a mount across reorder/overlapping scroll.
Replacing the Store remounts it. Duplicate source keys, empty keys and keys containing
`/` reject the whole incoming snapshot before changing live mounts and populate
`error()` and `diagnostics()`. Items and Content both validate public `Keyed` fields.

Lists use actual iced scroll events, fixed-height rows and before/after spacers.
`Viewport.count` is clamped to 1..256, row height to 1..4096 logical pixels.
Fractional scrolling mounts at most one extra trailing row (maximum 257 mounts).
Only the mounted range owns widget bindings/commands. Eviction explicitly drops
subscriptions, including custom primitive tokens; source Stores are not closed.
`keep` called synchronously inside a template also belongs to that template mount.
Programmatic `viewport` and writable viewport bindings also move the real native
scroll widget. Read-only viewport bindings still permit local user scrolling.

`mount(parent,id,Rc<dyn Fn()->Element>,Vec<Subscription>)` is the application
primitive extension point. It accepts actual iced advanced widgets; factories
must be side-effect-free while rendering. Publish `Message::Custom(id)` and
register `on_custom` for callbacks, or use ordinary Edit/Click messages.
`meter` demonstrates a software primitive, not a platform-native progress widget.

## Menus and input boundaries

Menu snapshots and titles are reactive. Normal items parse muda accelerators
(e.g. `Ctrl+KeyS`) before replacing the native snapshot and dispatch their frozen
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
them are rejected. Quit uses muda's predefined role on Windows/macOS. GTK3 muda
does not implement Quit, so Linux has an explicit platform shim: a native normal
menu item with the Quit role and a stored weak-owner closure calling `View::close`,
never the application dispatcher. This shim also supports enabled state and
`activate_menu` (including native event delivery); custom Quit shortcuts remain
rejected consistently with predefined roles. It closes the owning View, not other
Views or unrelated GTK windows.
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
The pure transaction test injects failures before/after every remove/append.
The Linux display test also provokes an actual GTK append error (`Ctrl+Fn`),
checks old native IDs/order/hotkeys/actions and title preservation, then activates
the native Quit shim. It is ignored by default to isolate GTK thread ownership;
from `prototype-2`, run it separately with
`xvfb-run -a cargo test -p wakui2-tao --lib tests::native_menu_append_failure_restores_accelerators_registry_and_quit -- --exact --ignored --test-threads=1`.
`G_DEBUG=fatal-criticals` can additionally enforce clean GTK diagnostics.
`examples/native_smoke.rs` has a real `main` calling `run`; it opens/draws a native
window and menu, mounts an application-defined advanced widget that roundtrips
the system clipboard, awaits a worker completion, exercises automation and closes.
On Linux it also activates the real native accelerator group and observes/closes
the actual predefined About dialog, then activates native Quit and checks the close
callback. This is not synthesized physical keyboard input.
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