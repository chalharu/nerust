//! Pure main-window lifecycle and fullscreen acknowledgement gates.

use std::sync::Arc;

#[derive(Debug, Clone)]
enum State {
    Live,
    Closed,
    Failed(Arc<str>),
}

#[derive(Debug, Clone)]
pub struct MainState {
    state: State,
    begun: bool,
}

impl Default for MainState {
    fn default() -> Self {
        Self::new()
    }
}

impl MainState {
    pub fn new() -> Self {
        Self {
            state: State::Live,
            begun: false,
        }
    }
    pub fn fail(&mut self, detail: String) {
        self.state = State::Failed(Arc::from(detail));
    }
    pub fn fatal_message(&self) -> Option<&str> {
        match &self.state {
            State::Failed(detail) => Some(detail),
            _ => None,
        }
    }
    pub fn is_usable(&self) -> bool {
        matches!(self.state, State::Live)
    }
    pub fn begin_close(&mut self) -> bool {
        if self.begun {
            return false;
        }
        self.begun = true;
        if matches!(self.state, State::Live) {
            self.state = State::Closed;
        }
        true
    }
    pub fn exited(&self) -> bool {
        self.begun
    }
}

#[derive(Default)]
pub(super) struct FullscreenSync {
    pending: Option<bool>,
}

impl FullscreenSync {
    pub fn pending(&self) -> Option<bool> {
        self.pending
    }
    pub fn request(&mut self, native: bool, target: bool) -> Option<bool> {
        self.pending = (native != target).then_some(target);
        self.pending
    }
    pub fn observe(&mut self, native: bool, persisted: bool) -> bool {
        if self.pending.is_some_and(|target| target != native) {
            return false;
        }
        self.pending = None;
        native != persisted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveness_truth_is_single() {
        assert!(MainState::new().is_usable());
        let mut closed = MainState::new();
        assert!(closed.begin_close());
        assert!(!closed.is_usable());
        let mut failed = MainState::new();
        failed.fail("no display".into());
        assert!(!failed.is_usable());
        assert_eq!(failed.fatal_message(), Some("no display"));
    }
    #[test]
    fn close_is_idempotent_and_retires_liveness() {
        let mut guard = MainState::new();
        assert!(guard.is_usable());
        assert!(guard.begin_close());
        assert!(!guard.is_usable());
        assert!(!guard.begin_close());
        assert!(guard.exited());
    }
    #[test]
    fn fail_retires_liveness_but_keeps_detail() {
        let mut guard = MainState::new();
        guard.fail("no display".into());
        assert_eq!(guard.fatal_message(), Some("no display"));
        assert!(!guard.is_usable());
        assert!(guard.begin_close());
        assert!(!guard.is_usable());
        assert_eq!(guard.fatal_message(), Some("no display"));
    }
    #[test]
    fn fail_after_close_keeps_terminal_failed() {
        let mut guard = MainState::new();
        assert!(guard.begin_close());
        guard.fail("late display loss".into());
        assert_eq!(guard.fatal_message(), Some("late display loss"));
        assert!(!guard.is_usable());
        assert!(!guard.begin_close());
        assert!(guard.exited());
    }
    #[test]
    fn second_fail_overwrites_detail() {
        let mut guard = MainState::new();
        guard.fail("first".into());
        guard.fail("second".into());
        assert_eq!(guard.fatal_message(), Some("second"));
        assert!(!guard.is_usable());
    }
    #[test]
    fn fullscreen_sync_ignores_transient_state_while_transition_is_pending() {
        let mut gate = FullscreenSync::default();
        assert_eq!(gate.request(false, true), Some(true));
        assert!(!gate.observe(false, true));
        assert_eq!(gate.pending(), Some(true));
    }
    #[test]
    fn fullscreen_sync_clears_pending_transition_once_window_matches_target() {
        let mut gate = FullscreenSync::default();
        gate.request(false, true);
        assert!(!gate.observe(true, true));
        assert_eq!(gate.pending(), None);
    }
    #[test]
    fn fullscreen_sync_persists_native_changes_once_no_transition_is_pending() {
        assert!(FullscreenSync::default().observe(false, true));
    }
    #[test]
    fn fullscreen_replacement_target_and_cancel_use_last_request() {
        let mut gate = FullscreenSync::default();
        gate.request(false, true);
        assert_eq!(gate.request(true, false), Some(false));
        assert!(!gate.observe(true, false));
        assert!(!gate.observe(false, false));
        gate.request(false, true);
        assert_eq!(gate.request(false, false), None);
        assert!(gate.observe(true, false));
    }
}
