//! Debugger and PPU-viewer window state: aux handles, positions,
//! open-request flags, and the live-follow throttle. `HostState`
//! orchestrates; this struct owns.

use std::time::{Duration, Instant};

use crate::aux::focus::WindowHandler;
use tao::window::WindowId;

/// Live PPU cadence while the core runs: the viewer follows the
/// running picture instead of freezing on the open-time frame.
/// Ticks are images-only empty batches (never kill staged
/// transactions).
pub(super) const PPU_LIVE_INTERVAL: Duration = Duration::from_millis(250);

pub(super) struct DebugWindows {
    debug_window: Option<crate::aux::debug::DebugWindowHandle>,
    ppu_window: Option<crate::aux::ppu::PpuWindowHandle>,
    debug_open_requested: bool,
    ppu_open_requested: bool,
    debug_window_pos: Option<(i32, i32)>,
    ppu_window_pos: Option<(i32, i32)>,
    last_ppu_live: Option<Instant>,
    /// Pause state at the last completed debug drain, for focus
    /// coalescing. `None` before the first drain.
    last_drain_paused: Option<bool>,
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
            last_drain_paused: None,
        }
    }

    /// Share the debugger window for bridge reads, if open.
    pub(super) fn debug_window(&self) -> Option<&crate::aux::debug::DebugWindowHandle> {
        self.debug_window.as_ref()
    }

    /// Borrow the debugger window for event routing, if open.
    pub(super) fn debug_window_mut(&mut self) -> Option<&mut crate::aux::debug::DebugWindowHandle> {
        self.debug_window.as_mut()
    }

    /// Share the PPU viewer for bridge reads, if open.
    pub(super) fn ppu_window(&self) -> Option<&crate::aux::ppu::PpuWindowHandle> {
        self.ppu_window.as_ref()
    }

    /// Borrow the PPU viewer for event routing, if open.
    pub(super) fn ppu_window_mut(&mut self) -> Option<&mut crate::aux::ppu::PpuWindowHandle> {
        self.ppu_window.as_mut()
    }

    /// Seat a newly created debugger window (single writer: the open path).
    pub(super) fn set_debug_window(&mut self, handle: crate::aux::debug::DebugWindowHandle) {
        self.debug_window = Some(handle);
    }

    /// Seat a newly created PPU viewer (single writer: the open path).
    pub(super) fn set_ppu_window(&mut self, handle: crate::aux::ppu::PpuWindowHandle) {
        self.ppu_window = Some(handle);
    }

    /// Whether the debugger window is currently open.
    pub(super) fn is_debug_open(&self) -> bool {
        self.debug_window.is_some()
    }

    /// Whether the PPU viewer is currently open.
    pub(super) fn is_ppu_open(&self) -> bool {
        self.ppu_window.is_some()
    }

    pub(crate) fn is_debug_window(&self, window_id: WindowId) -> bool {
        self.debug_window
            .as_ref()
            .is_some_and(|h| h.tao_window_id() == window_id)
    }

    pub(crate) fn is_ppu_window(&self, window_id: WindowId) -> bool {
        self.ppu_window
            .as_ref()
            .is_some_and(|h| h.tao_window_id() == window_id)
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

    pub(super) fn request_ppu_open(&mut self) {
        self.ppu_open_requested = true;
    }

    pub(super) fn last_drain_paused(&self) -> Option<bool> {
        self.last_drain_paused
    }

    pub(super) fn set_last_drain_paused(&mut self, paused: bool) {
        self.last_drain_paused = Some(paused);
    }

    /// Drain one batch from the debugger window: queued requests plus
    /// the toolbar PPU-open flag. `None` with no debugger open.
    pub(super) fn take_debug_batch(
        &mut self,
    ) -> Option<(bool, Vec<nerust_gui_shell::debug::DebugAction>)> {
        let handle = self.debug_window.as_ref()?;
        let open = handle
            .bridge
            .open_ppu
            .swap(false, std::sync::atomic::Ordering::SeqCst);
        Some((open, handle.take_requests()))
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

    /// Drop a dead handle (external destroy without CloseRequested):
    /// a stale handle must not linger and block reopening.
    pub(super) fn drop_debug_window(&mut self) {
        self.debug_window.take();
    }

    /// Same guard for the PPU viewer.
    pub(super) fn drop_ppu_window(&mut self) {
        self.ppu_window.take();
    }

    /// Close-request flag raised by the debugger window itself.
    pub(super) fn debug_close_requested(&self) -> bool {
        self.debug_window
            .as_ref()
            .is_some_and(|h| h.close_requested())
    }

    /// Same flag for the PPU viewer.
    pub(super) fn ppu_close_requested(&self) -> bool {
        self.ppu_window
            .as_ref()
            .is_some_and(|h| h.close_requested())
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
        windows.request_ppu_open();
        assert!(windows.take_debug_open_request());
        assert!(windows.take_ppu_open_request());
        assert!(!windows.take_debug_open_request());
        assert!(!windows.take_ppu_open_request());
    }

    #[test]
    fn close_all_clears_without_windows() {
        // Headless: no windows open, so this only proves the call
        // is total and keeps the rest of the state intact.
        let mut windows = DebugWindows::new();
        windows.remember_debug_window_pos(1, 2);
        windows.close_all();
        assert_eq!(windows.debug_window_pos(), Some((1, 2)));
        assert!(!windows.take_debug_open_request());
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

    #[test]
    fn drop_and_close_flags_total_on_empty() {
        // Headless: no windows open; drops are no-ops and no close
        // is ever reported.
        let mut windows = DebugWindows::new();
        windows.drop_debug_window();
        windows.drop_ppu_window();
        assert!(!windows.debug_close_requested());
        assert!(!windows.ppu_close_requested());
    }
}
