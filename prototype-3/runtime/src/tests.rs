use crate::*;
use futures::{
    FutureExt,
    channel::oneshot,
    executor::{LocalPool, LocalSpawner, block_on},
    future::{LocalBoxFuture, pending},
    task::LocalSpawnExt,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};

struct LocalExecutor(LocalSpawner);
impl Executor for LocalExecutor {
    fn spawn(&self, future: LocalBoxFuture<'static, ()>) {
        self.0.spawn_local(future).unwrap();
    }
}
#[derive(Default)]
struct Inline {
    after: RefCell<Option<Box<dyn FnOnce()>>>,
}
impl Executor for Inline {
    fn spawn(&self, future: LocalBoxFuture<'static, ()>) {
        assert!(future.now_or_never().is_some());
        let after = self.after.borrow_mut().take();
        if let Some(after) = after {
            after();
        }
    }
}
fn local<S: Clone + 'static>(state: S) -> (LocalPool, Store<S>) {
    let pool = LocalPool::new();
    let store = Store::new(state, Rc::new(LocalExecutor(pool.spawner())));
    (pool, store)
}
fn record<T: Clone + 'static>(binding: &Binding<T>) -> (Rc<RefCell<Vec<T>>>, Subscription) {
    let values = Rc::new(RefCell::new(Vec::new()));
    let out = values.clone();
    let token = binding.subscribe(Rc::new(move |v| out.borrow_mut().push(v)));
    (values, token)
}
#[derive(Clone, Default)]
struct Model {
    value: i32,
    enabled: bool,
    argument: String,
    error: String,
    pending: bool,
}
fn number(s: &Store<Model>) -> Binding<i32> {
    s.bind(|s| s.value, Some(|s, v| s.value = v))
}

#[test]
fn dispatch_budget_yields_preserves_fifo_and_close_cancels_continuation() {
    let mut pool = LocalPool::new();
    let store = Store::with_dispatch_budget(
        0usize,
        Rc::new(LocalExecutor(pool.spawner())),
        Limits {
            ingress: 2,
            tasks: 2,
        },
        std::num::NonZeroUsize::new(2).unwrap(),
    );
    let token = store.watch(store.bind(|value| *value, None), |target, value| {
        if value < 100 {
            assert!(target.update(move |state| *state = value + 1));
        }
    });
    assert!(store.snapshot() < 100);
    pool.run_until_stalled();
    assert_eq!(store.snapshot(), 100);
    assert!(store.update(|state| *state = 0));
    let before_close = store.snapshot();
    assert!(before_close < 100);
    store.close();
    pool.run_until_stalled();
    assert_eq!(store.snapshot(), before_close);
    drop(token);
}

#[test]
fn dispatch_budget_allows_immediate_continuation_polling_without_stalling() {
    struct Immediate;
    impl Executor for Immediate {
        fn spawn(&self, mut future: LocalBoxFuture<'static, ()>) {
            let waker = futures::task::noop_waker();
            let mut context = std::task::Context::from_waker(&waker);
            for _ in 0..2 {
                if future.as_mut().poll(&mut context).is_ready() {
                    return;
                }
            }
            panic!("continuation did not complete after its yield");
        }
    }
    let store = Store::with_dispatch_budget(
        0usize,
        Rc::new(Immediate),
        Limits::default(),
        std::num::NonZeroUsize::new(2).unwrap(),
    );
    let token = store.watch(store.bind(|value| *value, None), |target, value| {
        if value < 20 {
            assert!(target.update(move |state| *state = value + 1));
        }
    });
    assert_eq!(store.snapshot(), 20);
    drop(token);
}
fn text(s: &Store<Model>) -> Binding<String> {
    s.bind(|s| s.argument.clone(), Some(|s, v| s.argument = v))
}
fn error(s: &Store<Model>) -> Binding<String> {
    s.bind(|s| s.error.clone(), Some(|s, v| s.error = v))
}
fn pending_binding(s: &Store<Model>) -> Binding<bool> {
    s.bind(|s| s.pending, Some(|s, v| s.pending = v))
}

struct CountedState {
    value: usize,
    payload: Vec<u8>,
    clones: Rc<Cell<usize>>,
}
impl Clone for CountedState {
    fn clone(&self) -> Self {
        self.clones.set(self.clones.get() + 1);
        Self {
            value: self.value,
            payload: self.payload.clone(),
            clones: self.clones.clone(),
        }
    }
}

#[test]
fn projections_clone_no_state_mutation_clones_once_and_snapshot_is_owned() {
    let clones = Rc::new(Cell::new(0));
    let (_, store) = local(CountedState {
        value: 0,
        payload: vec![7; 1024 * 1024],
        clones: clones.clone(),
    });
    let binding = store.bind(|s| s.value, Some(|s, v| s.value = v));
    let (values, token) = record(&binding);
    assert_eq!(binding.get(), 0);
    assert_eq!(clones.get(), 0);
    let frozen = store.snapshot();
    assert_eq!(clones.get(), 1);
    assert!(binding.set(1));
    assert_eq!(clones.get(), 2);
    assert_eq!(frozen.value, 0);
    assert_eq!(frozen.payload[0], 7);
    assert_eq!(*values.borrow(), [0, 1]);
    store.close();
    drop(store);
    assert!(!binding.set(2));
    assert_eq!(binding.get(), 1);
    let (_, closed_token) = record(&binding);
    assert_eq!(clones.get(), 2);
    drop((token, closed_token));
}

#[test]
fn projection_read_cost_is_independent_of_large_unprojected_state() {
    // Informational timings, deterministic clone-count assertions (no flaky ratio).
    // Publish still evaluates every subscribed projection; no field-dirty index.
    for bytes in [0, 16 * 1024 * 1024] {
        let clones = Rc::new(Cell::new(0));
        let (_, store) = local(CountedState {
            value: 42,
            payload: vec![0; bytes],
            clones: clones.clone(),
        });
        let binding = store.bind(|s| s.value, None);
        let start = std::time::Instant::now();
        for _ in 0..50_000 {
            assert_eq!(std::hint::black_box(&binding).get(), 42);
        }
        println!(
            "projection: state_bytes={bytes}, reads=50000, elapsed={:?}, state_clones={}",
            start.elapsed(),
            clones.get()
        );
        assert_eq!(clones.get(), 0);
    }
}

type Reentry = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;
fn reenter(hook: &Reentry) {
    let action = hook.borrow_mut().take();
    if let Some(action) = action {
        action();
    }
}
struct ReentrantState {
    value: i32,
    clone_hook: Reentry,
    drop_hook: Reentry,
}
impl Clone for ReentrantState {
    fn clone(&self) -> Self {
        reenter(&self.clone_hook);
        Self {
            value: self.value,
            clone_hook: self.clone_hook.clone(),
            drop_hook: self.drop_hook.clone(),
        }
    }
}
impl Drop for ReentrantState {
    fn drop(&mut self) {
        reenter(&self.drop_hook);
    }
}

