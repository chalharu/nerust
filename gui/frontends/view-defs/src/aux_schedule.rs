//! View-side auxiliary scheduling state: deferred open requests,
//! remembered window positions, and live cadence. Generic over the
//! view-side aux kind `K`; native window handles remain in the
//! auxiliary-window registry.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// Live aux cadence while the core runs: images-only ticks, never full drains.
pub const LIVE_INTERVAL: Duration = Duration::from_millis(250);

/// Deferred open requests, remembered positions, and live cadence, keyed by
/// the view-side aux kind.
pub struct AuxSchedule<K: Copy + Eq + Hash> {
    open_requested: HashSet<K>,
    window_pos: HashMap<K, (i32, i32)>,
    last_live: Option<Instant>,
    /// Pause state at the last completed debug drain, for focus
    /// coalescing. `None` before the first drain.
    last_drain_paused: Option<bool>,
    /// Live-tick interest: set every pass; the tick thread parks when clear.
    live_wanted: Arc<AtomicBool>,
    /// Parked tick thread: set once, read lock-free on every arm.
    tick_thread: std::sync::OnceLock<std::thread::Thread>,
}

impl<K: Copy + Eq + Hash> Default for AuxSchedule<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Copy + Eq + Hash> AuxSchedule<K> {
    pub fn new() -> Self {
        Self {
            open_requested: HashSet::new(),
            window_pos: HashMap::new(),
            last_live: None,
            last_drain_paused: None,
            live_wanted: Arc::new(AtomicBool::new(false)),
            tick_thread: std::sync::OnceLock::new(),
        }
    }

