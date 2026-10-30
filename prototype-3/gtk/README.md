# GTK4 backend

Implements the frozen `../API.md` View/Parent API using GTK4 widgets, Gio menus
and actions, and a GLib-local executor. No dependency on prototype_1.

## Native extensions

The crate re-exports `gtk`. Application factories can construct any GTK widget
and call `view.mount_native(parent, id, &widget)`. `view.bind_native(id, binding,
&object, apply)` subscribes with a weak object reference. Register custom native
signals with `view.track_signal(id, &object, signal)` for automatic disconnection.
Factories must not capture View strongly in their signals or binding callbacks.
`meter` mounts a genuine `gtk::ProgressBar`.

Entry writeback uses `notify::text`, coalesced by GTK across `set_text`'s
delete/insert replacement, rather than rolling back during `changed`'s temporary
empty value. Conversion and canonical rollback finish synchronously, without an
idle turn. Native insert/delete edits still write immediately; model updates and
canonicalization do not echo back, and read-only entries remain noneditable.

Every mounted control has a Box frame: margins apply outside the frame, padding
is frame-local CSS, and placement applies to the frame. Stack is a GTK Box, Grid
is a GTK Grid with real attachment spans, Flex is a wrapping FlowBox, and
Absolute is a Fixed. Grid/Absolute coordinates on other layouts explicitly panic
rather than silently falling back. Sizes and spacing are nonnegative finite
logical pixels, rounded to integers; GTK size requests are minimum sizes, not
hard constraints on a widget's natural minimum. Padding works on leaves too.

### Common layout subset (also implemented by Tao)

Grid uses homogeneous columns and expands across the available logical width.
For `n` columns and gap `g`, a track is `(available - (n-1)*g)/n`; a column span
occupies its tracks plus internal gaps. Empty declared columns retain a track.
Explicit zero-based row/column and positive row/column spans are supported.
GTK's unstyled children retain its existing auto-flow; portable code supplies
explicit placement for every Grid child (Tao defaults to cell 0,0).
GTK native minimum sizes can enlarge tracks/window; integer allocation rounds
tracks, so one logical pixel of difference is expected. Explicit width/height
are minimum requests on GTK, not hard clipping constraints. Use feasible sizes.
Flex's common contract is horizontal flow with gap and wrapping only: GTK FlowBox
and iced may distribute space and choose line breaks differently. No full MAUI
layout, flex grow/shrink/alignment, or cross-backend pixel identity is claimed.

## Templates and virtualization

`items<T>` and `content<T>` accept the exact typed recursive callback from
API.md. The callback receives the original source Store and a prefix; append
`/{localid}` to that prefix, including the template root. Nested items/content
can instantiate different item types. Internal callbacks own only weak View
references. `keep` inside template construction belongs to that template scope
(useful for validation tokens); outside templates it belongs to the whole View.

Rows are clipped fixed-height native scrolled windows, with top/bottom spacers
and a real scrolling adjustment. At most `Viewport.count` rows are mounted per
list; no overscan. Source Stores survive eviction, and visible matching Stores
survive reorder. Eviction removes all descendant widgets, lists, menu actions,
subscriptions and signals before ID reuse. Duplicate keys reject the complete
snapshot before mutation. `items_error(id)` exposes invalid snapshots.
Variable-height rows are intentionally unsupported. The shared runtime validator
requires count 1..256 and finite row height 1..4096 logical pixels, as for
Tao/AppKit and proc-macro literals. Invalid viewport updates preserve mounts;
an invalid initial viewport mounts nothing. Valid updates clear the viewport
error without clearing independent item-key errors. Out-of-range starts show
the last full page (or all rows for a shorter source). IDs and keys form hierarchical
paths; applications must avoid ambiguous overlapping scope paths.

## Menus