#[test]
fn state_clone_drop_and_projection_can_reenter_without_snapshot_borrows() {
    let clone_hook = Reentry::default();
    let drop_hook = Reentry::default();
    let (_, store) = local(ReentrantState {
        value: 0,
        clone_hook: clone_hook.clone(),
        drop_hook: drop_hook.clone(),
    });
    let binding = store.bind(|s| s.value, None);
    let owner = store.clone();
    clone_hook.replace(Some(Box::new(move || {
        // Avoid dropping an auxiliary S clone before the publish Drop probe.
        assert_eq!(owner.bind(|s| s.value, None).get(), 0);
        assert!(owner.update(|s| s.value = 2));
    })));
    let owner = store.clone();
    drop_hook.replace(Some(Box::new(move || {
        assert_eq!(owner.snapshot().value, 1);
        assert!(owner.update(|s| s.value = 3));
    })));
    assert!(store.update(|s| s.value = 1));
    assert_eq!(binding.get(), 3);

    let owner = store.clone();
    let project_hook = Reentry::default();
    let hook = project_hook.clone();
    let projected = store.bind(
        move |s| {
            reenter(&hook);
            s.value
        },
        None,
    );
    project_hook.replace(Some(Box::new(move || {
        assert!(owner.update(|s| s.value = 4));
    })));
    // The Rc snapshot pins the old state across a reentrant publish.
    assert_eq!(projected.get(), 3);
    assert_eq!(projected.get(), 4);
    let owner = store.clone();
    project_hook.replace(Some(Box::new(move || {
        assert!(owner.update(|s| s.value = 5));
    })));
    let (_, _token) = record(&projected);
    assert_eq!(projected.get(), 5);
}

struct ReentrantValue {
    value: i32,
    hook: Reentry,
}
impl Clone for ReentrantValue {
    fn clone(&self) -> Self {
        reenter(&self.hook);
        Self {
            value: self.value,
            hook: self.hook.clone(),
        }
    }
}
impl PartialEq for ReentrantValue {
    fn eq(&self, other: &Self) -> bool {
        reenter(&self.hook);
        self.value == other.value
    }
}
impl Drop for ReentrantValue {
    fn drop(&mut self) {
        reenter(&self.hook);
    }
}

#[test]
fn projection_clone_equality_and_drop_remain_reentrant() {
    let (_, store) = local(0);
    let hook = Reentry::default();
    let probe = hook.clone();
    let binding = store.bind(
        move |s| ReentrantValue {
            value: *s,
            hook: probe.clone(),
        },
        None,
    );
    let (_, _token) = record(&binding);
    let owner = store.clone();
    hook.replace(Some(Box::new(move || {
        assert!(owner.update(|s| *s = 2));
    })));
    assert!(store.update(|s| *s = 1));
    assert_eq!(binding.get().value, 2);
    let owner = store.clone();
    hook.replace(Some(Box::new(move || {
        assert!(owner.update(|s| *s = 3));
    })));
    drop(binding.get());
    assert_eq!(store.snapshot(), 3);
}

#[test]
fn typed_command_captures_integer_argument_before_dequeue() {
    let (_, store) = local(Model {
        enabled: true,
        value: 7,
        ..Model::default()
    });
    let command: Command = store.command(
        |s| s.enabled,
        |model, arg: i32| {
            model.update(move |s| s.error = arg.to_string());
        },
        number(&store),
    );
    let owner = store.clone();
    store.update(move |_| {
        assert!(owner.update(|s| s.value = 9));
        assert!(command.invoke());
    });
    assert_eq!(store.snapshot().error, "7");
}

#[test]
fn binding_conversions_readonly_initial_and_equal_suppression() {
    let (_, store) = local(Model::default());
    let binding = number(&store);
    let (values, _token) = record(&binding);
    assert_eq!(*values.borrow(), [0]);
    let converted = binding.convert(
        |v| v.to_string(),
        |v: String| v.parse().map_err(|_| "integer required".into()),
    );
    assert!(converted.is_writable());
    assert!(!converted.set("bad".into()));
    assert_eq!(binding.get(), 0);
    assert!(converted.set("42".into()));
    assert!(binding.set(42));
    assert_eq!(*values.borrow(), [0, 42]);
    let mapped = binding.map(|v| v.to_string());
    assert!(!mapped.is_writable());
    assert!(!mapped.set("1".into()));
    let readonly = mapped.convert(|v| v, Ok);
    assert!(!readonly.set("2".into()));
    let constant = Binding::constant(7);
    let (initial, _token) = record(&constant);
    assert_eq!(*initial.borrow(), [7]);
    assert!(!constant.set(8));
    #[derive(Clone)]
    struct NoEq(i32);
    assert_eq!(binding.map(NoEq).get().0, 42);
}

#[test]
fn fifo_callbacks_reenter_without_interleaving_notifications() {
    let (_, store) = local(Model::default());
    let log = Rc::new(RefCell::new(Vec::new()));
    let out = log.clone();
    let model = store.clone();
    let _a = number(&store).subscribe(Rc::new(move |v| {
        out.borrow_mut().push(('a', v));
        if v == 1 {
            assert!(model.update(|s| s.value = 2));
            assert!(model.update(|s| s.value = 3));
            assert_eq!(model.snapshot().value, 1);
        }
    }));
    let out = log.clone();
    let _b = number(&store).subscribe(Rc::new(move |v| out.borrow_mut().push(('b', v))));
    assert!(number(&store).set(1));
    assert_eq!(
        *log.borrow(),
        [
            ('a', 0),
            ('b', 0),
            ('a', 1),
            ('b', 1),
            ('a', 2),
            ('b', 2),
            ('a', 3),
            ('b', 3)
        ]
    );
}

#[test]
fn initial_delivery_reentry_and_writeback_are_safe() {
    let (_, store) = local(Model::default());
    let target = number(&store);
    let binding = target.clone();
    let (calls, out) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let n = calls.clone();
    let seen = out.clone();
    let _token = binding.subscribe(Rc::new(move |v| {
        n.set(n.get() + 1);
        seen.set(v);
        assert!(target.set(if v == 0 { 1 } else { v }));
    }));
    assert_eq!(calls.get(), 2);
    assert_eq!(out.get(), 1);
}

#[test]
fn ingress_saturation_is_false_and_fifo_is_preserved() {
    let pool = LocalPool::new();
    let store = Store::with_limits(
        Vec::<i32>::new(),
        Rc::new(LocalExecutor(pool.spawner())),
        Limits {
            ingress: 2,
            tasks: 1,
        },
    );
    let owner = store.clone();
    assert!(store.update(move |state| {
        state.push(0);
        assert!(owner.update(|s| s.push(1)));
        assert!(owner.update(|s| s.push(2)));
        assert!(!owner.update(|s| s.push(3)));
        assert!(!owner.effect(Policy::Parallel, "full", async {}, |_, _| {}));
        assert_eq!(owner.pending_tasks(), 0);
    }));
    assert_eq!(store.snapshot(), [0, 1, 2]);
}

