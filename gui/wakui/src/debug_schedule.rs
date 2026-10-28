//! Toolkit-independent debugger and PPU-viewer scheduling state.
//! Native window handles remain in the auxiliary-window registry.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// Live PPU cadence while the core runs: images-only ticks, never full drains.
pub const PPU_LIVE_INTERVAL: Duration = Duration::from_millis(250);

/// Deferred open requests, remembered positions, and live debug cadence.
pub struct DebugSchedule {
    debug_open_requested: bool,
    ppu_open_requested: bool,
    debug_window_pos: Option<(i32, i32)>,
    ppu_window_pos: Option<(i32, i32)>,
    last_ppu_live: Option<Instant>,
    /// Pause state at the last completed debug drain, for focus
    /// coalescing. `None` before the first drain.
    last_drain_paused: Option<bool>,
    /// Live-tick interest: set every pass; the tick thread parks when clear.
    live_wanted: Arc<AtomicBool>,
    /// Parked tick thread: set once, read lock-free on every arm.
    tick_thread: std::sync::OnceLock<std::thread::Thread>,
}

impl Default for DebugSchedule {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugSchedule {
    pub fn new() -> Self {
        Self {
            debug_open_requested: false,
            ppu_open_requested: false,
            debug_window_pos: None,
            ppu_window_pos: None,
            last_ppu_live: None,
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

    pub fn request_debug_open(&mut self) {
        self.debug_open_requested = true;
    }

    pub fn take_debug_open_request(&mut self) -> bool {
        std::mem::take(&mut self.debug_open_requested)
    }

    pub fn take_ppu_open_request(&mut self) -> bool {
        std::mem::take(&mut self.ppu_open_requested)
    }

    pub fn request_ppu_open(&mut self) {
        self.ppu_open_requested = true;
    }

    pub fn last_drain_paused(&self) -> Option<bool> {
        self.last_drain_paused
    }

    pub fn set_last_drain_paused(&mut self, paused: bool) {
        self.last_drain_paused = Some(paused);
    }

    pub fn debug_window_pos(&self) -> Option<(i32, i32)> {
        self.debug_window_pos
    }

    pub fn ppu_window_pos(&self) -> Option<(i32, i32)> {
        self.ppu_window_pos
    }

    pub fn remember_debug_window_pos(&mut self, x: i32, y: i32) {
        self.debug_window_pos = Some((x, y));
    }

    pub fn remember_ppu_window_pos(&mut self, x: i32, y: i32) {
        self.ppu_window_pos = Some((x, y));
    }

    /// Throttle gate for the live follow: true at most once per [`PPU_LIVE_INTERVAL`].
    pub fn live_due(&mut self, now: Instant) -> bool {
        if self
            .last_ppu_live
            .is_some_and(|last| now.duration_since(last) < PPU_LIVE_INTERVAL)
        {
            return false;
        }
        self.last_ppu_live = Some(now);
        true
    }

    /// Next scheduled live tick, for native event-loop wait backstops.
    /// `None` before the first tick.
    pub fn next_live_tick(&self) -> Option<Instant> {
        self.last_ppu_live.map(|last| last + PPU_LIVE_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_requests_latch_once() {
        let mut windows = DebugSchedule::new();
        assert!(!windows.take_debug_open_request());
        assert!(!windows.take_ppu_open_request());
        windows.request_debug_open();
        windows.request_ppu_open();
        assert!(windows.take_debug_open_request());
        assert!(windows.take_ppu_open_request());
        assert!(!windows.take_debug_open_request());
        assert!(!windows.take_ppu_open_request());
    }

    #[test]
    fn repeated_open_requests_coalesce_independently_of_pause_truth() {
        let mut windows = DebugSchedule::default();
        assert_eq!(windows.last_drain_paused(), None);
        for paused in [None, Some(true), Some(false)] {
            if let Some(paused) = paused {
                windows.set_last_drain_paused(paused);
            }
            windows.request_debug_open();
            windows.request_debug_open();
            windows.request_ppu_open();
            windows.request_ppu_open();
            assert_eq!(windows.last_drain_paused(), paused);
            assert!(windows.take_debug_open_request());
            assert!(!windows.take_debug_open_request());
            assert!(windows.take_ppu_open_request());
            assert!(!windows.take_ppu_open_request());
            assert_eq!(windows.last_drain_paused(), paused);
        }
        windows.request_debug_open();
        windows.request_ppu_open();
        windows.set_last_drain_paused(true);
        windows.set_last_drain_paused(false);
        assert!(windows.take_debug_open_request());
        assert!(windows.take_ppu_open_request());
        assert_eq!(windows.last_drain_paused(), Some(false));
    }

    #[test]
    fn window_positions_remember() {
        let mut windows = DebugSchedule::new();
        assert_eq!(windows.debug_window_pos(), None);
        windows.remember_debug_window_pos(60, 20);
        windows.remember_ppu_window_pos(1250, 100);
        assert_eq!(windows.debug_window_pos(), Some((60, 20)));
        assert_eq!(windows.ppu_window_pos(), Some((1250, 100)));
    }

    #[test]
    fn live_throttle_fires_at_most_once_per_interval() {
        let mut windows = DebugSchedule::new();
        assert_eq!(PPU_LIVE_INTERVAL, Duration::from_millis(250));
        assert_eq!(windows.next_live_tick(), None);
        let first = Instant::now();
        assert!(windows.live_due(first));
        assert_eq!(windows.next_live_tick(), Some(first + PPU_LIVE_INTERVAL));
        // Immediate re-checks stay quiet until the interval elapses.
        assert!(!windows.live_due(first));
        assert!(!windows.live_due(first + PPU_LIVE_INTERVAL - Duration::from_nanos(1)));
        assert!(windows.live_due(first + PPU_LIVE_INTERVAL));
    }

    #[test]
    fn live_wanted_tracks_follow_interest() {
        let windows = DebugSchedule::new();
        let wanted = windows.live_armed();
        assert!(!wanted.load(Ordering::Acquire));
        windows.set_live_wanted(true);
        assert!(wanted.load(Ordering::Acquire));
        windows.set_live_wanted(false);
        assert!(!wanted.load(Ordering::Acquire));
    }

    #[test]
    fn arming_unparks_registered_thread() {
        let windows = DebugSchedule::new();
        let first = std::thread::current();
        windows.register_tick_thread(first.clone());
        // No parking here: unparking a running thread only sets its
        // token, so arming stays total without a sleeper.
        windows.set_live_wanted(true);
        assert!(windows.live_armed().load(Ordering::Acquire));
        // First registration wins: a second thread never replaces it.
        let second = std::thread::spawn(std::thread::current).join().unwrap();
        assert_ne!(first.id(), second.id());
        windows.register_tick_thread(second);
        assert_eq!(windows.tick_thread.get().unwrap().id(), first.id());
        windows.set_live_wanted(false);
        windows.set_live_wanted(true);
        assert!(windows.live_armed().load(Ordering::Acquire));
    }
}
