//! Futures never leave the UI thread. Only numeric wake IDs cross the proxy.
use futures::{
    future::LocalBoxFuture,
    task::{ArcWake, waker_ref},
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};
use wakui_runtime::Executor;

type Notify = Arc<dyn Fn(usize) + Send + Sync>;
struct Wake {
    id: usize,
    queued: AtomicBool,
    notify: Notify,
}
impl ArcWake for Wake {
    fn wake_by_ref(this: &Arc<Self>) {
        if !this.queued.swap(true, Ordering::AcqRel) {
            (this.notify)(this.id);
        }
    }
}
struct Task {
    future: LocalBoxFuture<'static, ()>,
    wake: Arc<Wake>,
}
pub(super) struct Scheduler {
    tasks: RefCell<HashMap<usize, Task>>,
    ready: RefCell<VecDeque<usize>>,
    next: Cell<usize>,
    closed: Cell<bool>,
    notify: Notify,
}
impl Scheduler {
    pub fn new(notify: Notify) -> Self {
        Self {
            tasks: RefCell::default(),
            ready: RefCell::default(),
            next: Cell::new(0),
            closed: Cell::new(false),
            notify,
        }
    }
    pub fn ready(&self, id: usize) {
        if !self.closed.get() && self.tasks.borrow().contains_key(&id) {
            self.ready.borrow_mut().push_back(id);
        }
    }
    /// Bounded even when futures continually spawn more immediately-ready work.
    pub fn turn(&self) -> bool {
        for _ in 0..64 {
            if self.closed.get() {
                break;
            }
            let id = self.ready.borrow_mut().pop_front();
            let Some(id) = id else { break };
            let task = self.tasks.borrow_mut().remove(&id);
            let Some(mut task) = task else { continue };
            task.wake.queued.store(false, Ordering::Release);
            let waker = waker_ref(&task.wake);
            let result = task.future.as_mut().poll(&mut Context::from_waker(&waker));
            if result == Poll::Pending && !self.closed.get() {
                self.tasks.borrow_mut().insert(id, task);
            }
        }
        !self.ready.borrow().is_empty()
    }
    pub fn close(&self) {
        self.closed.set(true);
        self.ready.borrow_mut().clear();
        let tasks = self.tasks.take();
        drop(tasks);
    }
}
impl Executor for Scheduler {
    fn spawn(&self, future: LocalBoxFuture<'static, ()>) {
        if self.closed.get() {
            return;
        }
        let id = self.next.get();
        self.next.set(id.checked_add(1).expect("task ID exhausted"));
        self.tasks.borrow_mut().insert(
            id,
            Task {
                future,
                wake: Arc::new(Wake {
                    id,
                    queued: AtomicBool::new(true),
                    notify: self.notify.clone(),
                }),
            },
        );
        // The native event loop starts only after make returns and root attachment.
        (self.notify)(id);
    }
}
impl Drop for Scheduler {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{rc::Rc, sync::Mutex};
    #[test]
    fn spawn_is_deferred_and_close_drops_unpolled_future() {
        let ids = Arc::new(Mutex::new(Vec::new()));
        let out = ids.clone();
        let scheduler = Scheduler::new(Arc::new(move |id| out.lock().unwrap().push(id)));
        let flag = Rc::new(Cell::new(false));
        let out = flag.clone();
        scheduler.spawn(Box::pin(async move {
            out.set(true);
        }));
        assert!(!flag.get());
        scheduler.ready(ids.lock().unwrap()[0]);
        assert!(!scheduler.turn());
        assert!(flag.get());
        let marker = Rc::new(());
        let weak = Rc::downgrade(&marker);
        scheduler.spawn(Box::pin(async move {
            let _marker = marker;
            futures::future::pending::<()>().await;
        }));
        scheduler.close();
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn wake_coalesces_until_polled() {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let out = count.clone();
        let wake = Arc::new(Wake {
            id: 7,
            queued: AtomicBool::new(false),
            notify: Arc::new(move |id| {
                assert_eq!(id, 7);
                out.fetch_add(1, Ordering::Relaxed);
            }),
        });
        Wake::wake_by_ref(&wake);
        Wake::wake_by_ref(&wake);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        wake.queued.store(false, Ordering::Release);
        Wake::wake_by_ref(&wake);
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn idle_worker_completion_notifies_without_polling_timer() {
        let (notify, receive) = std::sync::mpsc::channel();
        let scheduler = Scheduler::new(Arc::new(move |id| {
            let _ = notify.send(id);
        }));
        let (send, result) = futures::channel::oneshot::channel();
        let completed = Rc::new(Cell::new(false));
        let out = completed.clone();
        scheduler.spawn(Box::pin(async move {
            result.await.unwrap();
            out.set(true);
        }));
        scheduler.ready(receive.recv().unwrap());
        assert!(!scheduler.turn());
        assert!(!completed.get());
        std::thread::spawn(move || send.send(()).unwrap())
            .join()
            .unwrap();
        scheduler.ready(receive.recv().unwrap());
        assert!(!scheduler.turn());
        assert!(completed.get());
    }
    #[test]
    fn one_turn_is_bounded_and_future_can_spawn_reentrantly() {
        let ids = Arc::new(Mutex::new(Vec::new()));
        let out = ids.clone();
        let scheduler = Rc::new(Scheduler::new(Arc::new(move |id| {
            out.lock().unwrap().push(id)
        })));
        let count = Rc::new(Cell::new(0));
        for _ in 0..130 {
            let count = count.clone();
            scheduler.spawn(Box::pin(async move {
                count.set(count.get() + 1);
            }));
        }
        for id in ids.lock().unwrap().drain(..) {
            scheduler.ready(id);
        }
        assert!(scheduler.turn());
        assert_eq!(count.get(), 64);
        assert!(scheduler.turn());
        assert!(!scheduler.turn());
        assert_eq!(count.get(), 130);
        let owner = scheduler.clone();
        scheduler.spawn(Box::pin(async move {
            owner.spawn(Box::pin(async {}));
        }));
        for id in ids.lock().unwrap().drain(..) {
            scheduler.ready(id);
        }
        scheduler.turn();
        assert_eq!(ids.lock().unwrap().len(), 1);
    }
}
