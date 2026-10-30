//! Native-loop-neutral host attachment and cooperative multiwindow shutdown.
use std::{
    cell::{Cell, RefCell}, collections::BTreeMap, rc::{Rc, Weak}, time::Duration,
};
use futures::{
    FutureExt, channel::oneshot,
    future::{AbortHandle, Abortable, Either, LocalBoxFuture, select, join_all},
};
use crate::{Executor, Subscription};

/// An already-running UI host. Both methods must be UI-local and nonblocking;
/// `attach` registers/presents the session, never starts a nested native loop.
/// The host retains its Session clone until the session is closed/detached.
pub trait UiHost {
    fn executor(&self) -> Rc<dyn Executor>;
    fn attach(&self, session: Session) -> Result<(), String>;
}

/// Attaches on the existing UI thread. This future does not drive or block any
/// native event loop. Construction and attachment happen on its first poll.
pub async fn run_async(
    host: &dyn UiHost,
    make: impl FnOnce(Rc<dyn Executor>) -> Session,
) -> Result<Session, String> {
    let session = make(host.executor());
    if let Err(error) = host.attach(session.clone()) {
        session.shutdown().close_now();
        return Err(error);
    }
    Ok(session)
}

/// Hooks are owned by the registration token, not by the session. Capture
/// native windows weakly. `cancel` stops application work; `drain` waits for it;
/// `close` performs final native teardown. All hooks must return promptly.
#[derive(Clone)]
pub struct WindowHooks {
    pub confirm: Rc<dyn Fn() -> LocalBoxFuture<'static, bool>>,
    pub cancel: Rc<dyn Fn()>,
    pub drain: Rc<dyn Fn() -> LocalBoxFuture<'static, ()>>,
    pub close: Rc<dyn Fn()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownPhase { Running, Confirming, Draining, Closed }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownOutcome { Closed, Cancelled, TimedOut, Busy }

struct WindowSlot {
    hooks: WindowHooks,
    cancelled: Cell<bool>,
    removed: Cell<bool>,
    abort: RefCell<Option<AbortHandle>>,
}
impl WindowSlot {
    fn cancel(&self) {
        if !self.cancelled.replace(true) { (self.hooks.cancel)(); }
    }
    fn wait<T>(&self, future: LocalBoxFuture<'static, T>) -> Abortable<LocalBoxFuture<'static, T>> {
        let (abort, registration) = AbortHandle::new_pair();
        let old = self.abort.replace(Some(abort));
        if let Some(old) = old { old.abort(); }
        if self.removed.get() { self.remove(); }
        Abortable::new(future, registration)
    }
    fn remove(&self) {
        self.removed.set(true);
        let abort = self.abort.borrow_mut().take();
        if let Some(abort) = abort { abort.abort(); }
    }
}

struct ShutdownCore {
    windows: RefCell<BTreeMap<String, Weak<WindowSlot>>>,
    phase: Cell<ShutdownPhase>,
    generation: Cell<u64>,
    cancel: RefCell<Option<oneshot::Sender<()>>>,
}
#[derive(Clone)]
pub struct ShutdownCoordinator { core: Rc<ShutdownCore> }
impl Default for ShutdownCoordinator {
    fn default() -> Self { Self::new() }
}
impl ShutdownCoordinator {
    pub fn new() -> Self {
        Self { core: Rc::new(ShutdownCore {
            windows: RefCell::new(BTreeMap::new()), phase: Cell::new(ShutdownPhase::Running),
            generation: Cell::new(0), cancel: RefCell::new(None),
        }) }
    }
    pub fn phase(&self) -> ShutdownPhase { self.core.phase.get() }
    pub fn window_count(&self) -> usize {
        self.core.windows.borrow().values().filter(|w| w.strong_count() > 0).count()
    }
    /// Duplicate IDs and registration during shutdown are explicit errors.
    pub fn register_window(&self, id: impl Into<String>, hooks: WindowHooks) -> Result<Subscription, String> {
        if self.phase() != ShutdownPhase::Running { return Err("session is shutting down".into()); }
        let id = id.into();
        if self.core.windows.borrow().get(&id).is_some_and(|w| w.strong_count() > 0) {
            return Err(format!("duplicate window: {id}"));
        }
        let hooks = Rc::new(WindowSlot { hooks, cancelled: Cell::new(false), removed: Cell::new(false), abort: RefCell::new(None) });
        self.core.windows.borrow_mut().insert(id.clone(), Rc::downgrade(&hooks));
        let core = Rc::downgrade(&self.core);
        Ok(Subscription::new(move || {
            if let Some(core) = core.upgrade() { core.windows.borrow_mut().remove(&id); }
            hooks.remove();
            drop(hooks);
        }))
    }
    fn windows(&self) -> Vec<Weak<WindowSlot>> { self.core.windows.borrow().values().cloned().collect() }
    /// Cancels an in-progress request without closing windows. Work already
    /// cancelled during Draining cannot be resurrected; applications may retry.
    pub fn cancel_shutdown(&self) -> bool {
        if !matches!(self.phase(), ShutdownPhase::Confirming | ShutdownPhase::Draining) { return false; }
        self.core.generation.set(self.core.generation.get().checked_add(1).expect("shutdown generation exhausted"));
        self.core.phase.set(ShutdownPhase::Running);
        let cancel = self.core.cancel.borrow_mut().take();
        if let Some(cancel) = cancel { let _ = cancel.send(()); }
        true
    }
    /// Idempotent forced teardown. Cancels before closing, outside all borrows.
    pub fn close_now(&self) {
        if self.core.phase.replace(ShutdownPhase::Closed) == ShutdownPhase::Closed { return; }
        let cancel = self.core.cancel.borrow_mut().take();
        if let Some(cancel) = cancel { let _ = cancel.send(()); }
        let windows = std::mem::take(&mut *self.core.windows.borrow_mut());
        for window in windows.into_values() {
            if let Some(window) = window.upgrade() { window.remove(); window.cancel(); (window.hooks.close)(); }
        }
    }
    /// Deadline includes confirmation and drain. On timeout, remaining work is
    /// cancelled and windows forcibly closed. Drain futures run concurrently.
    /// Dropping the request future restores Running (unless already Closed).
    pub async fn request_shutdown(&self, timeout: Duration) -> ShutdownOutcome {
        match self.phase() {
            ShutdownPhase::Closed => return ShutdownOutcome::Closed,
            ShutdownPhase::Running => (),
            _ => return ShutdownOutcome::Busy,
        }
        self.core.phase.set(ShutdownPhase::Confirming);
        self.core.generation.set(self.core.generation.get().checked_add(1).expect("shutdown generation exhausted"));
        let generation = self.core.generation.get();
        let mut guard = RequestGuard { core: Rc::downgrade(&self.core), generation, armed: true };
        let (cancel, canceled) = oneshot::channel();
        let old = self.core.cancel.replace(Some(cancel));
        drop(old);
        let windows = self.windows();
        for window in &windows {
            if let Some(window) = window.upgrade() { window.cancelled.set(false); }
        }
        let work = async {
            if self.core.generation.get() != generation || self.phase() == ShutdownPhase::Closed { return false; }
            for window in &windows {
                let future = window.upgrade().map(|w| w.wait((w.hooks.confirm)()));
                if let Some(future) = future {
                    if future.await == Ok(false) { return false; }
                }
                if self.core.generation.get() != generation || self.phase() == ShutdownPhase::Closed { return false; }
            }
            self.core.phase.set(ShutdownPhase::Draining);
            let mut drains = Vec::new();
            for window in &windows {
                if let Some(window) = window.upgrade() {
                    window.cancel();
                    if self.core.generation.get() != generation || self.phase() == ShutdownPhase::Closed { return false; }
                    drains.push(window.wait((window.hooks.drain)()));
                }
            }
            join_all(drains).await;
            true
        }.boxed_local();
        let deadline = futures_timer::Delay::new(timeout).boxed_local();
        let result = select(select(work, deadline), canceled).await;
        // A callback may cancel/close synchronously even if work is already Ready.
        let outcome = if self.phase() == ShutdownPhase::Closed {
            ShutdownOutcome::Closed
        } else if self.core.generation.get() != generation {
            ShutdownOutcome::Cancelled
        } else {
            match result {
                Either::Left((Either::Left((true, _)), _)) => {
                    // Cancellation was already issued before draining.
                    self.core.phase.set(ShutdownPhase::Closed);
                    let windows = std::mem::take(&mut *self.core.windows.borrow_mut());
                    for window in windows.into_values() {
                        if let Some(window) = window.upgrade() { window.remove(); (window.hooks.close)(); }
                    }
                    ShutdownOutcome::Closed
                }
                Either::Left((Either::Left((false, _)), _)) => {
                    self.core.phase.set(ShutdownPhase::Running);
                    ShutdownOutcome::Cancelled
                }
                Either::Left((Either::Right(_), _)) => {
                    self.close_now();
                    ShutdownOutcome::TimedOut
                }
                Either::Right(_) => ShutdownOutcome::Cancelled,
            }
        };
        if self.core.generation.get() == generation {
            let old = self.core.cancel.borrow_mut().take();
            drop(old);
        }
        guard.armed = false;
        outcome
    }
}
struct RequestGuard { core: Weak<ShutdownCore>, generation: u64, armed: bool }
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Some(core) = self.core.upgrade().filter(|c| c.generation.get() == self.generation) {
                if core.phase.get() != ShutdownPhase::Closed { core.phase.set(ShutdownPhase::Running); }
                let old = core.cancel.borrow_mut().take();
                drop(old);
            }
        }
    }
}

/// A host-owned collection of window registrations. Applications retain the
/// tokens on their view scopes; unmounting removes the corresponding window.
#[derive(Clone, Default)]
pub struct Session { shutdown: ShutdownCoordinator }
impl Session {
    pub fn new() -> Self { Self::default() }
    pub fn shutdown(&self) -> ShutdownCoordinator { self.shutdown.clone() }
    pub fn register_window(&self, id: impl Into<String>, hooks: WindowHooks) -> Result<Subscription, String> {
        self.shutdown.register_window(id, hooks)
    }
    pub fn window_count(&self) -> usize { self.shutdown.window_count() }
}