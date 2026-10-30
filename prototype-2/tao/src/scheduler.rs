//! UI-local futures; only coalesced numeric wake IDs cross threads.
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
use wakui2_runtime::Executor;
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
    pub fn turn(&self) -> bool {
        for _ in 0..64 {
            if self.closed.get() {
                break;
            }
            let Some(id) = self.ready.borrow_mut().pop_front() else {
                break;
            };
            let Some(mut task) = self.tasks.borrow_mut().remove(&id) else {
                continue;
            };
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
        drop(self.tasks.take());
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
    fn worker_wakes_idle_ui_without_timer_and_close_drops_tasks() {
        let (send_id, ids) = std::sync::mpsc::channel();
        let executor = Scheduler::new(Arc::new(move |id| {
            let _ = send_id.send(id);
        }));
        let (send, receive) = futures::channel::oneshot::channel();
        let done = Rc::new(Cell::new(false));
        let out = done.clone();
        executor.spawn(Box::pin(async move {
            receive.await.unwrap();
            out.set(true);
        }));
        assert!(!done.get());
        executor.ready(ids.recv().unwrap());
        assert!(!executor.turn());
        std::thread::spawn(move || send.send(()).unwrap())
            .join()
            .unwrap();
        executor.ready(ids.recv().unwrap());
        executor.turn();
        assert!(done.get());
        let marker = Rc::new(());
        let weak = Rc::downgrade(&marker);
        executor.spawn(Box::pin(async move {
            let _marker = marker;
            futures::future::pending::<()>().await;
        }));
        executor.close();
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn bounded_turn_and_coalesced_wake() {
        let ids = Arc::new(Mutex::new(Vec::new()));
        let out = ids.clone();
        let executor = Scheduler::new(Arc::new(move |id| out.lock().unwrap().push(id)));
        let count = Rc::new(Cell::new(0));
        for _ in 0..130 {
            let out = count.clone();
            executor.spawn(Box::pin(async move {
                out.set(out.get() + 1);
            }));
        }
        for id in ids.lock().unwrap().drain(..) {
            executor.ready(id);
        }
        assert!(executor.turn());
        assert_eq!(count.get(), 64);
        assert!(executor.turn());
        assert!(!executor.turn());
        assert_eq!(count.get(), 130);
        let wake = Arc::new(Wake {
            id: 1000,
            queued: AtomicBool::new(false),
            notify: executor.notify.clone(),
        });
        Wake::wake_by_ref(&wake);
        Wake::wake_by_ref(&wake);
        assert_eq!(*ids.lock().unwrap(), vec![1000]);
    }
}
