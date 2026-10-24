//! Main-window liveness plus close idempotence in one owned state.

use std::sync::Arc;

#[derive(Debug, Clone)]
enum State {
    Live,
    Closed,
    Failed(Arc<str>),
}

/// Single close bit: liveness plus the exactly-once begin flag.
/// Query liveness only through [`MainState::is_usable`].
#[derive(Debug, Clone)]
pub(crate) struct MainState {
    state: State,
    begun: bool,
}

impl MainState {
    pub(crate) fn new() -> Self {
        Self {
            state: State::Live,
            begun: false,
        }
    }

    /// Record a window-creation failure and retire liveness.
    /// `Failed` is terminal: the state stays `Failed`, never `Closed`.
    pub(crate) fn fail(&mut self, detail: String) {
        self.state = State::Failed(Arc::from(detail));
    }

    /// Creation failure, if any.
    pub(crate) fn fatal_message(&self) -> Option<&str> {
        match &self.state {
            State::Failed(detail) => Some(detail),
            _ => None,
        }
    }

    /// Native window still usable for redraws and surfaces.
    pub(crate) fn is_usable(&self) -> bool {
        matches!(self.state, State::Live)
    }

    /// Idempotent close entry: true exactly once, retiring `Live` to `Closed`.
    pub(crate) fn begin_close(&mut self) -> bool {
        if self.begun {
            return false;
        }
        self.begun = true;
        if matches!(self.state, State::Live) {
            self.state = State::Closed;
        }
        true
    }

    /// Close teardown already ran (test read of the guard).
    #[cfg(test)]
    pub(crate) fn exited(&self) -> bool {
        self.begun
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
        failed.fail("no display".to_string());
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
        guard.fail("no display".to_string());
        assert_eq!(guard.fatal_message(), Some("no display"));
        assert!(!guard.is_usable());
        // A later close still runs exactly once; Failed stays Failed.
        assert!(guard.begin_close());
        assert!(!guard.is_usable());
        assert_eq!(guard.fatal_message(), Some("no display"));
    }

    #[test]
    fn fail_after_close_keeps_terminal_failed() {
        let mut guard = MainState::new();
        assert!(guard.begin_close());
        guard.fail("late display loss".to_string());
        assert_eq!(guard.fatal_message(), Some("late display loss"));
        assert!(!guard.is_usable());
        assert!(!guard.begin_close());
        assert!(guard.exited());
    }

    #[test]
    fn second_fail_overwrites_detail() {
        let mut guard = MainState::new();
        guard.fail("first".to_string());
        guard.fail("second".to_string());
        assert_eq!(guard.fatal_message(), Some("second"));
        assert!(!guard.is_usable());
    }
}
