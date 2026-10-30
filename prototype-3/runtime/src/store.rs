use futures::{
    FutureExt,
    channel::oneshot,
    future::{AbortHandle, Abortable, LocalBoxFuture, Shared},
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    future::Future,
    rc::{Rc, Weak},
};

/// Scheduling is UI-local and nonblocking. Immediate polling is permitted.
pub trait Executor {
    fn spawn(&self, future: LocalBoxFuture<'static, ()>);
}

#[must_use = "retain this token for as long as notifications are wanted"]
pub struct Subscription {
    cancel: Option<Box<dyn FnOnce()>>,
}
impl Subscription {
    /// Also useful for backend signal disconnection and composite ownership.
    pub fn new(cancel: impl FnOnce() + 'static) -> Self {
        Self {
            cancel: Some(Box::new(cancel)),
        }
    }
    pub fn empty() -> Self {
        Self { cancel: None }
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

type Guard = Rc<dyn Fn() -> bool>;
type Callback<T> = Rc<dyn Fn(T)>;
type Writer<T> = Rc<dyn Fn(T, Guard) -> bool>;
type Subscriber<T> = Rc<dyn Fn(Callback<T>) -> Subscription>;

/// Live projection; `set` returns acceptance, not whether the value changed.
#[derive(Clone)]
pub struct Binding<T: Clone + 'static> {
    read: Rc<dyn Fn() -> T>,
    write: Option<Writer<T>>,
    listen: Subscriber<T>,
}
impl<T: Clone + 'static> Binding<T> {
    pub fn get(&self) -> T {
        (self.read)()
    }
    pub fn set(&self, value: T) -> bool {
        self.set_guarded(value, Rc::new(|| true))
    }
    fn set_guarded(&self, value: T, guard: Guard) -> bool {
        self.write.as_ref().is_some_and(|write| write(value, guard))
    }
    /// Indicates a writer exists; a closed or saturated Store still rejects sets.
    pub fn is_writable(&self) -> bool {
        self.write.is_some()
    }
    pub fn subscribe(&self, callback: Rc<dyn Fn(T)>) -> Subscription {
        (self.listen)(callback)
    }
    pub fn constant(value: T) -> Self {
        let initial = value.clone();
        Self {
            read: Rc::new(move || value.clone()),
            write: None,
            listen: Rc::new(move |callback| {
                callback(initial.clone());
                Subscription::empty()
            }),
        }
    }
    /// Read-only projection. Projected equal values may repeat.
    pub fn map<U: Clone + 'static>(&self, forward: impl Fn(T) -> U + 'static) -> Binding<U> {
        let forward = Rc::new(forward);
        let read = self.read.clone();
        let f = forward.clone();
        let source = self.clone();
        Binding {
            read: Rc::new(move || f(read())),
            write: None,
            listen: Rc::new(move |callback| {
                let f = forward.clone();
                source.subscribe(Rc::new(move |value| callback(f(value))))
            }),
        }
    }
    /// A rejected backward conversion returns false without changing the model.
    pub fn convert<U: Clone + 'static>(
        &self,
        forward: impl Fn(T) -> U + 'static,
        backward: impl Fn(U) -> Result<T, String> + 'static,
    ) -> Binding<U> {
        let mut binding = self.map(forward);
        if let Some(write) = self.write.clone() {
            binding.write = Some(Rc::new(move |value, guard| {
                backward(value).is_ok_and(|value| write(value, guard))
            }));
        }
        binding
    }
}

#[derive(Clone)]
pub struct Command {
    pub enabled: Binding<bool>,
    invoke: Rc<dyn Fn() -> bool>,
}
impl Command {
    /// Argument is read now; enabled is checked now and on dequeue.
    pub fn invoke(&self) -> bool {
        (self.invoke)()
    }
}

