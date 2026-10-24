//! Picker ownership and lane bounds as JNI-free logic.
//! The android picker mirrors this decision table; host tests pin it.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Slot ownership without JNI types: generation plus handle presence.
/// Test-only table mirrored by the android picker; host tests pin interleavings.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct SlotState {
    pub in_flight: Option<u64>,
    pub signal: bool,
    pub stream: bool,
}

#[cfg(test)]
impl SlotState {
    /// Publish a handle: true only for the owning generation.
    pub(crate) fn publish(&mut self, seq: u64) -> bool {
        self.in_flight == Some(seq)
    }

    /// Take both handles for reset/cancel; exactly one side wins.
    pub(crate) fn take_all(&mut self) -> (bool, bool) {
        (
            std::mem::replace(&mut self.signal, false),
            std::mem::replace(&mut self.stream, false),
        )
    }

    /// Take handles only when the caller still owns the slot.
    /// A stale completion never steals the current job's handles.
    pub(crate) fn release_if_ours(&mut self, seq: u64) -> (bool, bool) {
        if self.in_flight == Some(seq) {
            self.take_all()
        } else {
            (false, false)
        }
    }

    /// Clear the gate; handles leave through `take_all` first.
    pub(crate) fn clear(&mut self) {
        self.in_flight = None;
    }
}

/// Latest-only lane: one pending slot, newer pushes drop the stale one.
/// Total retention is pending (<=1) plus the active job, never a backlog.
pub(crate) struct LatestLane<T> {
    slot: std::sync::Mutex<LaneSlot<T>>,
    waker: std::sync::Condvar,
}

struct LaneSlot<T> {
    pending: Option<T>,
    closed: bool,
    dropped: u64,
}

impl<T> Default for LatestLane<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LatestLane<T> {
    pub(crate) const fn new() -> Self {
        Self {
            slot: std::sync::Mutex::new(LaneSlot {
                pending: None,
                closed: false,
                dropped: 0,
            }),
            waker: std::sync::Condvar::new(),
        }
    }

    /// Push, replacing a still-pending job; false only when closed.
    pub(crate) fn send(&self, job: T) -> bool {
        let mut slot = lock_recovered(&self.slot);
        if slot.closed {
            return false;
        }
        if slot.pending.replace(job).is_some() {
            slot.dropped += 1;
        }
        self.waker.notify_one();
        true
    }

    /// Blocking take for the single worker; `None` means closed and drained.
    pub(crate) fn take(&self) -> Option<T> {
        let mut slot = lock_recovered(&self.slot);
        loop {
            if let Some(job) = slot.pending.take() {
                return Some(job);
            }
            if slot.closed {
                return None;
            }
            slot = self
                .waker
                .wait(slot)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Close: pending drains, later sends fail so callers can report.
    pub(crate) fn close(&self) {
        lock_recovered(&self.slot).closed = true;
        self.waker.notify_all();
    }

    pub(crate) fn pending(&self) -> usize {
        usize::from(lock_recovered(&self.slot).pending.is_some())
    }

    pub(crate) fn dropped(&self) -> u64 {
        lock_recovered(&self.slot).dropped
    }
}

fn lock_recovered<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Bumper pressure gauge: overflow never drops or blocks; the count exposes stalls.
pub(crate) struct BumperBudget {
    pending: AtomicUsize,
    cap: usize,
}

impl BumperBudget {
    pub(crate) const fn new(cap: usize) -> Self {
        Self {
            pending: AtomicUsize::new(0),
            cap,
        }
    }

    /// Reserve a queue slot; true while within the cap.
    pub(crate) fn pushed(&self) -> bool {
        self.pending.fetch_add(1, Ordering::AcqRel) < self.cap
    }

    /// Release a queue slot after the provider call finishes.
    pub(crate) fn popped(&self) {
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }

    pub(crate) fn pending(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reset between check and registration: the stale job loses the slot.
    #[test]
    fn stale_publish_refused_after_reset() {
        let mut slot = SlotState {
            in_flight: Some(7),
            signal: false,
            stream: false,
        };
        slot.clear();
        assert!(!slot.publish(7));
        slot.in_flight = Some(8);
        assert!(slot.publish(8));
    }

    /// Stale release never steals the current job's fresh handles.
    #[test]
    fn stale_release_keeps_current_handles() {
        let mut slot = SlotState {
            in_flight: Some(9),
            signal: true,
            stream: true,
        };
        assert_eq!(slot.release_if_ours(8), (false, false));
        assert!(slot.signal && slot.stream);
        assert_eq!(slot.release_if_ours(9), (true, true));
        assert!(!slot.signal && !slot.stream);
    }

    /// Reset takes exactly once: a second take finds nothing.
    #[test]
    fn reset_take_is_single_owner() {
        let mut slot = SlotState {
            in_flight: Some(3),
            signal: true,
            stream: true,
        };
        slot.clear();
        assert_eq!(slot.take_all(), (true, true));
        assert_eq!(slot.take_all(), (false, false));
    }

    /// Latest wins: a queued stale job drops instead of retaining.
    #[test]
    fn latest_push_drops_stale_pending() {
        let lane = LatestLane::new();
        assert!(lane.send("old"));
        assert!(lane.send("new"));
        assert_eq!(lane.pending(), 1);
        assert_eq!(lane.dropped(), 1);
        assert_eq!(lane.take(), Some("new"));
        assert_eq!(lane.pending(), 0);
    }

    /// Closed lane drains then refuses, so callers can report.
    #[test]
    fn closed_lane_drains_then_refuses() {
        let lane = LatestLane::new();
        assert!(lane.send("job"));
        lane.close();
        assert_eq!(lane.take(), Some("job"));
        assert!(lane.take().is_none());
        assert!(!lane.send("late"));
    }

    /// Budget exposes pressure without dropping or blocking.
    #[test]
    fn budget_flags_overflow_keeps_count() {
        let budget = BumperBudget::new(2);
        assert!(budget.pushed());
        assert!(budget.pushed());
        assert!(!budget.pushed());
        assert_eq!(budget.pending(), 3);
        budget.popped();
        assert_eq!(budget.pending(), 2);
    }
}
