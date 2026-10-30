# wakui2-runtime

UI-local, closure-based MVVM. The API in `../API.md` is implemented and all shared
types are exported at the crate root. No application Contract/Input/Message or
native toolkit dependency is required.

## Additional public API

- `Limits { ingress: usize, tasks: usize }`, default **256 / 64**.
  `Store::with_limits(state, executor, limits)` configures them.
  `Store::pending_tasks()` includes aborted wrappers until the executor releases
  them. `Store::ptr_eq(&other)` compares owner identity.
- `Subscription::new(impl FnOnce() + 'static)` wraps native disconnection or
  composite cleanup; `Subscription::empty()` has no cleanup. Tokens are not Clone.
- `BlockingPool::new(workers, capacity)`, `try_spawn(F)` returning
  `Result<BoxFuture<'static, Result<T, TaskError>>, TaskError>`, with
  `F: FnOnce() -> T + Send + 'static`, `T: Send + 'static`, and `shutdown(&mut self)`.
  `TaskError` is `Full | Shutdown | Panic(String) | Canceled` and implements Error.
- `UiHost` has `executor(&self) -> Rc<dyn Executor>` and
  `attach(&self, Session) -> Result<(), String>`. Implement it on an already-running
  UI host; neither method may drive a nested loop or block the UI thread.
- `async run_async(&dyn UiHost, impl FnOnce(Rc<dyn Executor>) -> Session)` returns
  `Result<Session, String>`. It constructs/attaches on its first poll, without
  running a native loop. Failed attachment forcibly closes the constructed session.
- `Session` is Clone/Default and has `new`, `shutdown() -> ShutdownCoordinator`,
  `window_count() -> usize`, and
  `register_window(id: impl Into<String>, WindowHooks) -> Result<Subscription, String>`.
  Retain registration tokens on window/view scopes. Session registries hold weak
  hook slots; dropping a token unregisters and aborts its pending shutdown waits.
- `WindowHooks` has public Rc callbacks: `confirm` produces
  `LocalBoxFuture<'static, bool>`, `cancel` returns unit, `drain` produces
  `LocalBoxFuture<'static, ()>`, and `close` returns unit. Capture native views weakly.
- `ShutdownCoordinator` is Clone/Default, has `new`, `register_window`,
  `window_count`, `phase() -> ShutdownPhase`, `cancel_shutdown() -> bool`,
  `close_now()`, and `async request_shutdown(Duration) -> ShutdownOutcome`.
  Phases: `Running | Confirming | Draining | Closed`.
  Outcomes: `Closed | Cancelled | TimedOut | Busy`.

## Scheduling and lifecycle

### Cooperative dispatch (opt-in)

`Store::with_dispatch_budget(state, executor, limits, NonZeroUsize)` limits events
per drain. `new` and `with_limits` keep their synchronous behavior. When a budget
is exhausted, a weak continuation yields once before draining again. Queued writes
retain FIFO order and normal capacity checks; close discards them. Accepted writes
may not yet be applied when the call returns. Executors can poll immediately, but
fairness requires the host to return to its OS loop between continuation polls.
The AppKit host bounds polling per native turn. This is not a wall-clock budget:
one clone, projection, callback or future poll cannot be interrupted.

Use direct `Binding::subscribe` for synchronous native property assignment, and
`watch` for derived-state/effect requests after publication. Retain the token,
exclude derived outputs from the source, and use the callback's `target` rather
than capturing a strong Store. For example, on a Store with a `query: String`:

```rust,ignore
let token = store.watch(store.bind(|state| state.query.clone(), None), |target, query| {
  let accepted = target.effect(Policy::Latest, "search", search(query), |state, rows| {
    state.rows = rows;
  });
  if !accepted {
    target.update(|state| state.status = "Search was not accepted".into());
  }
});
view.keep(token);
```

This is a latest-value mailbox, not an event log or debounce. Initial reaction may
run before `watch` returns. Dropping `token` prevents queued reactions, not effects
already started. Check effect acceptance; a positive ingress capacity is necessary
for eventual retry. The settings and library wiring contain executable examples
of rejection recovery and generation checks.

### Third-prototype reads and typed arguments

`Store::watch(source, react) -> Subscription` connects state changes to a
reaction `Fn(&Store<S>, T)`, without capturing a strong Store owner. Unlike a
direct `Binding::subscribe`, reactions run as ordinary FIFO events after the
current publication. Each scope holds at most one latest value and one queued
reaction; a full queue is retried after dequeues. This does not bypass ingress
limits or allocate additional executor tasks. Positive ingress is required.
Intermediate values can coalesce: this is for derived state / latest requests,
not an event log or a transactional stream. Initial delivery can be synchronous;
this is not a timer-based debounce.

A reaction starts after its ingress permit is released, so it can submit one
update or effect even with ingress=1. Further operations still check acceptance.
An effect may still fail its task limit; the reaction can instead submit one
error update. Settings uses this for projection and save recovery; the library
screen uses it for query, Latest scan and projection. Exclude derived output
fields from sources to avoid feedback loops. Dropping the scope invalidates
queued reactions, but does not cancel effects that already started. Store
close/drop discards queued reactions; tokens themselves are not Store owners.