Snapshots rebuild real Gio actions, GMenu sections (separators), and GTK
ShortcutController/NamedAction shortcuts. Shortcuts first use the shared runtime
`Shortcut` grammar, e.g. `Primary+S`, `Primary+Q`, `Ctrl+Shift+S`. Modifiers
`Primary`, `Ctrl`, `Cmd`, `Alt`, `Shift` precede one ASCII letter/digit, F1..F12,
or Enter/Tab/Space/Escape/Backspace/Delete/Insert/Home/End/PageUp/PageDown/
Left/Right/Up/Down. Case and token-edge whitespace are normalized. Primary maps
to Cmd (GTK Meta) on macOS, Ctrl elsewhere; Shift is always explicit.
`Shortcut::parse`, `to_gtk`, `to_muda` share conversion; `*_for(ShortcutTarget)`
allows explicit target conversion independently of the build host.
If common parsing fails, existing GTK trigger syntax (`<Control>s`) is accepted
as **legacy backend-specific input**, not portable grammar. The legacy parser
may accept more keys/modifiers than the strict common parser. Invalid shortcuts
and duplicate IDs reject the snapshot. Normal
items dispatch their String argument; built-in roles operate on native GTK
focus/clipboard, open a native AboutDialog, or request View closure. Clipboard roles
require a focused widget that implements the respective GTK action. No claim is
made about OS-wide shortcut registration, all IME paths, or accessibility
conformance merely because GTK widgets are used.

## Loop ownership and shutdown

`run(make)` initializes GTK and owns a native Application loop. For an existing
GTK loop, initialize GTK on that thread and retain `GtkHost::attach()`; obtain its
`executor()` without starting a nested loop. `native_window()` exposes the real
window for setting an existing Application. `register_view(&session, id, &view)`
registers and retains a native View; the `UiHost` implementation accepts runtime
`run_async` sessions and presents registered windows without blocking. The caller
continues to drive its GTK loop. Session hooks use weak View references.

Default registered View hooks immediately confirm and have no app work to drain;
register application cancellation/drain hooks with Session separately. View
`on_close` callbacks are additive and should close application Stores. Dropping
View disconnects signals/subscriptions; closing/dropping GtkHost aborts all its
tasks. `attach_view(application, view)` is the single-window convenience used by
`run`, and also closes the host executor when that View closes; use
`native_window().set_application(...)` for multiwindow attached hosts instead.
Task completion queues a coalesced, one-shot idle callback to release GLib
JoinHandles, even without another spawn. Tasks and cleanup callbacks refer to the
executor weakly; handle/Source destruction runs outside registry borrows. There
is no permanent idle source or periodic polling timer. Registered window closure
also queues cleanup of closed sessions; task completion does the same.

### Opt-in coordinated native close

For each window, call `host.register_view(&session, "main", &view)` and then
`host.coordinate_close(&view, &session, Duration::from_secs(10))?`. Repeat with
unique IDs for every View sharing the Session, before attaching/presenting it.
Without `coordinate_close`, native close keeps its original synchronous behavior.
The host must outlive the windows and the caller must keep driving the GTK loop.
For multiwindow Applications, set each `native_window().set_application(...)`;
do not use the single-window executor-closing `attach_view` convenience.

Coordination is the fallback for `request_close` (including native/menu close);
it never registers or replaces
Session hooks. Application pattern: separately register a `WindowHooks` token
under an ID such as `"0-application-work"` and retain it in the application scope
(or `view.keep(token)` for View-scoped work). Its `confirm` asks the user, `cancel`
signals workers/Stores, and `drain` returns a future waiting for their completion;
`close` performs any remaining application teardown. `register_view` supplies the
weak, force-closing native hooks independently. Alternatively, register your own
complete per-window hooks with Session, retaining the tokens yourself and using
weak View captures in `close`; no `register_view` call is required in that case,
but the application must own/present those Views itself. Never capture View
strongly in a token stored on that same View.

Native close stops GTK's default synchronous destruction and schedules one
request per requesting View. Repeated requests while queued or shutting down
are ignored; a concurrent external request can return runtime `Busy`, which is
treated as a harmless rejection. Confirmation denial or `cancel_shutdown()`
leaves every View open and allows retry. Cancelling during drain cannot undo
application work already cancelled. During confirmation/drain, GTK keeps running
so dialogs, oneshots and worker completions can progress. The deadline covers
both phases and force-closes all registered windows on timeout. Hook tasks hold
no strong View/executor references; View teardown aborts its pending request and
host teardown drops pending tasks. A dead executor rejects native requests and
leaves the View open for explicit owner teardown.

