//! Small synchronization helpers shared by view definitions.
//!
//! Poison recovery with a warning: native adapters must never wedge when
//! a background flight dies holding a lock.

use std::sync::{Mutex, MutexGuard};

/// Recover the inner value after poisoning; it may be stale until the next drain.
pub fn lock_recovered<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| {
        log::warn!("mutex recovered after poisoning; state may be stale");
        error.into_inner()
    })
}