type StateCallback<S> = Rc<dyn Fn(&S)>;
struct Listener<S> {
    callback: RefCell<Option<StateCallback<S>>>,
}
impl<S> Listener<S> {
    fn call(&self, state: &S) {
        let callback = self.callback.borrow().clone();
        if let Some(callback) = callback {
            callback(state);
        }
    }
    fn cancel(&self) {
        let old = self.callback.borrow_mut().take();
        drop(old);
    }
}

// Bindings keep this snapshot hub alive, but it never owns a Store or callback.
struct Snapshots<S: Clone + 'static> {
    value: RefCell<Rc<S>>,
    listeners: RefCell<BTreeMap<u64, Weak<Listener<S>>>>,
    next: Cell<u64>,
    closed: Cell<bool>,
}
impl<S: Clone + 'static> Snapshots<S> {
    // Clone only the pointer under the borrow. Projection/Clone/Drop are user code.
    fn get(&self) -> Rc<S> {
        self.value.borrow().clone()
    }
    fn publish(&self, state: S) {
        if self.closed.get() {
            return;
        }
        let state = Rc::new(state);
        let old = self.value.replace(state.clone());
        drop(old);
        // All subscribed projections are evaluated; equal results are suppressed
        // by bind(). No per-field dirty tracking is assumed.
        let listeners: Vec<_> = self.listeners.borrow().values().cloned().collect();
        for listener in listeners {
            if self.closed.get() {
                break;
            }
            if let Some(listener) = listener.upgrade() {
                listener.call(&state);
            }
        }
    }
    fn subscribe(self: &Rc<Self>, callback: StateCallback<S>) -> Subscription {
        if self.closed.get() {
            callback(&self.get());
            return Subscription::empty();
        }
        let id = next(&self.next);
        let listener = Rc::new(Listener {
            callback: RefCell::new(Some(callback)),
        });
        self.listeners
            .borrow_mut()
            .insert(id, Rc::downgrade(&listener));
        let hub = Rc::downgrade(self);
        let owned = listener.clone();
        let token = Subscription::new(move || {
            if let Some(hub) = hub.upgrade() {
                hub.listeners.borrow_mut().remove(&id);
            }
            owned.cancel();
        });
        listener.call(&self.get());
        token
    }
    fn close(&self) {
        self.closed.set(true);
        let listeners = std::mem::take(&mut *self.listeners.borrow_mut());
        for listener in listeners.into_values() {
            if let Some(listener) = listener.upgrade() {
                listener.cancel();
            }
        }
    }
}

fn next(counter: &Cell<u64>) -> u64 {
    let value = counter
        .get()
        .checked_add(1)
        .expect("runtime identifier exhausted");
    counter.set(value);
    value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Latest,
    Serial,
    Parallel,
}

/// Normal ingress and outstanding effects (including aborted-but-not-yet-polled
/// wrappers) are bounded separately. Each effect reserves a completion slot, so
/// an accepted Parallel result is never silently lost to ingress saturation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub ingress: usize,
    pub tasks: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            ingress: 256,
            tasks: 64,
        }
    }
}

type Action<S> = Box<dyn FnOnce(&Rc<Core<S>>)>;
struct Event<S: Clone + 'static> {
    action: Action<S>,
    ingress: bool,
}
type Done = Shared<LocalBoxFuture<'static, ()>>;
#[derive(Clone, PartialEq, Eq)]
enum Slot {
    User(String),
    Validation(u64),
}
struct Task {
    slot: Slot,
    policy: Policy,
    valid: bool,
    abort: AbortHandle,
    done: Done,
    finish: Option<oneshot::Sender<()>>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.abort.abort();
        if let Some(finish) = self.finish.take() {
            let _ = finish.send(());
        }
    }
}

