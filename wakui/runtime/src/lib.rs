//! Single-threaded, typed MVVM runtime for the declarative wakui prototype.
//!
//! Subscriptions deliver their initial value **synchronously**, including for
//! constants and closed models. Native adapters must install their signal guard
//! before subscribing, retain the returned token, and capture native widgets
//! weakly. Dropping the token unsubscribes; bindings never keep a model alive.
//!
//! Model inputs, commands and asynchronous completions share one FIFO. User
//! futures and native callbacks run only outside model borrows. Executors may
//! poll immediately in `spawn`; they must drive local futures on the UI thread.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, VecDeque},
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::{Rc, Weak},
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
};

use futures::{
    FutureExt,
    channel::oneshot,
    future::{AbortHandle, Abortable, BoxFuture, LocalBoxFuture},
};
use wakui_contract::{CommandKey, Contract, PropertyKey};

/// A UI-thread executor. Scheduling must not move the future to another thread.
pub trait Executor {
    fn spawn(&self, future: LocalBoxFuture<'static, ()>);
}

/// Domain behavior belongs here, never in the binding or native adapter.
pub trait ViewModel: 'static {
    type Contract: Contract;
    type Message: 'static;

    fn state(&self) -> <Self::Contract as Contract>::State;
    fn update(
        &mut self,
        input: <Self::Contract as Contract>::Input,
        cx: &mut UpdateCtx<Self::Message>,
    );
    fn complete(&mut self, message: Self::Message, cx: &mut UpdateCtx<Self::Message>);
}

pub use wakui_contract::ListItem;

/// Own this token for as long as notifications are wanted. Not clonable.
#[must_use = "dropping a subscription immediately unsubscribes"]
pub struct Subscription {
    cancel: Option<Box<dyn FnOnce()>>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

type Callback<T> = Rc<dyn Fn(T)>;
type Subscribe<T> = dyn Fn(Callback<T>) -> Subscription;

/// A live value. Clones share their source, not a copied snapshot.
pub struct Binding<T: Clone + 'static> {
    read: Rc<dyn Fn() -> T>,
    write: Option<Rc<dyn Fn(T) -> bool>>,
    listen: Rc<Subscribe<T>>,
}

impl<T: Clone + 'static> Clone for Binding<T> {
    fn clone(&self) -> Self {
        Self {
            read: self.read.clone(),
            write: self.write.clone(),
            listen: self.listen.clone(),
        }
    }
}

impl<T: Clone + 'static> Binding<T> {
    pub fn get(&self) -> T {
        (self.read)()
    }

    pub fn is_writable(&self) -> bool {
        self.write.is_some()
    }

    /// Returns false for read-only bindings or a closed/dropped model.
    /// True means accepted, not necessarily that the value changed.
    pub fn set(&self, value: T) -> bool {
        self.write.as_ref().is_some_and(|write| write(value))
    }

    /// Calls `callback` synchronously with the initial value before returning.
    /// Subsequent property notifications suppress equal values.
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
                Subscription { cancel: None }
            }),
        }
    }

    /// A read-only projection; one notification per source notification.
    /// No `PartialEq` bound is required, so equal projected values may repeat.
    pub fn map<U: Clone + 'static>(&self, f: impl Fn(T) -> U + 'static) -> Binding<U> {
        let f = Rc::new(f);
        let read = self.read.clone();
        let project = f.clone();
        let source = self.clone();
        Binding {
            read: Rc::new(move || project(read())),
            write: None,
            listen: Rc::new(move |callback| {
                let f = f.clone();
                source.subscribe(Rc::new(move |value| callback(f(value))))
            }),
        }
    }
}

#[derive(Clone)]
pub struct Command {
    pub enabled: Binding<bool>,
    invoke: Rc<dyn Fn() -> bool>,
}

impl Command {
    /// Checks the latest state now and again when a reentrant request dequeues.
    /// True means accepted into the queue; a later state change may reject it.
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
        let callback = self.callback.borrow_mut().take();
        drop(callback);
    }
}

// Only weak listeners are retained here. In particular a callback capturing a
// ModelHandle cannot create model -> callback -> model ownership cycles.
struct StateStore<S: Clone + 'static> {
    value: RefCell<S>,
    listeners: RefCell<BTreeMap<u64, Weak<Listener<S>>>>,
    next_id: Cell<u64>,
    closed: Cell<bool>,
}