Binding reads and initial subscriptions hold an `Rc<S>` snapshot while projecting
outside internal borrows; they do not clone S. Mutation clones S once, and the
public `snapshot()` still returns an owned clone. Publication still scans all
subscribed projections and compares their values; there is no field-level dirty
tracking. Tests count Clone calls and measure scalar reads with 0/16 MiB of
unprojected state. These are not frame-time or machine-independent guarantees.
`cargo run --locked -p wakui2-runtime --example scaling` also observes 0/1/16 MiB
states with 1/100/1000 subscribers. Every case asserts zero S clones during
50,000 scalar reads, one S clone per update, and exact notification counts.
Twenty update samples produce informational p50/p95 values, without a timing
threshold. Remaining full-state clone and projection-scan costs are intentional
measurement targets, not claims of O(1) updates.

`Store::command` accepts `Binding<A>` and an action receiving `A: Clone + 'static`.
The resulting `Command` remains non-generic and argument-erased for native UI.
Invocation captures A immediately and rechecks enabled when dequeued. Dynamic
menu dispatch remains String-based; generated typed commands can be called from it.

Shared generated `view!` retains a Store until View close/drop without closing
other owners; `owned_view!` explicitly closes that Store. Bindings/tasks retain
weak owners; application-captured strong ownership remains the application's
responsibility. Neither mode changes item eviction into model closure.

All ordinary updates, binding writes, command actions and effect starts share a
bounded FIFO. A true return means accepted, not necessarily applied: command
enablement is checked again on dequeue, and lifecycle/generation checks reject
stale effects. Command string arguments are captured at invocation. Reentrant
updates run after all listeners for the current snapshot.

Each accepted effect reserves one completion slot in addition to ordinary
ingress. Consequently queued events are bounded by `ingress + tasks`; accepted
Parallel results do not disappear when normal ingress fills. Rejection returns
false and does not poll the rejected future. Latest invalidates and aborts older
requests on acceptance, including results already in the FIFO. Policy domains
are `(policy, slot)`: Serial sequencing and Latest replacement do not cancel
Parallel work using the same string slot. Serial waits for its predecessor's
**application and publication**, not merely its future's completion. It does not
poll waiting user futures. Different Serial slots are independent.

Task wrappers and completion leases retain weak Store owners. Executor discard,
panic unwind, completion discard and cancellation all release permits. Aborted
wrappers still occupy a task slot until polled/dropped, preventing an unbounded
backlog of aborting Latest tasks. Executors must service or discard scheduled
futures. All runtime RefCell borrows end before user callbacks, destructors,
conversion/Clone/PartialEq implementations, or future polls.

Bindings retain the snapshot hub rather than the Store. Close/drop freezes the
last published snapshot, releases listeners, aborts tasks and rejects writes.
An update that closes its own Store is not published. Subscription initial
delivery is synchronous, even after close. Source equal values are suppressed;
`map`/`convert` need no target PartialEq and may repeat equal projections.
`is_writable` means a writer exists, not that the owner is open or has capacity.

Validation is initial + Latest per subscription, using a private slot namespace.
Token drop invalidates queued results **and queued derived status writes**, aborts
work, and resets pending. Its status mailbox coalesces/retries writes through the
ordinary bounded FIFO; unmount resets also retry when normal ingress is full.
Mailboxes/cleanup records are subscription lifecycle bookkeeping, not additional
queued ingress or executor tasks. Overload is reported in the error binding as
`validation task limit reached`, with pending false. Use writable error/pending
fields of the source Store, accepting those values, and positive ingress for
guaranteed status retry. Outputs on unrelated Stores are allowed but cannot rely
on this Store's dequeue notifications to retry a full unrelated queue.

Shutdown confirms windows in ID order, then cancels work and concurrently drains
all windows. The real timer deadline covers both confirmation and drain. Denial,
explicit cancellation or dropping the request restores Running; work already
cancelled during drain is not resurrected. Timeout cancels remaining work and
forcibly closes windows. Cancellation is at most once per window per request.
Repeated close is idempotent. Registrations during shutdown and duplicate IDs
return errors. Unmounted windows stop participating in outstanding waits.

## Deliberate boundaries

- Only the runtime was implemented here. Native hosts must implement UiHost and
  connect actual close events to the coordinator; no GTK/Tao integration is
  claimed by these host-neutral tests.
- Cancellation is cooperative. Blocking callbacks/future polls cannot be
  interrupted by abort or timer. Validators should construct futures promptly;
  send blocking work to BlockingPool. Its Drop detaches; explicit shutdown joins
  and must run **after** the UI loop, never on a worker or the live UI thread.
- Keep runtime-owned closures weak. Application closures/futures can explicitly
  capture strong Store/native owners; the runtime cannot rewrite those captures.
  Likewise `S: Clone` does not deep-freeze application interior-mutable objects.
- No OS-specific input/accessibility conformance, clipboard, or native layout
  implementation is implied. Dynamic menu arguments remain strings.