struct Core<S: Clone + 'static> {
    snapshots: Rc<Snapshots<S>>,
    executor: Rc<dyn Executor>,
    queue: RefCell<VecDeque<Event<S>>>,
    ingress: Cell<usize>,
    draining: Cell<bool>,
    closed: Cell<bool>,
    sequence: Cell<u64>,
    tasks: RefCell<BTreeMap<u64, Task>>,
    flushers: RefCell<BTreeMap<u64, Weak<dyn Fn()>>>,
    finalizers: RefCell<Vec<Rc<dyn Fn() -> bool>>>,
    limits: Limits,
}
struct DrainGuard<'a>(&'a Cell<bool>);
impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

// Travels from ingress to executor to completion. A dropped executor future,
// panic, rejected enqueue or discarded completion always releases its permit.
struct TaskLease<S: Clone + 'static> {
    owner: Weak<Core<S>>,
    id: u64,
}
impl<S: Clone + 'static> Drop for TaskLease<S> {
    fn drop(&mut self) {
        if let Some(core) = self.owner.upgrade() {
            core.finish(self.id);
        }
    }
}

impl<S: Clone + 'static> Core<S> {
    fn enqueue(self: &Rc<Self>, action: Action<S>, ingress: bool) -> bool {
        if self.closed.get() || (ingress && self.ingress.get() >= self.limits.ingress) {
            return false;
        }
        if ingress {
            self.ingress.set(self.ingress.get() + 1);
        }
        self.queue.borrow_mut().push_back(Event { action, ingress });
        self.drain();
        true
    }
    fn drain(self: &Rc<Self>) {
        if self.draining.replace(true) {
            return;
        }
        let _guard = DrainGuard(&self.draining);
        loop {
            let event = self.queue.borrow_mut().pop_front();
            let Some(event) = event else {
                break;
            };
            if event.ingress {
                self.ingress.set(self.ingress.get() - 1);
            }
            if !self.closed.get() {
                (event.action)(self);
            }
            if !self.closed.get() {
                self.flush();
            }
        }
    }
    fn flush(&self) {
        // Status mailboxes belong to subscription scopes, not the ingress queue.
        // They coalesce and retry only when a dequeue has made space available.
        let flushers: Vec<_> = self.flushers.borrow().values().cloned().collect();
        for flusher in flushers {
            if self.closed.get() {
                break;
            }
            if let Some(flusher) = flusher.upgrade() {
                flusher();
            }
        }
        let finalizers = std::mem::take(&mut *self.finalizers.borrow_mut());
        let mut pending = Vec::new();
        for finalizer in finalizers {
            if !self.closed.get() && !finalizer() {
                pending.push(finalizer);
            }
        }
        if !self.closed.get() {
            self.finalizers.borrow_mut().extend(pending);
        }
    }
    fn mutate(&self, update: impl FnOnce(&mut S)) {
        let mut state = (*self.snapshots.get()).clone();
        update(&mut state);
        if !self.closed.get() {
            self.snapshots.publish(state);
        }
    }
    fn valid(&self, id: u64) -> bool {
        !self.closed.get() && self.tasks.borrow().get(&id).is_some_and(|task| task.valid)
    }
    fn finish(&self, id: u64) {
        let task = self.tasks.borrow_mut().remove(&id);
        drop(task);
    }
    fn cancel(&self, id: u64) {
        let abort = {
            let mut tasks = self.tasks.borrow_mut();
            tasks.get_mut(&id).map(|task| {
                task.valid = false;
                task.abort.clone()
            })
        };
        if let Some(abort) = abort {
            abort.abort();
        }
    }
    fn effect<R: 'static>(
        self: &Rc<Self>,
        policy: Policy,
        slot: Slot,
        future: impl Future<Output = R> + 'static,
        apply: impl FnOnce(&mut S, R) + 'static,
    ) -> Option<u64> {
        if self.closed.get()
            || self.ingress.get() >= self.limits.ingress
            || self.tasks.borrow().len() >= self.limits.tasks
        {
            return None;
        }
        let id = next(&self.sequence);
        let (abort, registration) = AbortHandle::new_pair();
        let (finish, finished) = oneshot::channel();
        let done = async move {
            let _ = finished.await;
        }
        .boxed_local()
        .shared();
        let (gate, superseded) = {
            let mut tasks = self.tasks.borrow_mut();
            let gate = if policy == Policy::Serial {
                tasks
                    .values()
                    .rev()
                    .find(|t| t.slot == slot && t.policy == Policy::Serial)
                    .map(|t| t.done.clone())
            } else {
                None
            };
            let mut superseded = Vec::new();
            if policy == Policy::Latest {
                for task in tasks
                    .values_mut()
                    .filter(|t| t.slot == slot && t.policy == Policy::Latest)
                {
                    task.valid = false;
                    superseded.push(task.abort.clone());
                }
            }
            tasks.insert(
                id,
                Task {
                    slot,
                    policy,
                    valid: true,
                    abort,
                    done,
                    finish: Some(finish),
                },
            );
            (gate, superseded)
        };
        for abort in superseded {
            abort.abort();
        }
        // Reservation is installed before calling an executor which can reenter.
        let lease = TaskLease {
            owner: Rc::downgrade(self),
            id,
        };
        let accepted = self.enqueue(
            Box::new(move |core| {
                if !core.valid(id) {
                    return;
                }
                let owner = Rc::downgrade(core);
                core.executor.spawn(
                    async move {
                        let lease = lease;
                        let result = Abortable::new(
                            async move {
                                if let Some(gate) = gate {
                                    gate.await;
                                }
                                let result = future.await;
                                (result, apply)
                            },
                            registration,
                        )
                        .await;
                        if let Some(core) = owner.upgrade() {
                            match result {
                                Ok((result, apply)) => {
                                    // Reserved completion: at most one per outstanding task.
                                    core.enqueue(
                                        Box::new(move |core| {
                                            let _lease = lease;
                                            if core.valid(id) {
                                                core.mutate(move |state| apply(state, result));
                                            }
                                        }),
                                        false,
                                    );
                                }
                                Err(_) => drop(lease),
                            }
                        }
                    }
                    .boxed_local(),
                );
            }),
            true,
        );
        if !accepted {
            return None;
        }
        Some(id)
    }
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        let tasks = std::mem::take(&mut *self.tasks.borrow_mut());
        let queue = std::mem::take(&mut *self.queue.borrow_mut());
        let finalizers = std::mem::take(&mut *self.finalizers.borrow_mut());
        self.flushers.borrow_mut().clear();
        self.ingress.set(0);
        self.snapshots.close();
        drop(tasks);
        drop(queue);
        drop(finalizers);
    }
}
impl<S: Clone + 'static> Drop for Core<S> {
    fn drop(&mut self) {
        self.close();
    }
}