impl<S: Clone + 'static> StateStore<S> {
    fn new(state: S) -> Self {
        Self {
            value: RefCell::new(state),
            listeners: RefCell::new(BTreeMap::new()),
            next_id: Cell::new(0),
            closed: Cell::new(false),
        }
    }

    fn get(&self) -> S {
        self.value.borrow().clone()
    }

    fn subscribe(self: &Rc<Self>, callback: Rc<dyn Fn(&S)>) -> Subscription {
        if self.closed.get() {
            callback(&self.get());
            return Subscription { cancel: None };
        }
        let id = self.next_id.get();
        self.next_id
            .set(id.checked_add(1).expect("subscription ID exhausted"));
        let listener = Rc::new(Listener {
            callback: RefCell::new(Some(callback)),
        });
        self.listeners
            .borrow_mut()
            .insert(id, Rc::downgrade(&listener));
        let store = Rc::downgrade(self);
        // Construct the guard before the initial call, including its panic path.
        let token_listener = listener.clone();
        let token = Subscription {
            cancel: Some(Box::new(move || {
                if let Some(store) = store.upgrade() {
                    store.listeners.borrow_mut().remove(&id);
                }
                token_listener.cancel();
            })),
        };
        listener.call(&self.get());
        token
    }

    fn publish(&self, state: S) {
        if self.closed.get() {
            return;
        }
        let old = self.value.replace(state.clone());
        drop(old);
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

struct TaskRequest<Message> {
    slot: &'static str,
    future: LocalBoxFuture<'static, Message>,
}

/// Collects owned effects; constructing an effect never polls its future.
pub struct UpdateCtx<Message: 'static> {
    tasks: Vec<TaskRequest<Message>>,
}

impl<Message: 'static> UpdateCtx<Message> {
    /// Replaces the task in this model's slot. Superseded futures are aborted;
    /// already queued results are also rejected by lifecycle and sequence ID.
    /// Cancellation is cooperative: it cannot interrupt a blocking future poll.
    pub fn latest(&mut self, slot: &'static str, future: impl Future<Output = Message> + 'static) {
        self.tasks.push(TaskRequest {
            slot,
            future: future.boxed_local(),
        });
    }
}

struct ActiveTask {
    sequence: u64,
    abort: AbortHandle,
}

type StateOf<M> = <<M as ViewModel>::Contract as Contract>::State;
type InputOf<M> = <<M as ViewModel>::Contract as Contract>::Input;

enum Event<M: ViewModel> {
    Input(InputOf<M>),
    Command(&'static CommandKey<M::Contract>),
    Completion {
        generation: u64,
        sequence: u64,
        slot: &'static str,
        message: M::Message,
    },
}

struct ModelCore<M: ViewModel> {
    model: RefCell<M>,
    state: Rc<StateStore<StateOf<M>>>,
    executor: Rc<dyn Executor>,
    queue: RefCell<VecDeque<Event<M>>>,
    draining: Cell<bool>,
    closed: Cell<bool>,
    generation: Cell<u64>,
    sequence: Cell<u64>,
    tasks: RefCell<HashMap<&'static str, ActiveTask>>,
}

struct DrainGuard<'a>(&'a Cell<bool>);

impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl<M: ViewModel> ModelCore<M> {
    fn enqueue(self: &Rc<Self>, event: Event<M>) {
        if self.closed.get() {
            return;
        }
        self.queue.borrow_mut().push_back(event);
        if self.draining.replace(true) {
            return;
        }
        let _guard = DrainGuard(&self.draining);
        loop {
            let event = self.queue.borrow_mut().pop_front();
            let Some(event) = event else { break };
            if self.closed.get() {
                break;
            }
            let mut cx = UpdateCtx { tasks: Vec::new() };
            let input = match event {
                Event::Input(input) => Some(input),
                Event::Command(key) => {
                    if !(key.enabled)(&self.state.get()) {
                        continue;
                    }
                    Some((key.input)())
                }
                Event::Completion {
                    generation,
                    sequence,
                    slot,
                    message,
                } => {
                    let current = generation == self.generation.get()
                        && self
                            .tasks
                            .borrow()
                            .get(slot)
                            .is_some_and(|task| task.sequence == sequence);
                    if !current {
                        continue;
                    }
                    self.tasks.borrow_mut().remove(slot);
                    self.model.borrow_mut().complete(message, &mut cx);
                    None
                }
            };
            if let Some(input) = input {
                self.model.borrow_mut().update(input, &mut cx);
            }
            if self.closed.get() {
                continue;
            }
            let state = self.model.borrow().state();
            // Install all sequence IDs before notifying or calling the executor.
            let mut pending = Vec::new();
            for task in cx.tasks {
                let sequence = self
                    .sequence
                    .get()
                    .checked_add(1)
                    .expect("task sequence exhausted");
                self.sequence.set(sequence);
                let (abort, registration) = AbortHandle::new_pair();
                let previous = self
                    .tasks
                    .borrow_mut()
                    .insert(task.slot, ActiveTask { sequence, abort });
                if let Some(previous) = previous {
                    previous.abort.abort();
                }
                pending.push((task, sequence, registration));
            }
            self.state.publish(state);
            for (task, sequence, registration) in pending {
                if self.closed.get()
                    || !self
                        .tasks
                        .borrow()
                        .get(task.slot)
                        .is_some_and(|active| active.sequence == sequence)
                {
                    continue;
                }
                let model = Rc::downgrade(self);
                let generation = self.generation.get();
                self.executor.spawn(
                    async move {
                        if let Ok(message) = Abortable::new(task.future, registration).await
                            && let Some(model) = model.upgrade()
                        {
                            model.enqueue(Event::Completion {
                                generation,
                                sequence,
                                slot: task.slot,
                                message,
                            });
                        }
                    }
                    .boxed_local(),
                );
            }
        }
    }

    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.generation.set(
            self.generation
                .get()
                .checked_add(1)
                .expect("model generation exhausted"),
        );
        let tasks = std::mem::take(&mut *self.tasks.borrow_mut());
        let queue = std::mem::take(&mut *self.queue.borrow_mut());
        for task in tasks.into_values() {
            task.abort.abort();
        }
        self.state.close();
        drop(queue);
    }
}