#[test]
fn watch_coalesces_under_saturation_after_complete_publication() {
    let pool = LocalPool::new();
    let store = Store::with_limits(
        0,
        Rc::new(LocalExecutor(pool.spawner())),
        Limits {
            ingress: 1,
            tasks: 1,
        },
    );
    let log = Rc::new(RefCell::new(Vec::new()));
    let out = log.clone();
    let owner = store.clone();
    let _first = store.bind(|s| *s, None).subscribe(Rc::new(move |v| {
        out.borrow_mut().push(('a', v));
        if v == 1 {
            assert!(owner.update(|s| *s = 2));
        }
    }));
    let out = log.clone();
    let _watch = store.watch(store.bind(|s| *s, None), move |_, v| {
        out.borrow_mut().push(('w', v));
    });
    let out = log.clone();
    let _last = store.bind(|s| *s, None).subscribe(Rc::new(move |v| {
        out.borrow_mut().push(('b', v));
    }));
    log.borrow_mut().clear();
    assert!(store.update(|s| *s = 1));
    assert_eq!(
        *log.borrow(),
        [('a', 1), ('b', 1), ('a', 2), ('b', 2), ('w', 2)]
    );
}

#[test]
fn watch_reaction_frees_one_slot_for_update_or_effect() {
    let mut pool = LocalPool::new();
    let store = Store::with_limits(
        Model::default(),
        Rc::new(LocalExecutor(pool.spawner())),
        Limits {
            ingress: 1,
            tasks: 1,
        },
    );
    let _derived = store.watch(number(&store), |store, value| {
        assert!(store.update(move |s| s.argument = value.to_string()));
    });
    let _effect = store.watch(number(&store), |store, value| {
        assert!(
            store.effect(Policy::Serial, "watch", async move { value }, |s, value| {
                s.error = value.to_string();
            })
        );
    });
    pool.run_until_stalled();
    assert!(number(&store).set(42));
    pool.run_until_stalled();
    assert_eq!(store.snapshot().argument, "42");
    assert_eq!(store.snapshot().error, "42");
}

#[test]
fn watch_drop_invalidates_both_mailbox_and_queued_reaction() {
    for saturate in [false, true] {
        let pool = LocalPool::new();
        let store = Store::with_limits(
            0,
            Rc::new(LocalExecutor(pool.spawner())),
            Limits {
                ingress: 1,
                tasks: 1,
            },
        );
        let owner = store.clone();
        let _fill = store.bind(|s| *s, None).subscribe(Rc::new(move |v| {
            if v == 1 && saturate {
                assert!(owner.update(|_| {}));
            }
        }));
        let calls = Rc::new(Cell::new(0));
        let count = calls.clone();
        let capture = Rc::new(());
        let weak = Rc::downgrade(&capture);
        let token = store.watch(store.bind(|s| *s, None), move |_, _| {
            let _ = &capture;
            count.set(count.get() + 1);
        });
        let slot = Rc::new(RefCell::new(Some(token)));
        let cancel = slot.clone();
        let _cancel = store.bind(|s| *s, None).subscribe(Rc::new(move |v| {
            if v == 1 {
                let token = cancel.borrow_mut().take();
                drop(token);
            }
        }));
        assert!(store.update(|s| *s = 1));
        assert_eq!(calls.get(), 1, "only the initial reaction runs");
        assert!(weak.upgrade().is_none());
    }
}

#[test]
fn watch_does_not_own_store_and_close_discards_queued_reaction() {
    let (_, store) = local(0);
    let binding = store.bind(|s| *s, Some(|s, v| *s = v));
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let _token = store.watch(binding.clone(), move |_, _| count.set(count.get() + 1));
    let owner = store.clone();
    let _closer = binding.subscribe(Rc::new(move |v| {
        if v == 1 {
            owner.close();
        }
    }));
    assert!(binding.set(1));
    assert_eq!(calls.get(), 1);
    drop((_closer, store));
    assert!(!binding.set(2));

    let (_, store) = local(0);
    let binding = store.bind(|s| *s, Some(|s, v| *s = v));
    let _token = store.watch(binding.clone(), |_, _| {});
    drop(store);
    assert!(!binding.set(1), "watch tokens are not model owners");
}

#[test]
fn watch_initial_panic_unsubscribes_and_reentry_is_borrow_safe() {
    let (_, store) = local(Model::default());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _token = store.watch(number(&store), |_, _| panic!("initial reaction"));
        }))
        .is_err()
    );
    let _token = store.watch(number(&store), |store, value| {
        if value < 3 {
            assert!(store.update(move |s| s.value = value + 1));
        }
    });
    assert_eq!(number(&store).get(), 3);
    assert!(number(&store).set(4));
}

#[test]
fn watch_foreign_source_coalesces_without_partial_eq() {
    let pool = LocalPool::new();
    let store = Store::with_limits(
        Vec::new(),
        Rc::new(LocalExecutor(pool.spawner())),
        Limits {
            ingress: 1,
            tasks: 1,
        },
    );
    let (_, source) = local(Model::default());
    #[derive(Clone)]
    struct NoEq(i32);
    let _token = store.watch(number(&source).map(NoEq), |store, NoEq(value)| {
        assert!(store.update(move |s| s.push(value)));
    });
    let owner = store.clone();
    store.update(move |_| {
        assert!(owner.update(|_| {}));
        number(&source).set(1);
        number(&source).set(2);
    });
    assert_eq!(store.snapshot(), [0, 2]);
}

#[test]
fn command_captures_argument_now_but_rechecks_enabled_on_dequeue() {
    let (_, store) = local(Model {
        enabled: true,
        argument: "a".into(),
        ..Model::default()
    });
    let command = store.command(
        |s| s.enabled,
        |model, arg| {
            model.update(move |s| {
                s.value += 1;
                s.error = arg;
            });
        },
        text(&store),
    );
    let owner = store.clone();
    let invoke = command.clone();
    store.update(move |_| {
        assert!(owner.update(|s| s.argument = "b".into()));
        assert!(invoke.invoke());
    });
    assert_eq!(store.snapshot().error, "a");
    assert_eq!(store.snapshot().value, 1);
    let owner = store.clone();
    let invoke = command.clone();
    store.update(move |_| {
        owner.update(|s| s.enabled = false);
        assert!(invoke.invoke());
    });
    assert_eq!(store.snapshot().value, 1);
    assert!(!command.invoke());
}

#[test]
fn dropping_tokens_skips_later_listener_and_releases_captures() {
    let (_, store) = local(Model::default());
    let later = Rc::new(RefCell::new(None::<Subscription>));
    let slot = later.clone();
    let _first = number(&store).subscribe(Rc::new(move |v| {
        if v == 1 {
            let token = slot.borrow_mut().take();
            drop(token);
        }
    }));
    let captured = Rc::new(());
    let weak = Rc::downgrade(&captured);
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let token = number(&store).subscribe(Rc::new(move |_| {
        let _ = &captured;
        count.set(count.get() + 1);
    }));
    later.replace(Some(token));
    number(&store).set(1);
    assert_eq!(calls.get(), 1);
    assert!(weak.upgrade().is_none());
}