/// Strong model owner. Framework-created bindings, commands and tasks use Weak.
#[derive(Clone)]
pub struct Store<S: Clone + 'static> {
    core: Rc<Core<S>>,
}
impl<S: Clone + 'static> Store<S> {
    pub fn new(state: S, executor: Rc<dyn Executor>) -> Self {
        Self::with_limits(state, executor, Limits::default())
    }
    pub fn with_limits(state: S, executor: Rc<dyn Executor>, limits: Limits) -> Self {
        Self {
            core: Rc::new(Core {
                snapshots: Rc::new(Snapshots {
                    value: RefCell::new(Rc::new(state)),
                    listeners: RefCell::new(BTreeMap::new()),
                    next: Cell::new(0),
                    closed: Cell::new(false),
                }),
                executor,
                queue: RefCell::new(VecDeque::new()),
                ingress: Cell::new(0),
                draining: Cell::new(false),
                closed: Cell::new(false),
                sequence: Cell::new(0),
                tasks: RefCell::new(BTreeMap::new()),
                flushers: RefCell::new(BTreeMap::new()),
                finalizers: RefCell::new(Vec::new()),
                limits,
            }),
        }
    }
    /// An owned Clone of the current state; binding reads only clone their projection.
    pub fn snapshot(&self) -> S {
        (*self.core.snapshots.get()).clone()
    }
    pub fn update(&self, update: impl FnOnce(&mut S) + 'static) -> bool {
        self.core
            .enqueue(Box::new(move |core| core.mutate(update)), true)
    }
    /// React through ordinary FIFO ingress after publication, not inside a
    /// listener. Each token keeps only the latest source value and retries when
    /// a dequeue frees capacity. Unlike subscribe(), intermediate values may be
    /// coalesced. Positive ingress capacity is required for delivery.
    ///
    /// A reaction starts with one ingress slot free; it can submit one update or
    /// effect without competing with other listeners from that publication.
    /// Further submissions still require checking acceptance. Reactions are
    /// synchronous, not debounced, and initial delivery can occur before return.
    /// Dropping the token invalidates even queued reactions, but not effects
    /// already started. Capture the supplied Store only for the current call,
    /// rather than capturing a strong owner in the reaction itself.
    pub fn watch<T: Clone + 'static>(
        &self,
        source: Binding<T>,
        react: impl Fn(&Store<S>, T) + 'static,
    ) -> Subscription {
        let state = Rc::new(Watch {
            owner: Rc::downgrade(&self.core),
            alive: Cell::new(true),
            queued: Cell::new(false),
            latest: RefCell::new(None),
            react: Rc::new(react),
        });
        let id = next(&self.core.sequence);
        let weak = Rc::downgrade(&state);
        let flusher: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                state.flush();
            }
        });
        self.core
            .flushers
            .borrow_mut()
            .insert(id, Rc::downgrade(&flusher));
        let owner = Rc::downgrade(&self.core);
        // Install before subscribe's initial callback, including its panic path.
        let cleanup = Subscription::new(move || {
            if let Some(core) = owner.upgrade() {
                core.flushers.borrow_mut().remove(&id);
            }
        });
        let weak = Rc::downgrade(&state);
        let source = source.subscribe(Rc::new(move |value| {
            if let Some(state) = weak.upgrade().filter(|state| state.alive.get()) {
                let old = state.latest.replace(Some(value));
                drop(old);
                state.flush();
            }
        }));
        Subscription::new(move || {
            state.alive.set(false);
            let old = state.latest.take();
            drop(old);
            drop(source);
            drop(cleanup);
            drop(flusher);
        })
    }
    pub fn close(&self) {
        self.core.close();
    }
    pub fn is_closed(&self) -> bool {
        self.core.closed.get()
    }
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.core, &other.core)
    }
    pub fn pending_tasks(&self) -> usize {
        self.core.tasks.borrow().len()
    }
    pub fn bind<T: Clone + PartialEq + 'static>(
        &self,
        read: impl Fn(&S) -> T + 'static,
        write: Option<fn(&mut S, T)>,
    ) -> Binding<T> {
        let read = Rc::new(read);
        let hub = self.core.snapshots.clone();
        let value = hub.clone();
        let project = read.clone();
        let owner = Rc::downgrade(&self.core);
        Binding {
            read: Rc::new(move || project(&value.get())),
            write: write.map(|write| {
                Rc::new(move |value, guard: Guard| {
                    let Some(core) = owner.upgrade() else {
                        return false;
                    };
                    core.enqueue(
                        Box::new(move |core| {
                            if guard() {
                                core.mutate(move |state| write(state, value));
                            }
                        }),
                        true,
                    )
                }) as Writer<T>
            }),
            listen: Rc::new(move |callback| {
                let read = read.clone();
                let previous = RefCell::new(None);
                hub.subscribe(Rc::new(move |state| {
                    let value = read(state);
                    // Clone/PartialEq/Drop implementations are application code too.
                    let old = previous.take();
                    let changed = old.as_ref() != Some(&value);
                    previous.replace(Some(value.clone()));
                    drop(old);
                    if changed {
                        callback(value);
                    }
                }))
            }),
        }
    }
    /// Capture a typed argument at invocation; the returned UI command erases A.
    pub fn command<A: Clone + 'static>(
        &self,
        enabled: impl Fn(&S) -> bool + 'static,
        action: impl Fn(&Store<S>, A) + 'static,
        argument: Binding<A>,
    ) -> Command {
        let enabled = self.bind(enabled, None);
        let check = enabled.clone();
        let owner = Rc::downgrade(&self.core);
        let action = Rc::new(action);
        Command {
            enabled,
            invoke: Rc::new(move || {
                let Some(core) = owner.upgrade() else {
                    return false;
                };
                if core.closed.get() || !check.get() {
                    return false;
                }
                let argument = argument.get();
                let check = check.clone();
                let action = action.clone();
                core.enqueue(
                    Box::new(move |core| {
                        if check.get() {
                            action(&Store { core: core.clone() }, argument);
                        }
                    }),
                    true,
                )
            }),
        }
    }
    pub fn effect<R: 'static>(
        &self,
        policy: Policy,
        slot: &str,
        future: impl Future<Output = R> + 'static,
        apply: impl FnOnce(&mut S, R) + 'static,
    ) -> bool {
        self.core
            .effect(policy, Slot::User(slot.to_owned()), future, apply)
            .is_some()
    }
}