impl<M: ViewModel> Drop for ModelCore<M> {
    fn drop(&mut self) {
        self.close();
    }
}

/// A model owner. Bindings, commands and executor tasks retain only weak owners.
pub struct ModelHandle<M: ViewModel> {
    core: Rc<ModelCore<M>>,
}

impl<M: ViewModel> Clone for ModelHandle<M> {
    fn clone(&self) -> Self {
        Self {
            core: self.core.clone(),
        }
    }
}

impl<M: ViewModel> ModelHandle<M> {
    pub fn new(model: M, executor: Rc<dyn Executor>) -> Self {
        let state = Rc::new(StateStore::new(model.state()));
        Self {
            core: Rc::new(ModelCore {
                model: RefCell::new(model),
                state,
                executor,
                queue: RefCell::new(VecDeque::new()),
                draining: Cell::new(false),
                closed: Cell::new(false),
                generation: Cell::new(0),
                sequence: Cell::new(0),
                tasks: RefCell::new(HashMap::new()),
            }),
        }
    }

    /// The latest published snapshot, also safe during subscriber callbacks.
    /// After close it remains the last published value.
    pub fn snapshot(&self) -> StateOf<M> {
        self.core.state.get()
    }

    pub fn send(&self, input: InputOf<M>) {
        self.core.enqueue(Event::Input(input));
    }

    pub fn close(&self) {
        self.core.close();
    }

    pub fn is_closed(&self) -> bool {
        self.core.closed.get()
    }

