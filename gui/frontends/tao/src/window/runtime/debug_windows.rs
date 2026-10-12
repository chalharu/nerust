//! Debugger and PPU-viewer window state. `HostState` orchestrates
//! (drains, menu sync, focus policy); this struct owns the aux-window
//! handles, positions, open-request flags, and the live-follow
//! throttle. Splitting state from orchestration keeps the host
//! focused on the main window and session instead of growing into a
//! god object.

use std::time::{Duration, Instant};

use tao::window::WindowId;

/// Live PPU cadence while the core runs: the viewer follows the
/// running picture instead of freezing on the open-time frame.
/// Ticks are images-only empty batches (never kill staged
/// transactions).
pub(crate) const PPU_LIVE_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) struct DebugWindows {
    pub(crate) debug_window: Option<crate::debug_window::DebugWindowHandle>,
    pub(crate) ppu_window: Option<crate::ppu_window::PpuWindowHandle>,
    debug_open_requested: bool,
    pub(crate) ppu_open_requested: bool,
    debug_window_pos: Option<(i32, i32)>,
    ppu_window_pos: Option<(i32, i32)>,
    last_ppu_live: Option<Instant>,
}

impl DebugWindows {
    pub(crate) fn new() -> Self {
        Self {
            debug_window: None,
            ppu_window: None,
            debug_open_requested: false,
            ppu_open_requested: false,
            debug_window_pos: None,
            ppu_window_pos: None,
            last_ppu_live: None,
        }
    }

    pub(crate) fn is_debug_window(&self, window_id: WindowId) -> bool {
        self.debug_window
            .as_ref()
            .is_some_and(|h| h.window.id() == window_id)
    }

    pub(crate) fn is_ppu_window(&self, window_id: WindowId) -> bool {
        self.ppu_window
            .as_ref()
            .is_some_and(|h| h.window.id() == window_id)
    }

    pub(crate) fn request_debug_open(&mut self) {
        self.debug_open_requested = true;
    }

    pub(crate) fn take_debug_open_request(&mut self) -> bool {
        std::mem::take(&mut self.debug_open_requested)
    }

    pub(crate) fn take_ppu_open_request(&mut self) -> bool {
        std::mem::take(&mut self.ppu_open_requested)
    }

    pub(crate) fn debug_window_pos(&self) -> Option<(i32, i32)> {
        self.debug_window_pos
    }

    pub(crate) fn ppu_window_pos(&self) -> Option<(i32, i32)> {
        self.ppu_window_pos
    }

    pub(crate) fn remember_debug_window_pos(&mut self, x: i32, y: i32) {
        self.debug_window_pos = Some((x, y));
    }

    pub(crate) fn remember_ppu_window_pos(&mut self, x: i32, y: i32) {
        self.ppu_window_pos = Some((x, y));
    }

    pub(crate) fn close_all(&mut self) {
        self.debug_window.take();
        self.ppu_window.take();
    }

    /// Throttle gate for the live follow: true at most once per
    /// [`PPU_LIVE_INTERVAL`], recording the stamp. Pure state, no
    /// session involved, so the cadence is unit-testable.
    pub(crate) fn live_due(&mut self, now: Instant) -> bool {
        if self
            .last_ppu_live
            .is_some_and(|last| now.duration_since(last) < PPU_LIVE_INTERVAL)
        {
            return false;
        }
        self.last_ppu_live = Some(now);
        true
    }

    /// Next scheduled live tick, for `ControlFlow::WaitUntil`
    /// backstops. `None` before the first tick.
    pub(crate) fn next_live_tick(&self) -> Option<Instant> {
        self.last_ppu_live.map(|last| last + PPU_LIVE_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_requests_latch_once() {
        let mut windows = DebugWindows::new();
        assert!(!windows.take_debug_open_request());
        assert!(!windows.take_ppu_open_request());
        windows.request_debug_open();
        windows.ppu_open_requested = true;
        assert!(windows.take_debug_open_request());
        assert!(windows.take_ppu_open_request());
        assert!(!windows.take_debug_open_request());
        assert!(!windows.take_ppu_open_request());
    }

    #[test]
    fn window_positions_remember() {
        let mut windows = DebugWindows::new();
        assert_eq!(windows.debug_window_pos(), None);
        windows.remember_debug_window_pos(60, 20);
        windows.remember_ppu_window_pos(1250, 100);
        assert_eq!(windows.debug_window_pos(), Some((60, 20)));
        assert_eq!(windows.ppu_window_pos(), Some((1250, 100)));
    }

    #[test]
    fn live_throttle_fires_at_most_once_per_interval() {
        let mut windows = DebugWindows::new();
        assert_eq!(windows.next_live_tick(), None);
        let first = Instant::now();
        assert!(windows.live_due(first));
        assert!(windows.next_live_tick().is_some());
        // Immediate re-checks stay quiet until the interval elapses.
        assert!(!windows.live_due(first));
        assert!(!windows.live_due(first + PPU_LIVE_INTERVAL - Duration::from_nanos(1)));
        assert!(windows.live_due(first + PPU_LIVE_INTERVAL));
    }
}
