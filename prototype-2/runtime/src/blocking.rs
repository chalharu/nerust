//! Bounded worker pool, ported from prototype_1/runtime without a crate dependency.
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
};
use futures::{FutureExt, channel::oneshot, future::BoxFuture};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskError {
    Full,
    Shutdown,
    Panic(String),
    Canceled,
}
impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => f.write_str("blocking task queue is full"),
            Self::Shutdown => f.write_str("blocking pool is shut down"),
            Self::Panic(message) => write!(f, "blocking task panicked: {message}"),
            Self::Canceled => f.write_str("blocking task result channel was canceled"),
        }
    }
}
impl std::error::Error for TaskError {}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// A fixed-size pool with bounded, nonblocking submission. Capacity counts
/// waiting jobs, not running jobs. Drop detaches workers and NEVER joins on the
/// UI thread. Call `shutdown` after the UI driver exits to drain and join.
/// Dropping a result skips queued work; running closures cannot be interrupted.
pub struct BlockingPool {
    sender: Option<mpsc::SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
}
impl BlockingPool {
    pub fn new(workers: usize, capacity: usize) -> Self {
        assert!(workers > 0, "a blocking pool needs at least one worker");
        let (sender, receiver) = mpsc::sync_channel::<Job>(capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        let workers = (0..workers).map(|_| {
            let receiver = receiver.clone();
            thread::spawn(move || loop {
                let job = receiver.lock().expect("blocking queue mutex poisoned").recv();
                let Ok(job) = job else { break; };
                job();
            })
        }).collect();
        Self { sender: Some(sender), workers }
    }
    pub fn try_spawn<F, T>(&self, f: F) -> Result<BoxFuture<'static, Result<T, TaskError>>, TaskError>
    where F: FnOnce() -> T + Send + 'static, T: Send + 'static {
        let sender = self.sender.as_ref().ok_or(TaskError::Shutdown)?;
        let (result, receiver) = oneshot::channel();
        let job: Job = Box::new(move || {
            if result.is_canceled() { return; }
            let value = catch_unwind(AssertUnwindSafe(f)).map_err(|panic| {
                let message = if let Some(message) = panic.downcast_ref::<String>() {
                    message.clone()
                } else if let Some(message) = panic.downcast_ref::<&str>() {
                    (*message).to_owned()
                } else { "non-string panic payload".to_owned() };
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
    /// Blocking drain/join, only after the UI loop exits. Idempotent.
    pub fn shutdown(&mut self) {
        drop(self.sender.take());
        for worker in self.workers.drain(..) { let _ = worker.join(); }
    }
}
impl Drop for BlockingPool {
    fn drop(&mut self) { drop(self.sender.take()); }
}