#[test]
fn subscription_callback_can_own_store_without_cycle() {
    let (_, store) = local(Rc::new(()));
    let weak = Rc::downgrade(&store.snapshot());
    let owner = store.clone();
    let binding = store.bind(Rc::strong_count, None);
    let token = binding.subscribe(Rc::new(move |_| {
        let _ = owner.snapshot();
    }));
    drop(binding);
    drop(store);
    assert!(weak.upgrade().is_some());
    drop(token);
    assert!(weak.upgrade().is_none());
}

#[test]
fn initial_callback_panic_unsubscribes_and_later_update_works() {
    let (_, store) = local(Model::default());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _token = number(&store).subscribe(Rc::new(|_| panic!("initial")));
        }))
        .is_err()
    );
    assert!(number(&store).set(2));
    assert_eq!(store.snapshot().value, 2);
}

#[test]
fn latest_aborts_old_reverse_result_and_independent_slots_complete() {
    let (mut pool, store) = local(Vec::new());
    let (old_tx, old_rx) = oneshot::channel::<i32>();
    let (new_tx, new_rx) = oneshot::channel();
    assert!(store.effect(
        Policy::Latest,
        "search",
        async move { old_rx.await.unwrap() },
        |s, r| s.push(r)
    ));
    pool.run_until_stalled();
    assert!(store.effect(
        Policy::Latest,
        "search",
        async move { new_rx.await.unwrap() },
        |s, r| s.push(r)
    ));
    assert!(store.effect(Policy::Latest, "other", async { 30 }, |s, r| s.push(r)));
    new_tx.send(20).unwrap();
    pool.run_until_stalled();
    assert!(old_tx.send(10).is_err());
    let mut result = store.snapshot();
    result.sort();
    assert_eq!(result, [20, 30]);
    assert_eq!(store.pending_tasks(), 0);
}

#[test]
fn latest_suppresses_result_already_in_fifo() {
    let executor = Rc::new(Inline::default());
    let store = Store::new(Vec::new(), executor.clone());
    let owner = store.clone();
    executor.after.replace(Some(Box::new(move || {
        assert!(owner.effect(Policy::Latest, "slot", async { 2 }, |s, r| s.push(r)));
    })));
    assert!(store.effect(Policy::Latest, "slot", async { 1 }, |s, r| s.push(r)));
    assert_eq!(store.snapshot(), [2]);
}

#[test]
fn serial_does_not_poll_successor_until_predecessor_applied() {
    let (mut pool, store) = local(Vec::new());
    let (first_tx, first_rx) = oneshot::channel();
    let (last_tx, last_rx) = oneshot::channel();
    let polled = Rc::new(Cell::new(false));
    let seen = polled.clone();
    assert!(store.effect(
        Policy::Serial,
        "queue",
        async move { first_rx.await.unwrap() },
        |s, r| s.push(r)
    ));
    let source = store.bind(|s| s.clone(), None);
    assert!(store.effect(
        Policy::Serial,
        "queue",
        async move {
            seen.set(true);
            assert_eq!(source.get(), [1]);
            last_rx.await.unwrap()
        },
        |s, r| s.push(r)
    ));
    last_tx.send(2).unwrap();
    pool.run_until_stalled();
    assert!(!polled.get());
    assert!(store.snapshot().is_empty());
    first_tx.send(1).unwrap();
    pool.run_until_stalled();
    assert!(polled.get());
    assert_eq!(store.snapshot(), [1, 2]);
}

#[test]
fn parallel_all_complete_in_result_order() {
    let (mut pool, store) = local(Vec::new());
    let (a, ar) = oneshot::channel();
    let (b, br) = oneshot::channel();
    for rx in [ar, br] {
        assert!(store.effect(
            Policy::Parallel,
            "same",
            async move { rx.await.unwrap() },
            |s, r| s.push(r)
        ));
    }
    pool.run_until_stalled();
    b.send(2).unwrap();
    pool.run_until_stalled();
    a.send(1).unwrap();
    pool.run_until_stalled();
    assert_eq!(store.snapshot(), [2, 1]);
}

#[test]
fn task_saturation_counts_aborted_wrappers_and_never_polls_rejected_work() {
    let mut pool = LocalPool::new();
    let store = Store::with_limits(
        0,
        Rc::new(LocalExecutor(pool.spawner())),
        Limits {
            ingress: 4,
            tasks: 2,
        },
    );
    assert!(store.effect(Policy::Latest, "same", pending::<()>(), |_, _| {}));
    assert!(store.effect(Policy::Latest, "same", pending::<()>(), |_, _| {}));
    assert!(!store.effect(
        Policy::Parallel,
        "other",
        async { panic!("rejected") },
        |_, _: ()| {}
    ));
    assert_eq!(store.pending_tasks(), 2);
    pool.run_until_stalled();
    assert_eq!(store.pending_tasks(), 1);
    assert!(store.effect(Policy::Parallel, "other", async { 7 }, |s, r| *s = r));
    pool.run_until_stalled();
    assert_eq!(store.snapshot(), 7);
}

#[test]
fn accepted_completion_has_reserved_capacity_during_ingress_saturation() {
    let executor = Rc::new(Inline::default());
    let store = Store::with_limits(
        Vec::new(),
        executor.clone(),
        Limits {
            ingress: 1,
            tasks: 1,
        },
    );
    let owner = store.clone();
    executor.after.replace(Some(Box::new(move || {
        assert!(owner.update(|s| s.push(2)));
        assert!(!owner.update(|s| s.push(3)));
    })));
    store.effect(Policy::Parallel, "result", async { 1 }, |s, r| s.push(r));
    assert_eq!(store.snapshot(), [1, 2]);
}

