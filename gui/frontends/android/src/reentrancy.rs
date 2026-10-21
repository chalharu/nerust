//! Re-entrancy guard for the shared bridge vault.
//! `with_state` must never nest; every profile panics loudly on nesting.

/// Enter a guarded section; nesting panics in all profiles.
pub(crate) fn lock_reentrancy_guard() -> ReentrancyGuard {
    ReentrancyGuard::new()
}

/// Keep the reentrancy token alive throughout the operation, not just lock acquisition.
pub(crate) fn with_lock<T, R>(
    mutex: &std::sync::Mutex<T>,
    operation: impl FnOnce(&mut T) -> R,
) -> R {
    let _guard = lock_reentrancy_guard();
    let mut state = mutex.lock().unwrap_or_else(|poison| {
        log::error!("Android bridge mutex poisoned; recovering state");
        poison.into_inner()
    });
    operation(&mut state)
}

pub(crate) struct ReentrancyGuard;

impl ReentrancyGuard {
    fn new() -> Self {
        BRIDGE_REENTRANCY.with(|held| {
            let depth = held.get();
            assert_eq!(
                depth, 0,
                "nested Android bridge with_state: reorder to avoid nesting",
            );
            held.set(depth + 1);
        });
        Self
    }
}

impl Drop for ReentrancyGuard {
    fn drop(&mut self) {
        BRIDGE_REENTRANCY.with(|held| held.set(held.get().saturating_sub(1)));
    }
}

thread_local! {
    static BRIDGE_REENTRANCY: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_operation_panics_before_lock_and_recovers_after_unwind() {
        let mutex = std::sync::Mutex::new(0);
        let result = std::panic::catch_unwind(|| {
            with_lock(&mutex, |_| {
                with_lock(&mutex, |_| {
                    panic!("must reject before entering nested operation")
                });
            })
        });
        let payload = result.unwrap_err();
        let message = payload.downcast_ref::<String>().unwrap();
        assert!(message.contains("nested Android bridge with_state"));
        with_lock(&mutex, |state| *state += 1);
        assert_eq!(
            *mutex.lock().unwrap_or_else(|poison| poison.into_inner()),
            1
        );
    }

    #[test]
    fn different_threads_serialize_without_false_reentrancy() {
        let mutex = std::sync::Mutex::new(0);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..100 {
                        with_lock(&mutex, |state| *state += 1);
                    }
                });
            }
        });
        assert_eq!(*mutex.lock().unwrap(), 400);
    }

    #[test]
    fn sequential_guards_do_not_trip() {
        {
            let _first = lock_reentrancy_guard();
        }
        let _second = lock_reentrancy_guard();
    }

    #[test]
    #[should_panic(expected = "nested Android bridge with_state")]
    fn nested_guard_panics_in_all_profiles() {
        let _outer = lock_reentrancy_guard();
        let _inner = lock_reentrancy_guard();
    }
}
