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
Variable-height rows are intentionally unsupported. Use a positive count and a
finite row height of at least one logical pixel. IDs and keys form hierarchical
paths; applications must avoid ambiguous overlapping scope paths.

## Menus

Snapshots rebuild real Gio actions, GMenu sections (separators), and GTK
ShortcutController/NamedAction shortcuts. Shortcuts use GTK trigger syntax, e.g.
`<Control>s`; invalid shortcuts and duplicate IDs reject the snapshot. Normal
items dispatch their String argument; built-in roles operate on native GTK
focus/clipboard, open a native AboutDialog, or close the View. Clipboard roles
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

Coordination only wires native `close-request`; it never registers or replaces
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

`View::close()` remains forceful and idempotent: use it in runtime close hooks,
not to request confirmation. Built-in `MenuRole::Quit` still uses this forceful
API. `GtkWindow::destroy`, Application quit/process termination and forced owner
teardown are not intercepted. Applications requiring confirmation on menu quit
should dispatch a normal menu command to `native_window().close()` instead.

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