    /// Live-tick interest handle for the tick thread (parks when clear).
    pub fn live_armed(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.live_wanted)
    }

    /// Register the parked tick thread for arm wakeups (first wins).
    pub fn register_tick_thread(&self, thread: std::thread::Thread) {
        let _ = self.tick_thread.set(thread);
    }

    /// Record live-tick interest: lock-free arm/unpark, park on clear.
    pub fn set_live_wanted(&self, wanted: bool) {
        let was = self.live_wanted.swap(wanted, Ordering::Release);
        if wanted
            && !was
            && let Some(thread) = self.tick_thread.get()
        {
            thread.unpark();
        }
    }

    pub fn request_open(&mut self, kind: K) {
        self.open_requested.insert(kind);
    }

    pub fn take_open_request(&mut self, kind: K) -> bool {
        self.open_requested.remove(&kind)
    }

    pub fn last_drain_paused(&self) -> Option<bool> {
        self.last_drain_paused
    }

    pub fn set_last_drain_paused(&mut self, paused: bool) {
        self.last_drain_paused = Some(paused);
    }

    pub fn window_pos(&self, kind: K) -> Option<(i32, i32)> {
        self.window_pos.get(&kind).copied()
    }

    pub fn remember_window_pos(&mut self, kind: K, x: i32, y: i32) {
        self.window_pos.insert(kind, (x, y));
    }

    /// Throttle gate for the live follow: true at most once per [`LIVE_INTERVAL`].
    pub fn live_due(&mut self, now: Instant) -> bool {
        if self
            .last_live
            .is_some_and(|last| now.duration_since(last) < LIVE_INTERVAL)
        {
            return false;
        }
        self.last_live = Some(now);
        true
    }

    /// Next scheduled live tick, for native event-loop wait backstops.
    /// `None` before the first tick.
    pub fn next_live_tick(&self) -> Option<Instant> {
        self.last_live.map(|last| last + LIVE_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Kind {
        First,
        Second,
    }

    #[test]
    fn open_requests_latch_once_per_kind() {
        let mut schedule = AuxSchedule::new();
        assert!(!schedule.take_open_request(Kind::First));
        assert!(!schedule.take_open_request(Kind::Second));
        schedule.request_open(Kind::First);
        schedule.request_open(Kind::Second);
        assert!(schedule.take_open_request(Kind::First));
        assert!(schedule.take_open_request(Kind::Second));
        assert!(!schedule.take_open_request(Kind::First));
        assert!(!schedule.take_open_request(Kind::Second));
    }

    #[test]
    fn repeated_open_requests_coalesce_independently_of_pause_truth() {
        let mut schedule = AuxSchedule::<Kind>::default();
        assert_eq!(schedule.last_drain_paused(), None);
        for paused in [None, Some(true), Some(false)] {
            if let Some(paused) = paused {
                schedule.set_last_drain_paused(paused);
            }
            schedule.request_open(Kind::First);
            schedule.request_open(Kind::First);
            schedule.request_open(Kind::Second);
            schedule.request_open(Kind::Second);
            assert_eq!(schedule.last_drain_paused(), paused);
            assert!(schedule.take_open_request(Kind::First));
            assert!(!schedule.take_open_request(Kind::First));
            assert!(schedule.take_open_request(Kind::Second));
            assert!(!schedule.take_open_request(Kind::Second));
            assert_eq!(schedule.last_drain_paused(), paused);
        }
        schedule.request_open(Kind::First);
        schedule.request_open(Kind::Second);
        schedule.set_last_drain_paused(true);
        schedule.set_last_drain_paused(false);
        assert!(schedule.take_open_request(Kind::First));
        assert!(schedule.take_open_request(Kind::Second));
        assert_eq!(schedule.last_drain_paused(), Some(false));
    }

    #[test]
    fn window_positions_remember_per_kind() {
        let mut schedule = AuxSchedule::new();
        assert_eq!(schedule.window_pos(Kind::First), None);
        schedule.remember_window_pos(Kind::First, 60, 20);
        schedule.remember_window_pos(Kind::Second, 1250, 100);
        assert_eq!(schedule.window_pos(Kind::First), Some((60, 20)));
        assert_eq!(schedule.window_pos(Kind::Second), Some((1250, 100)));
    }

    #[test]
    fn live_throttle_fires_at_most_once_per_interval() {
        let mut schedule = AuxSchedule::<Kind>::new();
        assert_eq!(LIVE_INTERVAL, Duration::from_millis(250));
        assert_eq!(schedule.next_live_tick(), None);
        let first = Instant::now();
        assert!(schedule.live_due(first));
        assert_eq!(schedule.next_live_tick(), Some(first + LIVE_INTERVAL));
        // Immediate re-checks stay quiet until the interval elapses.
        assert!(!schedule.live_due(first));
        assert!(!schedule.live_due(first + LIVE_INTERVAL - Duration::from_nanos(1)));
        assert!(schedule.live_due(first + LIVE_INTERVAL));
    }

    #[test]
    fn live_wanted_tracks_follow_interest() {
        let schedule = AuxSchedule::<Kind>::new();
        let wanted = schedule.live_armed();
        assert!(!wanted.load(Ordering::Acquire));
        schedule.set_live_wanted(true);
        assert!(wanted.load(Ordering::Acquire));
        schedule.set_live_wanted(false);
        assert!(!wanted.load(Ordering::Acquire));
    }

    #[test]
    fn arming_unparks_registered_thread() {
        let schedule = AuxSchedule::<Kind>::new();
        let first = std::thread::current();
        schedule.register_tick_thread(first.clone());
        // No parking here: unparking a running thread only sets its
        // token, so arming stays total without a sleeper.
        schedule.set_live_wanted(true);
        assert!(schedule.live_armed().load(Ordering::Acquire));
        // First registration wins: a second thread never replaces it.
        let second = std::thread::spawn(std::thread::current).join().unwrap();
        assert_ne!(first.id(), second.id());
        schedule.register_tick_thread(second);
        assert_eq!(schedule.tick_thread.get().unwrap().id(), first.id());
        schedule.set_live_wanted(false);
        schedule.set_live_wanted(true);
        assert!(schedule.live_armed().load(Ordering::Acquire));
    }
}