    pub fn bind<T: Clone + PartialEq + 'static>(
        &self,
        key: &'static PropertyKey<M::Contract, T>,
    ) -> Binding<T> {
        let mut binding = self.project(key.read);
        if let Some(write) = key.write {
            let core = Rc::downgrade(&self.core);
            binding.write = Some(Rc::new(move |value| {
                let Some(core) = core.upgrade().filter(|core| !core.closed.get()) else {
                    return false;
                };
                core.enqueue(Event::Input(write(value)));
                true
            }));
        }
        binding
    }

    pub fn command(&self, key: &'static CommandKey<M::Contract>) -> Command {
        let core = Rc::downgrade(&self.core);
        Command {
            enabled: self.project(key.enabled),
            invoke: Rc::new(move || {
                let Some(core) = core.upgrade().filter(|core| !core.closed.get()) else {
                    return false;
                };
                if !(key.enabled)(&core.state.get()) {
                    return false;
                }
                core.enqueue(Event::Command(key));
                true
            }),
        }
    }

    fn project<T: Clone + PartialEq + 'static>(&self, read: fn(&StateOf<M>) -> T) -> Binding<T> {
        let source = self.core.state.clone();
        let value = source.clone();
        Binding {
            read: Rc::new(move || read(&value.get())),
            write: None,
            listen: Rc::new(move |callback| {
                let previous = RefCell::new(None);
                source.subscribe(Rc::new(move |state| {
                    let value = read(state);
                    if previous.borrow().as_ref() == Some(&value) {
                        return;
                    }
                    previous.replace(Some(value.clone()));
                    callback(value);
                }))
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaskError {
    #[error("blocking task queue is full")]
    Full,
    #[error("blocking pool is shut down")]
    Shutdown,
    #[error("blocking task panicked: {0}")]
    Panic(String),
    #[error("blocking task result channel was canceled")]
    Canceled,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A fixed-size pool with a bounded waiting queue and nonblocking submission.
///
/// Keep the pool alive while the UI driver runs, then call `shutdown` after that
/// driver exits to drain and join workers. `Drop` closes the sender but NEVER
/// joins: detached workers drain accepted jobs and exit when the channel closes.
/// Jobs must eventually return for either shutdown path to finish. A dropped
/// result future skips work not yet started; running work cannot be interrupted.
pub struct BlockingPool {
    sender: Option<mpsc::SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl BlockingPool {
    /// `capacity` bounds queued jobs, excluding those already running.
    /// A zero capacity is a rendezvous channel; `workers` must be positive.
    pub fn new(workers: usize, capacity: usize) -> Self {
        assert!(workers > 0, "a blocking pool needs at least one worker");
        let (sender, receiver) = mpsc::sync_channel::<Job>(capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        let workers = (0..workers)
            .map(|_| {
                let receiver = receiver.clone();
                thread::spawn(move || {
                    loop {
                        let job = receiver
                            .lock()
                            .expect("blocking queue mutex poisoned")
                            .recv();
                        let Ok(job) = job else { break };
                        job();
                    }
                })
            })
            .collect();
        Self {
            sender: Some(sender),
            workers,
        }
    }

    pub fn try_spawn<F, T>(
        &self,
        f: F,
    ) -> Result<BoxFuture<'static, Result<T, TaskError>>, TaskError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let sender = self.sender.as_ref().ok_or(TaskError::Shutdown)?;
        let (result, receiver) = oneshot::channel();
        let job: Job = Box::new(move || {
            if result.is_canceled() {
                return;
            }
            let value = catch_unwind(AssertUnwindSafe(f)).map_err(|panic| {
                let message = if let Some(message) = panic.downcast_ref::<String>() {
                    message.clone()
                } else if let Some(message) = panic.downcast_ref::<&str>() {
                    (*message).to_owned()
                } else {
                    "non-string panic payload".to_owned()
                };
                TaskError::Panic(message)
            });
            let _ = result.send(value);
        });
        sender.try_send(job).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => TaskError::Full,
            mpsc::TrySendError::Disconnected(_) => TaskError::Shutdown,
        })?;
        Ok(async move { receiver.await.map_err(|_| TaskError::Canceled)? }.boxed())
    }

    /// Blocking drain/join. Call only AFTER the UI driver exits, not on a worker.
    /// Idempotent; future submissions return `TaskError::Shutdown`.
    pub fn shutdown(&mut self) {
        drop(self.sender.take());
        for worker in self.workers.drain(..) {
            // User panics are already converted to TaskError inside each job.
            let _ = worker.join();
        }
    }
}

impl Drop for BlockingPool {
    fn drop(&mut self) {
        drop(self.sender.take());
        // Dropping JoinHandles detaches rather than blocking the UI thread.
    }
}

#[cfg(test)]
mod tests {
    use futures::{
        executor::{LocalPool, LocalSpawner, block_on},
        task::LocalSpawnExt,
    };

    use super::*;

    #[derive(Clone, Default)]
    struct TestState {
        value: i32,
        enabled: bool,
        requests: usize,
    }

    struct TestContract;

    impl Contract for TestContract {
        type State = TestState;
        type Input = Input;

        fn descriptor() -> wakui_contract::Descriptor {
            wakui_contract::Descriptor {
                name: "test",
                properties: Vec::new(),
                commands: Vec::new(),
            }
        }
    }

    enum Input {
        Value(i32),
        Enable(bool),
        Invoke,
        Start(&'static str, LocalBoxFuture<'static, i32>),
        Batch(Vec<TaskRequest<i32>>),
    }

    // Deliberately not Clone: cloning ModelHandle must not require it.
    struct TestModel {
        state: TestState,
        completions: Rc<Cell<usize>>,
        drops: Rc<Cell<usize>>,
    }

    impl Default for TestModel {
        fn default() -> Self {
            Self {
                state: TestState {
                    enabled: true,
                    ..TestState::default()
                },
                completions: Rc::new(Cell::new(0)),
                drops: Rc::new(Cell::new(0)),
            }
        }
    }

    impl ViewModel for TestModel {
        type Contract = TestContract;
        type Message = i32;

        fn state(&self) -> TestState {
            self.state.clone()
        }

        fn update(&mut self, input: Input, cx: &mut UpdateCtx<i32>) {
            match input {
                Input::Value(value) => self.state.value = value,
                Input::Enable(enabled) => self.state.enabled = enabled,
                Input::Invoke => self.state.value += 1,
                Input::Start(slot, future) => {
                    self.state.requests += 1;
                    cx.latest(slot, future);
                }
                Input::Batch(tasks) => {
                    for task in tasks {
                        cx.latest(task.slot, task.future);
                    }
                }
            }
        }

        fn complete(&mut self, message: i32, _cx: &mut UpdateCtx<i32>) {
            self.completions.set(self.completions.get() + 1);
            self.state.value = message;
        }
    }

    impl Drop for TestModel {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    static VALUE: PropertyKey<TestContract, i32> = PropertyKey {
        name: "value",
        read: |state| state.value,
        write: Some(Input::Value),
    };
    static READ_ONLY: PropertyKey<TestContract, i32> = PropertyKey {
        name: "read_only",
        read: |state| state.value,
        write: None,
    };
    static REQUESTS: PropertyKey<TestContract, usize> = PropertyKey {
        name: "requests",
        read: |state| state.requests,
        write: None,
    };
    static INCREMENT: CommandKey<TestContract> = CommandKey {
        name: "increment",
        input: || Input::Invoke,
        enabled: |state| state.enabled,
    };

    struct LocalExecutor(LocalSpawner);

    impl Executor for LocalExecutor {
        fn spawn(&self, future: LocalBoxFuture<'static, ()>) {
            self.0.spawn_local(future).expect("local executor is alive");
        }
    }

    #[derive(Default)]
    struct InlineExecutor {
        after: RefCell<Option<Box<dyn FnOnce()>>>,
    }

    impl Executor for InlineExecutor {
        fn spawn(&self, future: LocalBoxFuture<'static, ()>) {
            assert!(future.now_or_never().is_some(), "test future must be ready");
            let after = self.after.borrow_mut().take();
            if let Some(after) = after {
                after();
            }
        }
    }

    fn local_model() -> (LocalPool, ModelHandle<TestModel>) {
        let pool = LocalPool::new();
        let model = ModelHandle::new(TestModel::default(), Rc::new(LocalExecutor(pool.spawner())));
        (pool, model)
    }

    fn record<T: Clone + 'static>(binding: &Binding<T>) -> (Rc<RefCell<Vec<T>>>, Subscription) {
        let values = Rc::new(RefCell::new(Vec::new()));
        let out = values.clone();
        let subscription = binding.subscribe(Rc::new(move |value| out.borrow_mut().push(value)));
        (values, subscription)
    }

    #[test]
    fn two_way_initial_delivery_and_no_unchanged_echo() {
        let (_pool, model) = local_model();
        let binding = model.bind(&VALUE);
        let (values, _subscription) = record(&binding);
        assert_eq!(*values.borrow(), [0]);
        assert!(binding.clone().set(5));
        assert_eq!(model.snapshot().value, 5);
        model.clone().send(Input::Value(5));
        assert!(binding.set(5));
        assert_eq!(*values.borrow(), [0, 5]);
        model.send(Input::Value(8));
        assert_eq!(binding.get(), 8);
        assert_eq!(*values.borrow(), [0, 5, 8]);
    }

    #[test]
    fn writing_notification_back_does_not_echo() {
        let (_pool, model) = local_model();
        let binding = model.bind(&VALUE);
        let target = binding.clone();
        let values = Rc::new(RefCell::new(Vec::new()));
        let out = values.clone();
        let _subscription = binding.subscribe(Rc::new(move |value| {
            out.borrow_mut().push(value);
            assert!(target.set(value));
        }));
        model.send(Input::Value(4));
        assert_eq!(*values.borrow(), [0, 4]);
    }

    #[test]
    fn readonly_constant_and_mapped_bindings() {
        let (_pool, model) = local_model();
        assert!(!model.bind(&READ_ONLY).set(1));
        let constant = Binding::constant("constant".to_owned());
        assert!(!constant.set("other".to_owned()));
        let (values, _constant_subscription) = record(&constant.clone());
        assert_eq!(*values.borrow(), ["constant"]);
        let mapped = model.bind(&VALUE).map(|value| format!("value={value}"));
        let (values, _mapped_subscription) = record(&mapped);
        assert!(!mapped.set("ignored".to_owned()));
        model.send(Input::Value(4));
        assert_eq!(mapped.clone().get(), "value=4");
        assert_eq!(*values.borrow(), ["value=0", "value=4"]);
    }

    #[test]
    fn map_does_not_require_partial_eq() {
        #[derive(Clone)]
        struct NotComparable(i32);
        let binding = Binding::constant(3).map(NotComparable);
        let value = Rc::new(Cell::new(0));
        let out = value.clone();
        let _subscription = binding.subscribe(Rc::new(move |value| out.set(value.0)));
        assert_eq!(binding.get().0, 3);
        assert_eq!(value.get(), 3);
    }

    #[test]
    fn reentrant_subscriber_inputs_are_fifo_and_borrow_safe() {
        let (_pool, model) = local_model();
        let binding = model.bind(&VALUE);
        let values = Rc::new(RefCell::new(Vec::new()));
        let out = values.clone();
        let owner = model.clone();
        let _first = binding.subscribe(Rc::new(move |value| {
            out.borrow_mut().push(('a', value));
            if value == 1 {
                owner.send(Input::Value(2));
                assert_eq!(owner.snapshot().value, 1);
            }
        }));
        let out = values.clone();
        let _second = binding.subscribe(Rc::new(move |value| out.borrow_mut().push(('b', value))));
        model.send(Input::Value(1));
        assert_eq!(
            *values.borrow(),
            [('a', 0), ('b', 0), ('a', 1), ('b', 1), ('a', 2), ('b', 2)]
        );
    }

    #[test]
    fn synchronous_initial_callback_can_reenter() {
        let (_pool, model) = local_model();
        let owner = model.clone();
        let values = Rc::new(RefCell::new(Vec::new()));
        let out = values.clone();
        let _subscription = model.bind(&VALUE).subscribe(Rc::new(move |value| {
            out.borrow_mut().push(value);
            if value == 0 {
                owner.send(Input::Value(1));
            }
        }));
        assert_eq!(*values.borrow(), [0, 1]);
    }

    #[test]
    fn dropping_subscription_unregisters_and_releases_callback() {
        let (_pool, model) = local_model();
        let retained = Rc::new(());
        let weak = Rc::downgrade(&retained);
        let subscription = model.bind(&VALUE).subscribe(Rc::new(move |_| {
            let _ = &retained;
        }));
        assert!(weak.upgrade().is_some());
        drop(subscription);
        assert!(weak.upgrade().is_none());
        assert!(model.core.state.listeners.borrow().is_empty());
        model.send(Input::Value(1));
    }

    #[test]
    fn unsubscribe_during_notification_skips_later_listener() {
        let (_pool, model) = local_model();
        let later = Rc::new(RefCell::new(None::<Subscription>));
        let token = later.clone();
        let binding = model.bind(&VALUE);
        let _first = binding.subscribe(Rc::new(move |value| {
            if value == 1 {
                let subscription = token.borrow_mut().take();
                drop(subscription);
            }
        }));
        let (values, second) = record(&binding);
        later.replace(Some(second));
        model.send(Input::Value(1));
        assert_eq!(*values.borrow(), [0]);
    }

    #[test]
    fn initial_callback_panic_unregisters_listener() {
        let (_pool, model) = local_model();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _subscription = model.bind(&VALUE).subscribe(Rc::new(|_| panic!("initial")));
        }));
        assert!(result.is_err());
        assert!(model.core.state.listeners.borrow().is_empty());
    }

    #[test]
    fn latest_reversed_completion_aborts_previous() {
        let (mut pool, model) = local_model();
        let completions = model.core.model.borrow().completions.clone();
        let (old_tx, old_rx) = oneshot::channel();
        model.send(Input::Start(
            "search",
            async move { old_rx.await.unwrap() }.boxed_local(),
        ));
        pool.run_until_stalled();
        let (new_tx, new_rx) = oneshot::channel();
        model.send(Input::Start(
            "search",
            async move { new_rx.await.unwrap() }.boxed_local(),
        ));
        pool.run_until_stalled();
        new_tx.send(20).unwrap();
        pool.run_until_stalled();
        assert!(old_tx.send(10).is_err());
        pool.run_until_stalled();
        assert_eq!(model.snapshot().value, 20);
        assert_eq!(completions.get(), 1);
    }

    #[test]
    fn already_queued_stale_completion_is_rejected() {
        let model = ModelHandle::new(TestModel::default(), Rc::new(InlineExecutor::default()));
        let completions = model.core.model.borrow().completions.clone();
        let owner = model.clone();
        let _subscription = model.bind(&REQUESTS).subscribe(Rc::new(move |requests| {
            if requests == 1 {
                owner.send(Input::Start("search", async { 20 }.boxed_local()));
            }
        }));
        // Subscriber queues the new request before the inline executor queues
        // the old result. The old result must fail its dequeue-time check.
        model.send(Input::Start("search", async { 10 }.boxed_local()));
        assert_eq!(model.snapshot().value, 20);
        assert_eq!(completions.get(), 1);
    }

    #[test]
    fn future_is_not_polled_under_model_borrow() {
        let model = ModelHandle::new(TestModel::default(), Rc::new(InlineExecutor::default()));
        let weak = Rc::downgrade(&model.core);
        let polled = Rc::new(Cell::new(false));
        let out = polled.clone();
        model.send(Input::Start(
            "probe",
            async move {
                let core = weak.upgrade().unwrap();
                assert!(core.model.try_borrow_mut().is_ok());
                assert_eq!(core.state.get().requests, 1);
                out.set(true);
                42
            }
            .boxed_local(),
        ));
        assert!(polled.get());
        assert_eq!(model.snapshot().value, 42);
    }

    #[test]
    fn repeated_latest_in_one_update_polls_only_last_future() {
        let model = ModelHandle::new(TestModel::default(), Rc::new(InlineExecutor::default()));
        model.send(Input::Batch(vec![
            TaskRequest {
                slot: "same",
                future: async { panic!("superseded future polled") }.boxed_local(),
            },
            TaskRequest {
                slot: "same",
                future: async { 7 }.boxed_local(),
            },
        ]));
        assert_eq!(model.snapshot().value, 7);
    }

    #[test]
    fn different_latest_slots_do_not_cancel_each_other() {
        let (mut pool, model) = local_model();
        let completions = model.core.model.borrow().completions.clone();
        let (first_tx, first_rx) = oneshot::channel();
        let (second_tx, second_rx) = oneshot::channel();
        model.send(Input::Start(
            "first",
            async move { first_rx.await.unwrap() }.boxed_local(),
        ));
        model.send(Input::Start(
            "second",
            async move { second_rx.await.unwrap() }.boxed_local(),
        ));
        pool.run_until_stalled();
        second_tx.send(2).unwrap();
        pool.run_until_stalled();
        first_tx.send(1).unwrap();
        pool.run_until_stalled();
        assert_eq!(model.snapshot().value, 1);
        assert_eq!(completions.get(), 2);
    }

    #[test]
    fn close_aborts_pending_and_freezes_bindings() {
        let (mut pool, model) = local_model();
        let binding = model.bind(&VALUE);
        let command = model.command(&INCREMENT);
        let (values, _subscription) = record(&binding);
        let completions = model.core.model.borrow().completions.clone();
        let (tx, rx) = oneshot::channel();
        model.send(Input::Start(
            "pending",
            async move { rx.await.unwrap() }.boxed_local(),
        ));
        pool.run_until_stalled();
        model.close();
        model.close();
        assert!(model.clone().is_closed());
        model.send(Input::Value(9));
        assert!(!binding.set(9));
        assert!(!command.invoke());
        pool.run_until_stalled();
        assert!(tx.send(9).is_err());
        assert_eq!(binding.get(), 0);
        assert_eq!(*values.borrow(), [0]);
        assert_eq!(completions.get(), 0);
        assert!(model.core.tasks.borrow().is_empty());
        assert!(model.core.state.listeners.borrow().is_empty());
        let (initial, _subscription) = record(&binding);
        assert_eq!(*initial.borrow(), [0]);
    }

    #[test]
    fn close_before_first_poll_never_polls_user_future() {
        let (mut pool, model) = local_model();
        let polled = Rc::new(Cell::new(false));
        let out = polled.clone();
        model.send(Input::Start(
            "pending",
            async move {
                out.set(true);
                42
            }
            .boxed_local(),
        ));
        model.close();
        pool.run_until_stalled();
        assert!(!polled.get());
        assert_eq!(model.snapshot().value, 0);
    }

    #[test]
    fn close_clears_an_already_queued_completion() {
        let executor = Rc::new(InlineExecutor::default());
        let model = ModelHandle::new(TestModel::default(), executor.clone());
        let completions = model.core.model.borrow().completions.clone();
        let weak = Rc::downgrade(&model.core);
        executor.after.replace(Some(Box::new(move || {
            let core = weak.upgrade().unwrap();
            assert_eq!(core.queue.borrow().len(), 1);
            core.close();
            assert!(core.queue.borrow().is_empty());
        })));
        model.send(Input::Start("ready", async { 17 }.boxed_local()));
        assert!(model.is_closed());
        assert_eq!(model.snapshot().value, 0);
        assert_eq!(completions.get(), 0);
    }

    #[test]
    fn close_during_notification_clears_reentrant_input_and_callbacks() {
        let (_pool, model) = local_model();
        let binding = model.bind(&VALUE);
        let owner = model.clone();
        let _first = binding.subscribe(Rc::new(move |value| {
            if value == 1 {
                owner.send(Input::Value(2));
                owner.close();
            }
        }));
        let (values, _second) = record(&binding);
        model.send(Input::Value(1));
        assert_eq!(model.snapshot().value, 1);
        assert_eq!(*values.borrow(), [0]);
        assert!(model.core.queue.borrow().is_empty());
        assert!(model.core.state.listeners.borrow().is_empty());
    }

    #[test]
    fn close_releases_callback_even_if_subscription_is_retained() {
        let (_pool, model) = local_model();
        let retained = Rc::new(());
        let weak = Rc::downgrade(&retained);
        let _subscription = model.bind(&VALUE).subscribe(Rc::new(move |_| {
            let _ = &retained;
        }));
        model.close();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn dropping_model_aborts_tasks_without_binding_ownership_cycle() {
        let (mut pool, model) = local_model();
        let binding = model.bind(&VALUE);
        let command = model.command(&INCREMENT);
        let drops = model.core.model.borrow().drops.clone();
        let weak = Rc::downgrade(&model.core);
        let (tx, rx) = oneshot::channel();
        model.send(Input::Start(
            "pending",
            async move { rx.await.unwrap() }.boxed_local(),
        ));
        pool.run_until_stalled();
        drop(model);
        assert!(weak.upgrade().is_none());
        assert_eq!(drops.get(), 1);
        assert!(!binding.set(1));
        assert!(!command.invoke());
        assert_eq!(binding.get(), 0);
        pool.run_until_stalled();
        assert!(tx.send(1).is_err());
    }

    #[test]
    fn subscription_owning_handle_does_not_form_model_cycle() {
        let (_pool, model) = local_model();
        let weak = Rc::downgrade(&model.core);
        let owner = model.clone();
        let subscription = model.bind(&VALUE).subscribe(Rc::new(move |_| {
            let _ = owner.snapshot();
        }));
        drop(model);
        assert!(weak.upgrade().is_some());
        drop(subscription);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn can_execute_gates_invocation_and_notifies_changes() {
        let (_pool, model) = local_model();
        let command = model.command(&INCREMENT).clone();
        let (values, _subscription) = record(&command.enabled);
        assert!(command.invoke());
        model.send(Input::Enable(false));
        assert!(!command.enabled.get());
        assert!(!command.invoke());
        assert_eq!(model.snapshot().value, 1);
        model.send(Input::Enable(true));
        assert!(command.invoke());
        assert_eq!(model.snapshot().value, 2);
        assert_eq!(*values.borrow(), [true, false, true]);
    }

    #[test]
    fn reentrant_command_rechecks_enabled_when_dequeued() {
        let (_pool, model) = local_model();
        let owner = model.clone();
        let command = model.command(&INCREMENT);
        let _subscription = model.bind(&VALUE).subscribe(Rc::new(move |value| {
            if value == 1 {
                owner.send(Input::Enable(false));
                assert!(command.invoke());
            }
        }));
        model.send(Input::Value(1));
        assert_eq!(model.snapshot().value, 1);
        assert!(!model.snapshot().enabled);
    }

    #[test]
    fn blocking_pool_rejects_full_queue_without_waiting() {
        let mut pool = BlockingPool::new(1, 1);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first = pool
            .try_spawn(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                1
            })
            .unwrap();
        started_rx.recv().unwrap();
        let second = pool.try_spawn(|| 2).unwrap();
        assert!(matches!(pool.try_spawn(|| 3), Err(TaskError::Full)));
        release_tx.send(()).unwrap();
        assert_eq!(block_on(first), Ok(1));
        assert_eq!(block_on(second), Ok(2));
        pool.shutdown();
    }

    #[test]
    fn blocking_pool_reports_panic_and_reuses_worker() {
        let mut pool = BlockingPool::new(1, 1);
        let failed = pool
            .try_spawn(|| -> i32 { panic!("worker panic") })
            .unwrap();
        assert_eq!(
            block_on(failed),
            Err(TaskError::Panic("worker panic".to_owned()))
        );
        let next = pool.try_spawn(|| 42).unwrap();
        assert_eq!(block_on(next), Ok(42));
        pool.shutdown();
    }

    #[test]
    fn blocking_pool_skips_canceled_queued_receiver() {
        let mut pool = BlockingPool::new(1, 1);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first = pool
            .try_spawn(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .unwrap();
        started_rx.recv().unwrap();
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let out = ran.clone();
        let canceled = pool
            .try_spawn(move || out.store(true, std::sync::atomic::Ordering::SeqCst))
            .unwrap();
        drop(canceled);
        release_tx.send(()).unwrap();
        pool.shutdown();
        assert_eq!(block_on(first), Ok(()));
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn blocking_pool_shutdown_drains_and_rejects_new_work() {
        let mut pool = BlockingPool::new(2, 2);
        let first = pool.try_spawn(|| 10).unwrap();
        let second = pool.try_spawn(|| 20).unwrap();
        pool.shutdown();
        pool.shutdown();
        assert_eq!(block_on(first), Ok(10));
        assert_eq!(block_on(second), Ok(20));
        assert!(matches!(pool.try_spawn(|| 30), Err(TaskError::Shutdown)));
        assert!(pool.workers.is_empty());
    }

    #[test]
    fn blocking_pool_drop_does_not_join_running_work() {
        let pool = BlockingPool::new(1, 1);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let task = pool
            .try_spawn(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                99
            })
            .unwrap();
        started_rx.recv().unwrap();
        // A joining Drop would deadlock: release deliberately follows Drop.
        drop(pool);
        release_tx.send(()).unwrap();
        assert_eq!(block_on(task), Ok(99));
    }
}
