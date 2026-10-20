//! Re-entrancy guard for the three bridge mutexes.
//! `with_state` must never nest; every profile panics loudly on nesting.

/// Enter a guarded section; nesting panics in all profiles.
pub(crate) fn lock_reentrancy_guard() -> ReentrancyGuard {
    ReentrancyGuard::new()
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