type Reaction<S, T> = Rc<dyn Fn(&Store<S>, T)>;
struct Watch<S: Clone + 'static, T> {
    owner: Weak<Core<S>>,
    alive: Cell<bool>,
    queued: Cell<bool>,
    latest: RefCell<Option<T>>,
    react: Reaction<S, T>,
}
impl<S: Clone + 'static, T: 'static> Watch<S, T> {
    fn flush(self: &Rc<Self>) {
        if !self.alive.get() || self.latest.borrow().is_none() || self.queued.get() {
            return;
        }
        let Some(core) = self.owner.upgrade().filter(|core| !core.closed.get()) else {
            return;
        };
        self.queued.set(true);
        let weak = Rc::downgrade(self);
        if !core.enqueue(
            Box::new(move |core| {
                if let Some(state) = weak.upgrade().filter(|state| state.alive.get()) {
                    state.queued.set(false);
                    let value = state.latest.take();
                    if let Some(value) = value {
                        (state.react)(&Store { core: core.clone() }, value);
                    }
                }
            }),
            true,
        ) {
            self.queued.set(false);
        }
    }
}

struct ValidationStatus {
    generation: u64,
    error: Option<String>,
    pending: Option<bool>,
}
struct Validation<S: Clone + 'static> {
    owner: Weak<Core<S>>,
    alive: Cell<bool>,
    generation: Cell<u64>,
    task: Cell<Option<u64>>,
    error: Binding<String>,
    pending: Binding<bool>,
    status: RefCell<Option<ValidationStatus>>,
    flushing: Cell<bool>,
}
impl<S: Clone + 'static> Validation<S> {
    fn guard(self: &Rc<Self>, generation: u64) -> Guard {
        let weak = Rc::downgrade(self);
        Rc::new(move || {
            weak.upgrade()
                .is_some_and(|v| v.alive.get() && v.generation.get() == generation)
        })
    }
    fn cancel(&self) {
        next(&self.generation);
        let old = self.status.borrow_mut().take();
        drop(old);
        if let (Some(core), Some(id)) = (self.owner.upgrade(), self.task.take()) {
            core.cancel(id);
        }
    }
    fn status(&self, generation: u64, error: String, pending: bool) {
        let old = self.status.replace(Some(ValidationStatus {
            generation,
            error: Some(error),
            pending: Some(pending),
        }));
        drop(old);
    }
    fn flush(self: &Rc<Self>) {
        if self.flushing.replace(true) {
            return;
        }
        let _guard = DrainGuard(&self.flushing);
        let status = self.status.borrow_mut().take();
        let Some(mut status) = status else {
            return;
        };
        let guard = self.guard(status.generation);
        if !guard() {
            return;
        }
        if let Some(pending) = status.pending
            && self.pending.set_guarded(pending, guard.clone())
        {
            status.pending = None;
        }
        if guard()
            && let Some(error) = status.error.take()
            && !self.error.set_guarded(error.clone(), guard.clone())
        {
            status.error = Some(error);
        }
        if guard()
            && (status.pending.is_some() || status.error.is_some())
            && self.status.borrow().is_none()
        {
            self.status.replace(Some(status));
        }
    }
}
impl<S: Clone + 'static> Drop for Validation<S> {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Latest validation per token, including an immediate initial request. Dropping
/// the token aborts work and invalidates even already-queued binding writes.
/// Saturation is exposed as an error string. Status writes coalesce/retry through
/// the same bounded FIFO. Output bindings must be writable and accept the values;
/// use fields of this Store (and positive ingress capacity) for guaranteed retry.
pub fn validation<S: Clone + 'static>(
    store: &Store<S>,
    source: Binding<String>,
    error: Binding<String>,
    pending: Binding<bool>,
    validator: impl Fn(String) -> LocalBoxFuture<'static, Result<(), String>> + 'static,
) -> Subscription {
    let state = Rc::new(Validation {
        owner: Rc::downgrade(&store.core),
        alive: Cell::new(true),
        generation: Cell::new(0),
        task: Cell::new(None),
        error,
        pending,
        status: RefCell::new(None),
        flushing: Cell::new(false),
    });
    let id = next(&store.core.sequence);
    let flush_state = Rc::downgrade(&state);
    let flusher: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(state) = flush_state.upgrade() {
            state.flush();
        }
    });
    store
        .core
        .flushers
        .borrow_mut()
        .insert(id, Rc::downgrade(&flusher));
    let cleanup_owner = Rc::downgrade(&store.core);
    // Install cleanup before the initial subscriber/validator callback, including
    // its panic path. The guard owns only weak references until token creation.
    let cleanup = Subscription::new(move || {
        if let Some(core) = cleanup_owner.upgrade() {
            core.flushers.borrow_mut().remove(&id);
        }
    });
    let weak = Rc::downgrade(&state);
    let slot = Slot::Validation(id);
    let subscription = source.subscribe(Rc::new(move |value| {
        let Some(state) = weak.upgrade().filter(|s| s.alive.get()) else {
            return;
        };
        state.cancel();
        let generation = state.generation.get();
        let guard = state.guard(generation);
        let Some(core) = state.owner.upgrade().filter(|core| !core.closed.get()) else {
            return;
        };
        state.status(generation, String::new(), true);
        if !core.draining.get() {
            state.flush();
        }
        if !guard() {
            return;
        }
        let future = validator(value);
        if !guard() {
            return;
        }
        let weak = Rc::downgrade(&state);
        let task = core.effect(Policy::Latest, slot.clone(), future, move |_, result| {
            if let Some(state) = weak
                .upgrade()
                .filter(|s| s.alive.get() && s.generation.get() == generation)
            {
                state.status(generation, result.err().unwrap_or_default(), false);
                state.flush();
            }
        });
        if !state.alive.get() || state.generation.get() != generation {
            if let Some(id) = task {
                core.cancel(id);
            }
            return;
        }
        state.task.set(task);
        if task.is_none() {
            state.status(generation, "validation task limit reached".into(), false);
        }
        state.flush();
    }));
    Subscription::new(move || {
        state.alive.set(false);
        state.cancel();
        drop(subscription);
        drop(cleanup);
        drop(flusher);
        if !state.pending.set(false)
            && let Some(core) = state.owner.upgrade().filter(|c| !c.closed.get())
        {
            // A final unmount reset has no listener/task ownership and is
            // retried only as ordinary ingress space becomes available.
            let pending = state.pending.clone();
            core.finalizers
                .borrow_mut()
                .push(Rc::new(move || pending.set(false)));
        }
    })
}