struct DropCall(Option<Box<dyn FnOnce()>>);
impl Drop for DropCall {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

#[test]
fn close_freezes_snapshot_clears_fifo_releases_callbacks_and_never_polls_future() {
    let (mut pool, store) = local(Model::default());
    let binding = number(&store);
    let capture = Rc::new(());
    let weak = Rc::downgrade(&capture);
    let _token = binding.subscribe(Rc::new(move |_| {
        let _ = &capture;
    }));
    let ran = Rc::new(Cell::new(false));
    let flag = ran.clone();
    store.effect(
        Policy::Parallel,
        "pending",
        async move {
            flag.set(true);
            9
        },
        |s, r| s.value = r,
    );
    let owner = store.clone();
    store.update(move |s| {
        s.value = 2;
        owner.update(|s| s.value = 3);
        owner.close();
    });
    pool.run_until_stalled();
    assert!(!ran.get());
    assert_eq!(binding.get(), 0); // close inside update freezes last published state
    assert!(weak.upgrade().is_none());
    assert!(!binding.set(9));
    assert!(!store.update(|_| {}));
    assert_eq!(store.pending_tasks(), 0);
    store.close();
    drop(store);
    assert_eq!(binding.get(), 0);
    let (initial, _token) = record(&binding);
    assert_eq!(*initial.borrow(), [0]);
}

#[test]
fn user_future_poll_and_drop_and_callback_drop_can_reenter() {
    let (mut pool, store) = local(0);
    let owner = store.clone();
    let probe = DropCall(Some(Box::new(move || {
        assert!(owner.update(|s| *s += 1));
    })));
    store.effect(
        Policy::Latest,
        "slot",
        async move {
            let _probe = probe;
            pending::<()>().await
        },
        |_, _| {},
    );
    pool.run_until_stalled();
    let owner = store.clone();
    store.effect(
        Policy::Latest,
        "slot",
        async move {
            assert!(owner.update(|s| *s += 2));
            4
        },
        |s, r| *s += r,
    );
    pool.run_until_stalled();
    assert_eq!(store.snapshot(), 7);
    let owner = store.clone();
    let probe = DropCall(Some(Box::new(move || {
        assert!(owner.update(|s| *s += 8));
    })));
    let token = store.bind(|s| *s, None).subscribe(Rc::new(move |_| {
        let _ = &probe;
    }));
    drop(token);
    assert_eq!(store.snapshot(), 15);
}

#[test]
fn weak_tasks_do_not_keep_store_alive_and_binding_retains_final_snapshot() {
    let (mut pool, store) = local(Rc::new(()));
    let weak = Rc::downgrade(&store.snapshot());
    let binding = store.bind(Rc::strong_count, None);
    let (tx, rx) = oneshot::channel::<()>();
    store.effect(
        Policy::Parallel,
        "wait",
        async move { rx.await.unwrap() },
        |_, _| {},
    );
    pool.run_until_stalled();
    drop(store);
    assert!(!binding.set(0));
    pool.run_until_stalled();
    assert!(tx.send(()).is_err());
    drop(binding);
    assert!(weak.upgrade().is_none());
}

type ValidationRequests = Rc<RefCell<Vec<(String, oneshot::Sender<Result<(), String>>)>>>;
fn validate(store: &Store<Model>, requests: ValidationRequests) -> Subscription {
    validation(
        store,
        text(store),
        error(store),
        pending_binding(store),
        move |value| {
            let (tx, rx) = oneshot::channel();
            requests.borrow_mut().push((value, tx));
            async move { rx.await.unwrap() }.boxed_local()
        },
    )
}

#[test]
fn validation_initial_pending_error_reverse_and_unmount() {
    let (mut pool, store) = local(Model::default());
    let requests = ValidationRequests::default();
    let token = validate(&store, requests.clone());
    assert_eq!(requests.borrow()[0].0, "");
    assert!(store.snapshot().pending);
    pool.run_until_stalled();
    let old = requests.borrow_mut().remove(0).1;
    text(&store).set("new".into());
    let newest = requests.borrow_mut().remove(0).1;
    newest.send(Err("invalid".into())).unwrap();
    pool.run_until_stalled();
    assert!(old.send(Ok(())).is_err());
    assert!(!store.snapshot().pending);
    assert_eq!(store.snapshot().error, "invalid");
    text(&store).set("valid".into());
    assert_eq!(store.snapshot().error, "");
    requests.borrow_mut().remove(0).1.send(Ok(())).unwrap();
    pool.run_until_stalled();
    assert!(!store.snapshot().pending);
    text(&store).set("unmounted".into());
    pool.run_until_stalled();
    let canceled = requests.borrow_mut().remove(0).1;
    drop(token);
    assert!(!store.snapshot().pending);
    pool.run_until_stalled();
    assert!(canceled.send(Err("stale".into())).is_err());
    assert_eq!(store.snapshot().error, "");
}

#[test]
fn validation_tokens_are_independent() {
    let (mut pool, store) = local(Model::default());
    let a = ValidationRequests::default();
    let b = ValidationRequests::default();
    let one = validate(&store, a.clone());
    let two = validate(&store, b.clone());
    pool.run_until_stalled();
    drop(one);
    b.borrow_mut()
        .remove(0)
        .1
        .send(Err("second".into()))
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(store.snapshot().error, "second");
    assert!(a.borrow_mut().remove(0).1.send(Ok(())).is_err());
    drop(two);
}

#[test]
fn validation_drop_suppresses_already_queued_result() {
    let executor = Rc::new(Inline::default());
    let store = Store::new(Model::default(), executor.clone());
    let token = validation(
        &store,
        text(&store),
        error(&store),
        pending_binding(&store),
        |value| {
            async move {
                if value.is_empty() {
                    Ok(())
                } else {
                    Err("old error".into())
                }
            }
            .boxed_local()
        },
    );
    assert_eq!(store.snapshot().error, "");
    let slot = Rc::new(RefCell::new(Some(token)));
    let cancel = slot.clone();
    executor.after.replace(Some(Box::new(move || {
        let token = cancel.borrow_mut().take();
        drop(token);
    })));
    text(&store).set("next".into());
    assert_eq!(store.snapshot().error, "");
    assert!(!store.snapshot().pending);
}

#[test]
fn validation_task_saturation_reports_error_and_clears_pending() {
    let pool = LocalPool::new();
    let store = Store::with_limits(
        Model::default(),
        Rc::new(LocalExecutor(pool.spawner())),
        Limits {
            ingress: 8,
            tasks: 0,
        },
    );
    let _token = validation(
        &store,
        text(&store),
        error(&store),
        pending_binding(&store),
        |_| pending().boxed_local(),
    );
    assert!(!store.snapshot().pending);
    assert_eq!(store.snapshot().error, "validation task limit reached");
}

#[test]
fn item_key_validation_for_hierarchical_scopes() {
    for key in ["", "/", "parent/child", "/row", "row/"] {
        assert!(!valid_item_key(key), "accepted invalid key {key:?}");
    }
    for key in ["row", "row-1", "行", " ", "row\\child", "row\nchild"] {
        assert!(valid_item_key(key), "rejected valid key {key:?}");
    }
    let (_, model) = local(Model::default());
    let literal = Keyed {
        key: "parent/child".into(),
        model,
    };
    assert!(!valid_item_key(&literal.key));
}

#[test]
#[should_panic(expected = "item key must be nonempty and contain no '/'")]
fn keyed_constructor_rejects_empty_key() {
    let (_, model) = local(Model::default());
    let _ = Keyed::new("", model);
}

#[test]
#[should_panic(expected = "item key must be nonempty and contain no '/'")]
fn keyed_constructor_rejects_slash_key() {
    let (_, model) = local(Model::default());
    let _ = Keyed::new(String::from("parent/child"), model);
}

#[test]
fn keyed_item_identity_and_state_survive_mount_token_eviction() {
    let (_, item) = local(Model::default());
    let keyed = Keyed::new("row", item.clone());
    let same = keyed.clone();
    assert!(keyed == same);
    let (_, other) = local(Model::default());
    assert!(keyed != Keyed::new("row", other));
    let source = Binding::constant(vec![keyed]);
    let mounted = source.get().remove(0);
    let token = number(&mounted.model).subscribe(Rc::new(|_| {}));
    number(&mounted.model).set(12);
    drop(token);
    drop(mounted);
    let remount = source.get().remove(0);
    assert_eq!(remount.model.snapshot().value, 12);
    let (values, _token) = record(&number(&remount.model));
    assert_eq!(*values.borrow(), [12]);
    assert_eq!(
        Viewport::default(),
        Viewport {
            start: 0,
            count: 20,
            row_height: 48.0
        }
    );
    assert_eq!(Style::default().placement.row_span, 1);
    assert_eq!(Placement::default().column_span, 1);
}

fn hooks(
    log: Rc<RefCell<Vec<&'static str>>>,
    confirm: LocalBoxFuture<'static, bool>,
    drain: LocalBoxFuture<'static, ()>,
) -> WindowHooks {
    let confirm = Rc::new(RefCell::new(Some(confirm)));
    let drain = Rc::new(RefCell::new(Some(drain)));
    let cancels = log.clone();
    WindowHooks {
        confirm: Rc::new(move || {
            confirm
                .borrow_mut()
                .take()
                .unwrap_or_else(|| async { true }.boxed_local())
        }),
        cancel: Rc::new(move || cancels.borrow_mut().push("cancel")),
        drain: Rc::new(move || {
            drain
                .borrow_mut()
                .take()
                .unwrap_or_else(|| async {}.boxed_local())
        }),
        close: Rc::new(move || log.borrow_mut().push("close")),
    }
}

#[test]
fn shutdown_confirms_all_then_cancels_drains_and_closes_multiple_windows() {
    let mut pool = LocalPool::new();
    let session = Session::new();
    let coordinator = session.shutdown();
    let log = Rc::new(RefCell::new(Vec::new()));
    let (confirm, confirmed) = oneshot::channel();
    let (drain, drained) = oneshot::channel();
    let _one = session
        .register_window(
            "one",
            hooks(
                log.clone(),
                async move { confirmed.await.unwrap() }.boxed_local(),
                async move { drained.await.unwrap() }.boxed_local(),
            ),
        )
        .unwrap();
    let _two = session
        .register_window(
            "two",
            hooks(
                log.clone(),
                async { true }.boxed_local(),
                async {}.boxed_local(),
            ),
        )
        .unwrap();
    assert_eq!(session.window_count(), 2);
    let output = Rc::new(Cell::new(None));
    let result = output.clone();
    let shutdown = coordinator.clone();
    pool.spawner()
        .spawn_local(async move {
            result.set(Some(
                shutdown.request_shutdown(Duration::from_secs(5)).await,
            ));
        })
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(coordinator.phase(), ShutdownPhase::Confirming);
    assert!(log.borrow().is_empty());
    assert_eq!(
        block_on(coordinator.request_shutdown(Duration::ZERO)),
        ShutdownOutcome::Busy
    );
    confirm.send(true).unwrap();
    pool.run_until_stalled();
    assert_eq!(coordinator.phase(), ShutdownPhase::Draining);
    assert_eq!(*log.borrow(), ["cancel", "cancel"]);
    assert_eq!(output.get(), None);
    drain.send(()).unwrap();
    pool.run_until_stalled();
    assert_eq!(output.get(), Some(ShutdownOutcome::Closed));
    assert_eq!(*log.borrow(), ["cancel", "cancel", "close", "close"]);
    assert_eq!(session.window_count(), 0);
    assert_eq!(
        block_on(coordinator.request_shutdown(Duration::ZERO)),
        ShutdownOutcome::Closed
    );
}

#[test]
fn shutdown_denial_cancel_and_dropped_request_leave_session_running() {
    let session = Session::new();
    let coordinator = session.shutdown();
    let log = Rc::new(RefCell::new(Vec::new()));
    let token = session
        .register_window(
            "one",
            hooks(
                log.clone(),
                async { false }.boxed_local(),
                pending().boxed_local(),
            ),
        )
        .unwrap();
    assert_eq!(
        block_on(coordinator.request_shutdown(Duration::from_secs(1))),
        ShutdownOutcome::Cancelled
    );
    assert!(log.borrow().is_empty());
    drop(token);
    let (tx, rx) = oneshot::channel();
    let _token = session
        .register_window(
            "one",
            hooks(
                log.clone(),
                async move { rx.await.unwrap() }.boxed_local(),
                pending().boxed_local(),
            ),
        )
        .unwrap();
    let mut request = Box::pin(coordinator.request_shutdown(Duration::from_secs(10)));
    assert!(request.as_mut().now_or_never().is_none());
    assert!(coordinator.cancel_shutdown());
    assert_eq!(block_on(request), ShutdownOutcome::Cancelled);
    assert!(tx.send(true).is_err());
    assert_eq!(coordinator.phase(), ShutdownPhase::Running);
    let mut request = Box::pin(coordinator.request_shutdown(Duration::from_secs(10)));
    assert!(request.as_mut().now_or_never().is_none());
    assert_eq!(coordinator.phase(), ShutdownPhase::Draining);
    drop(request);
    assert_eq!(coordinator.phase(), ShutdownPhase::Running);
}

#[test]
fn shutdown_real_timer_timeout_in_confirmation_and_drain() {
    for confirming in [true, false] {
        let session = Session::new();
        let coordinator = session.shutdown();
        let log = Rc::new(RefCell::new(Vec::new()));
        let confirm = if confirming {
            pending().boxed_local()
        } else {
            async { true }.boxed_local()
        };
        let _token = session
            .register_window(
                "stuck",
                hooks(log.clone(), confirm, pending().boxed_local()),
            )
            .unwrap();
        assert_eq!(
            block_on(coordinator.request_shutdown(Duration::from_millis(2))),
            ShutdownOutcome::TimedOut
        );
        assert_eq!(coordinator.phase(), ShutdownPhase::Closed);
        assert_eq!(log.borrow().last(), Some(&"close"));
        assert_eq!(session.window_count(), 0);
    }
}

#[test]
fn window_registration_bookkeeping_and_hook_reentry_are_borrow_safe() {
    let session = Session::new();
    let coordinator = session.shutdown();
    let log = Rc::new(RefCell::new(Vec::new()));
    let first = session
        .register_window(
            "one",
            hooks(
                log.clone(),
                async { true }.boxed_local(),
                async {}.boxed_local(),
            ),
        )
        .unwrap();
    assert!(
        session
            .register_window(
                "one",
                hooks(
                    log.clone(),
                    async { true }.boxed_local(),
                    async {}.boxed_local()
                )
            )
            .is_err()
    );
    drop(first);
    assert_eq!(session.window_count(), 0);
    let session2 = session.clone();
    let mut h = hooks(
        log.clone(),
        async { true }.boxed_local(),
        async {}.boxed_local(),
    );
    h.cancel = Rc::new(move || {
        assert_eq!(session2.window_count(), 0);
    });
    let _token = session.register_window("one", h).unwrap();
    coordinator.close_now();
    coordinator.close_now();
    assert_eq!(*log.borrow(), ["close"]);
    assert!(
        session
            .register_window(
                "late",
                hooks(log, async { true }.boxed_local(), async {}.boxed_local())
            )
            .is_err()
    );
}

struct Host {
    executor: Rc<dyn Executor>,
    sessions: RefCell<Vec<Session>>,
    fail: bool,
}
impl UiHost for Host {
    fn executor(&self) -> Rc<dyn Executor> {
        self.executor.clone()
    }
    fn attach(&self, session: Session) -> Result<(), String> {
        if self.fail {
            Err("attach refused".into())
        } else {
            self.sessions.borrow_mut().push(session);
            Ok(())
        }
    }
}
#[test]
fn async_host_attach_is_lazy_nonblocking_and_uses_running_executor() {
    let mut pool = LocalPool::new();
    let host = Host {
        executor: Rc::new(LocalExecutor(pool.spawner())),
        sessions: RefCell::new(Vec::new()),
        fail: false,
    };
    let ran = Rc::new(Cell::new(false));
    let flag = ran.clone();
    let future = run_async(&host, move |executor| {
        executor.spawn(
            async move {
                flag.set(true);
            }
            .boxed_local(),
        );
        Session::new()
    });
    assert!(host.sessions.borrow().is_empty());
    let session = block_on(future).unwrap();
    assert_eq!(host.sessions.borrow().len(), 1);
    assert!(!ran.get());
    pool.run_until_stalled();
    assert!(ran.get());
    assert_eq!(session.shutdown().phase(), ShutdownPhase::Running);
    let failed = Host { fail: true, ..host };
    let retained = Session::new();
    let make = retained.clone();
    assert_eq!(
        block_on(run_async(&failed, |_| make)).err().as_deref(),
        Some("attach refused")
    );
    assert_eq!(retained.shutdown().phase(), ShutdownPhase::Closed);
}

#[test]
fn blocking_pool_bounded_jobs_cancel_panic_shutdown_and_nonblocking_drop() {
    let mut pool = BlockingPool::new(1, 1);
    let (started, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let first = pool
        .try_spawn(move || {
            started.send(()).unwrap();
            wait.recv().unwrap();
            1
        })
        .unwrap();
    ready.recv().unwrap();
    let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = ran.clone();
    let canceled = pool
        .try_spawn(move || flag.store(true, std::sync::atomic::Ordering::SeqCst))
        .unwrap();
    assert!(matches!(pool.try_spawn(|| 3), Err(TaskError::Full)));
    drop(canceled);
    release.send(()).unwrap();
    pool.shutdown();
    assert_eq!(block_on(first), Ok(1));
    assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
    assert!(matches!(pool.try_spawn(|| 4), Err(TaskError::Shutdown)));
    pool.shutdown();
    let mut pool = BlockingPool::new(1, 1);
    assert_eq!(
        block_on(pool.try_spawn(|| panic!("probe")).unwrap()),
        Err(TaskError::Panic("probe".into()))
    );
    assert_eq!(block_on(pool.try_spawn(|| 42).unwrap()), Ok(42));
    pool.shutdown();
    let pool = BlockingPool::new(1, 1);
    let (started, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let future = pool
        .try_spawn(move || {
            started.send(()).unwrap();
            wait.recv().unwrap();
            99
        })
        .unwrap();
    ready.recv().unwrap();
    drop(pool); // joining Drop would deadlock here
    release.send(()).unwrap();
    assert_eq!(block_on(future), Ok(99));
}

#[test]
fn executor_discard_and_user_panics_release_task_permits() {
    struct Discard;
    impl Executor for Discard {
        fn spawn(&self, _: LocalBoxFuture<'static, ()>) {}
    }
    let store = Store::with_limits(
        0,
        Rc::new(Discard),
        Limits {
            ingress: 1,
            tasks: 1,
        },
    );
    for _ in 0..3 {
        assert!(store.effect(Policy::Serial, "queue", pending::<()>(), |_, _| {}));
        assert_eq!(store.pending_tasks(), 0);
    }
    let store = Store::with_limits(
        0,
        Rc::new(Inline::default()),
        Limits {
            ingress: 1,
            tasks: 1,
        },
    );
    for in_poll in [true, false] {
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                store.effect(
                    Policy::Serial,
                    "queue",
                    async move {
                        assert!(!in_poll, "poll panic");
                    },
                    |_, _| panic!("apply panic"),
                );
            }))
            .is_err()
        );
        assert_eq!(store.pending_tasks(), 0);
        assert!(store.update(|s| *s += 1));
    }
    assert_eq!(store.snapshot(), 2);
}

