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
Replacing the Store remounts it. Duplicate source keys reject the whole snapshot
before changing live mounts and populate `error()` and `diagnostics()`.

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
Windows uses Tao's message hook and the current muda HACCEL table with
`TranslateAcceleratorW`; Linux uses the native GTK accelerator group.
Copy/Cut/Paste/SelectAll/About/Quit/Separator use actual muda predefined roles,
not aliases to the application dispatcher. Native roles cannot be activated by
`activate_menu`; that returns an explicit error. muda has no public enabled setter
for predefined items: disabled predefined actions and custom accelerators on
them are rejected. GTK3 muda does not implement Quit, so that role is explicitly
rejected on Linux; applications can supply a Normal close command.
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
`examples/native_smoke.rs` has a real `main` calling `run`; it opens/draws a native
window and menu, mounts an application-defined advanced widget that roundtrips
the system clipboard, awaits a worker completion, exercises automation and closes.
On Linux it also activates the real native accelerator group and observes/closes
the actual predefined About dialog. This is not synthesized physical keyboard input.
Run it in a separate process (e.g. under Xvfb on headless Linux). Other desktop
platforms require their own native verification; a Linux pass is not proof of
Windows/macOS IME or menu conformance.