### Ordinary close and weak application handles

`View::downgrade() -> WeakView` and `WeakView::upgrade() -> Option<View>` expose
non-owning handles. Upgrade returns None once the View is closed or dropped.
`on_close_request(Rc<dyn Fn()>)` replaces one application handler, not a list.
`request_close()` calls that handler; without one it uses `coordinate_close`,
or immediately closes by default. The application handler takes precedence over
the coordinator; registering both does not automatically run both mechanisms.
Native WM close and `MenuRole::Quit` (including a shortcut) use request_close.
Requests synchronously nested in that callback, including nested native/menu
requests, are ignored rather than queued. Later requests can retry. A panicking
handler is recorded in the View error and leaves the window open; retry is allowed.

For a settings application, let the request handler capture only a Store/Command
that starts confirmation/save and publishes `close_ready`. Subscribe to that flag
using a captured WeakView and call `upgrade().map(|view| view.close())` when ready.
Retain the subscription with `view.keep(token)`; it is owned until View shutdown
(or template eviction if registered inside a template). Register `on_close` once
to close the Store when the app actually ends, not when a request is vetoed.
Do not capture a strong View in any callback/token retained by that same View.

`View::close()` remains forceful and idempotent: use it in runtime final close
hooks or on close_ready, not to ask for confirmation. It never reemits a request,
and existing `on_close` callbacks run once. `GtkWindow::destroy`, Application
quit/process termination and forced owner teardown are not vetoable requests.

## Native verification

`src/tests.rs` contains GTK-thread-local tests intended for Xvfb with
`GSK_RENDERER=cairo`, `G_DEBUG=fatal-criticals` and `--test-threads=1`. Run each
test by its exact name in a separate process to preserve GTK thread affinity.
They fail rather than silently
skipping when a display is unavailable. It checks native layout properties and
geometry, nested typed templates, retained Entry/Command state, real scrolling,
bounded mounts, content switches, dynamic Gio/NamedAction rebuilding, native
clipboard/About/Quit roles, custom primitives, weak cleanup, task pruning,
runtime host attachment, and the standalone native loop. The dedicated
`tests::native_executor_completion` test checks timer-free completion cleanup,
external wakes without another spawn, destructor reentrancy, and host teardown
with an already queued cleanup callback.

The coordinated-close regressions are `tests::native_coordinated_multiwindow_close`
(denial, retry, live UI drain and exactly-once callbacks),
`tests::native_coordinated_close_timeout` (actual futures-timer deadlines in both
confirmation and drain), `tests::native_coordinated_close_cancel` (Busy and
explicit cancellation/retry), and `tests::native_coordinated_close_weak_teardown`
(pending-request abort and weak View/executor ownership). Each needs its own
process just like the other GTK tests.

`tests::native_entry_converted_replacement` reproduces the native replacement
signal sequence with a typed `f64` converter and checks immediate acceptance,
canonicalization, invalid-input rollback, insert/delete edits, no echo, and
closed-writer rejection. `tests::native_entry_incremental_edits_and_readonly`
checks String insert/delete/empty edits, one write per replacement, model-update
echo suppression, read-only rollback, and scoped signal disconnection. Run each
in its own process with the same native test environment above.

New ordinary-close regressions (5 tests total including layout/shortcut):
`tests::native_close_requests_weak_and_reentrant`,
`tests::native_close_ready_subscription_retains_model_without_view_cycle`,
`tests::native_close_request_handler_precedes_coordinator_and_menu_falls_back`,
`tests::native_canonical_shortcut_semantics`, and
`tests::native_grid_equal_columns_geometry_and_spans`.
The shortcut test checks actual GTK keyval/modifier parsing for both target
conversions and activates installed NamedActions (not synthesized key input).
The Grid test maps real widgets at two logical widths, checks equal allocations,
gaps, explicit spans and empty declared columns. Run the exact Grid test separately
with `GDK_SCALE=1` and `GDK_SCALE=2` under Xvfb (large enough physical screen),
`GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals`. These are logical
geometry regressions at integer scale factors, not screenshot/pixel identity or
fractional-DPI/Windows/macOS validation.