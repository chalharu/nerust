use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct NativeShellState {
    pub needs_redraw: bool,
    last_presented_frame_counter: u64,
    last_title_update: Instant,
}

impl NativeShellState {
    pub const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

    // On Android, use a coarser poll interval to avoid busy-looping the event
    // loop at 1ms which can produce jitter due to OS scheduling. Desktop and
    // other platforms keep the original 1ms value for responsiveness.
    #[cfg(target_os = "android")]
    pub const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(16);

    #[cfg(not(target_os = "android"))]
    pub const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);

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

        /// EmuThread が Timer ループで 60fps レンダリングし frame_count を更新するため、
    /// frame_counter の変化で再描画を検出できる。
    pub fn wants_redraw(&self, current_frame_counter: u64, _loaded: bool, _paused: bool) -> bool {
        self.needs_redraw || current_frame_counter != self.last_presented_frame_counter
    }

    pub fn wants_poll(&self, loaded: bool, paused: bool) -> bool {
        self.needs_redraw || (loaded && !paused)
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