#[test]
fn serial_slots_are_independent_and_close_drops_waiting_successors() {
    let (mut pool, store) = local(Vec::new());
    let (tx, rx) = oneshot::channel::<i32>();
    let (unused_tx, unused_rx) = oneshot::channel::<i32>();
    store.effect(
        Policy::Serial,
        "one",
        async move { rx.await.unwrap() },
        |s, r| s.push(r),
    );
    store.effect(
        Policy::Serial,
        "one",
        async move { unused_rx.await.unwrap() },
        |s, r| s.push(r),
    );
    store.effect(Policy::Serial, "two", async { 2 }, |s, r| s.push(r));
    pool.run_until_stalled();
    assert_eq!(store.snapshot(), [2]);
    store.close();
    pool.run_until_stalled();
    assert!(tx.send(1).is_err());
    assert!(unused_tx.send(3).is_err());
    assert_eq!(store.snapshot(), [2]);
}

#[test]
fn validation_pending_callback_reentry_cannot_supersede_newer_source() {
    let (mut pool, store) = local(Model::default());
    let owner = store.clone();
    let _pending = pending_binding(&store).subscribe(Rc::new(move |pending| {
        if pending && owner.snapshot().argument.is_empty() {
            text(&owner).set("newer".into());
        }
    }));
    let requests = ValidationRequests::default();
    let _token = validate(&store, requests.clone());
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(requests.borrow()[0].0, "newer");
    requests
        .borrow_mut()
        .remove(0)
        .1
        .send(Err("newer result".into()))
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(store.snapshot().error, "newer result");
    assert!(!store.snapshot().pending);
}

