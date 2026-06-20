use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct NativeShellState {
    pub needs_redraw: bool,
    last_presented_frame_counter: u64,
    last_title_update: Instant,
}

impl NativeShellState {
    pub const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);
    /// Target frame interval (~60fps). Used as a hint for WaitUntil scheduling.
    pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);

    pub fn new() -> Self {
        Self {
            needs_redraw: true,
            last_presented_frame_counter: 0,
            last_title_update: Instant::now(),
        }
    }

    pub fn on_frame_presented(&mut self, frame_counter: u64) {
        self.last_presented_frame_counter = frame_counter;
        self.needs_redraw = false;
    }

    /// Returns true if a redraw should be requested.
    pub fn wants_redraw(&self, current_frame_counter: u64, _loaded: bool, _paused: bool) -> bool {
        self.needs_redraw || current_frame_counter != self.last_presented_frame_counter
    }

    /// Returns true when the event loop should stay active (emulation running).
    /// `frame_interval` returns the expected interval between frames (~16ms).
    pub fn wants_active_loop(&self, loaded: bool, paused: bool) -> bool {
        loaded && !paused
    }

    /// Returns a `WaitUntil` deadline for the next expected frame.
    /// Falls back to `now + FRAME_INTERVAL` when running.
    pub fn next_frame_deadline(&self, now: Instant) -> Instant {
        now + Self::FRAME_INTERVAL
    }

    pub fn should_refresh_title(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last_title_update) >= Self::TITLE_UPDATE_INTERVAL {
            self.last_title_update = now;
            true
        } else {
            false
        }
    }
}

impl Default for NativeShellState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::NativeShellState;
    use std::time::Instant;

    #[test]
    fn native_shell_state_tracks_frame_presentation() {
        let mut shell = NativeShellState::new();
        assert!(shell.wants_redraw(0, false, false));
        shell.on_frame_presented(1);
        assert!(!shell.wants_redraw(1, false, false));
        assert!(shell.wants_redraw(2, false, false));
    }

    #[test]
    fn native_shell_state_refreshes_title_after_interval() {
        let mut shell = NativeShellState::new();
        let now = Instant::now();
        assert!(!shell.should_refresh_title(now));
        assert!(shell.should_refresh_title(now + NativeShellState::TITLE_UPDATE_INTERVAL));
    }
}