#[test]
fn validation_unmount_inside_validator_never_polls_returned_future() {
    let (mut pool, store) = local(Model::default());
    let slot = Rc::new(RefCell::new(None::<Subscription>));
    let unmount = slot.clone();
    let polled = Rc::new(Cell::new(false));
    let seen = polled.clone();
    let token = validation(
        &store,
        text(&store),
        error(&store),
        pending_binding(&store),
        move |value| {
            if value == "unmount" {
                let token = unmount.borrow_mut().take();
                drop(token);
                let seen = seen.clone();
                async move {
                    seen.set(true);
                    Ok(())
                }
                .boxed_local()
            } else {
                pending().boxed_local()
            }
        },
    );
    slot.replace(Some(token));
    pool.run_until_stalled();
    text(&store).set("unmount".into());
    pool.run_until_stalled();
    assert!(!polled.get());
    assert!(!store.snapshot().pending);
    assert_eq!(store.pending_tasks(), 0);
}

#[test]
fn validation_unmount_suppresses_writes_queued_after_result_dequeues() {
    let executor = Rc::new(Inline::default());
    let store = Store::new(Model::default(), executor.clone());
    let token = validation(
        &store,
        text(&store),
        error(&store),
        pending_binding(&store),
        |_| async { Err("stale".into()) }.boxed_local(),
    );
    let slot = Rc::new(RefCell::new(Some(token)));
    let owner = store.clone();
    executor.after.replace(Some(Box::new(move || {
        owner.update(move |_| {
            let token = slot.borrow_mut().take();
            drop(token);
        });
    })));
    text(&store).set("changed".into());
    assert_eq!(store.snapshot().error, "");
    assert!(!store.snapshot().pending);
}

#[test]
fn shutdown_window_unmount_aborts_confirmation_and_drain_waits() {
    for confirming in [true, false] {
        let session = Session::new();
        let coordinator = session.shutdown();
        let log = Rc::new(RefCell::new(Vec::new()));
        let confirm = if confirming {
            pending().boxed_local()
        } else {
            async { true }.boxed_local()
        };
        let token = session
            .register_window(
                "removed",
                hooks(log.clone(), confirm, pending().boxed_local()),
            )
            .unwrap();
        let mut request = Box::pin(coordinator.request_shutdown(Duration::from_secs(30)));
        assert!(request.as_mut().now_or_never().is_none());
        drop(token);
        assert_eq!(session.window_count(), 0);
        assert_eq!(block_on(request), ShutdownOutcome::Closed);
        assert!(!log.borrow().contains(&"close"));
    }
}

#[test]
fn cancelled_old_shutdown_cannot_poll_work_or_modify_new_request() {
    let session = Session::new();
    let coordinator = session.shutdown();
    let log = Rc::new(RefCell::new(Vec::new()));
    let _token = session
        .register_window(
            "one",
            hooks(log, pending().boxed_local(), pending().boxed_local()),
        )
        .unwrap();
    let mut old = Box::pin(coordinator.request_shutdown(Duration::from_secs(30)));
    assert!(old.as_mut().now_or_never().is_none());
    assert!(coordinator.cancel_shutdown());
    let mut new = Box::pin(coordinator.request_shutdown(Duration::from_secs(30)));
    assert!(new.as_mut().now_or_never().is_none());
    assert_eq!(coordinator.phase(), ShutdownPhase::Draining);
    assert_eq!(block_on(old), ShutdownOutcome::Cancelled);
    assert_eq!(coordinator.phase(), ShutdownPhase::Draining);
    assert!(coordinator.cancel_shutdown());
    assert_eq!(block_on(new), ShutdownOutcome::Cancelled);
    assert_eq!(coordinator.phase(), ShutdownPhase::Running);
}

#[test]
fn shutdown_timeout_cancels_once_and_hooks_can_force_close_reentrantly() {
    let session = Session::new();
    let coordinator = session.shutdown();
    let log = Rc::new(RefCell::new(Vec::new()));
    let _token = session
        .register_window(
            "one",
            hooks(
                log.clone(),
                async { true }.boxed_local(),
                pending().boxed_local(),
            ),
        )
        .unwrap();
    assert_eq!(
        block_on(coordinator.request_shutdown(Duration::from_millis(2))),
        ShutdownOutcome::TimedOut
    );
    assert_eq!(*log.borrow(), ["cancel", "close"]);
    let session = Session::new();
    let coordinator = session.shutdown();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut h = hooks(
        log.clone(),
        async { true }.boxed_local(),
        pending().boxed_local(),
    );
    let shutdown = coordinator.clone();
    h.cancel = Rc::new(move || shutdown.close_now());
    let _token = session.register_window("one", h).unwrap();
    assert_eq!(
        block_on(coordinator.request_shutdown(Duration::from_secs(30))),
        ShutdownOutcome::Closed
    );
    assert_eq!(*log.borrow(), ["close"]);
}

#[test]
fn validation_status_and_unmount_reset_retry_through_one_slot_ingress() {
    let mut pool = LocalPool::new();
    let store = Store::with_limits(
        Model::default(),
        Rc::new(LocalExecutor(pool.spawner())),
        Limits {
            ingress: 1,
            tasks: 4,
        },
    );
    let requests = ValidationRequests::default();
    let token = validate(&store, requests.clone());
    pool.run_until_stalled();
    requests.borrow_mut().remove(0).1.send(Ok(())).unwrap();
    pool.run_until_stalled();
    assert!(!store.snapshot().pending);
    text(&store).set("new".into());
    assert!(store.snapshot().pending);
    requests
        .borrow_mut()
        .remove(0)
        .1
        .send(Err("new error".into()))
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(store.snapshot().error, "new error");
    assert!(!store.snapshot().pending);
    text(&store).set("unmount".into());
    let owner = store.clone();
    store.update(move |_| {
        assert!(owner.update(|_| {})); // fill the only ingress slot
        drop(token);
    });
    pool.run_until_stalled();
    assert!(!store.snapshot().pending);
    assert_eq!(store.pending_tasks(), 0);
}

#[test]
fn validation_ingress_saturation_reports_error_without_stuck_pending() {
    let pool = LocalPool::new();
    let store = Store::with_limits(
        Model::default(),
        Rc::new(LocalExecutor(pool.spawner())),
        Limits {
            ingress: 1,
            tasks: 4,
        },
    );
    let _token = validation(
        &store,
        text(&store),
        error(&store),
        pending_binding(&store),
        |_| pending().boxed_local(),
    );
    let owner = store.clone();
    let _fill = text(&store).subscribe(Rc::new(move |value| {
        if value == "full" {
            owner.update(|_| {});
        }
    }));
    // Register the filler first on a second source binding so its FIFO event is
    // already queued when the validation source notification is delivered.
    store.update(|s| s.argument = "full".into());
    // Either an accepted request or explicit overload is valid; both must later
    // settle. Force task overload deterministically with the other task slots.
    for _ in 0..4 {
        store.effect(Policy::Parallel, "fill", pending::<()>(), |_, _| {});
    }
    text(&store).set("overloaded".into());
    assert_eq!(store.snapshot().error, "validation task limit reached");
    assert!(!store.snapshot().pending);
}

#[test]
fn window_can_unmount_inside_its_own_confirmation_hook() {
    let session = Session::new();
    let slot = Rc::new(RefCell::new(None::<Subscription>));
    let unmount = slot.clone();
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut h = hooks(
        log.clone(),
        async { true }.boxed_local(),
        async {}.boxed_local(),
    );
    h.confirm = Rc::new(move || {
        let token = unmount.borrow_mut().take();
        drop(token);
        pending().boxed_local()
    });
    slot.replace(Some(session.register_window("self-unmount", h).unwrap()));
    assert_eq!(
        block_on(session.shutdown().request_shutdown(Duration::from_secs(30))),
        ShutdownOutcome::Closed
    );
    assert!(log.borrow().is_empty());
}
