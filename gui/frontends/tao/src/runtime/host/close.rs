//! Window liveness plus close idempotence in one owned guard.
//! `HostState` delegates instead of threading three flags through
//! every close-adjacent method.

/// Fatal creation failure, close idempotence, and native liveness.
/// Constructed live; `begin_close` retires the window exactly once.
#[derive(Debug)]
pub(crate) struct CloseGuard {
    fatal: Option<String>,
    exited: bool,
    live: bool,
}

impl CloseGuard {
    pub(crate) fn new() -> Self {
        Self {
            fatal: None,
            exited: false,
            live: true,
        }
    }

    /// Record a window-creation failure; the loop exits gracefully.
    pub(crate) fn fail(&mut self, detail: String) {
        self.fatal = Some(detail);
    }

    /// Creation failure, if any.
    pub(crate) fn fatal_message(&self) -> Option<&str> {
        self.fatal.as_deref()
    }

    /// Native window still usable for redraws and surfaces.
    pub(crate) fn is_live(&self) -> bool {
        self.live
    }

    /// Idempotent close entry: true exactly once, retiring liveness.
    pub(crate) fn begin_close(&mut self) -> bool {
        if self.exited {
            return false;
        }
        self.exited = true;
        self.live = false;
        true
    }

    /// Close teardown already ran (test read of the guard).
    #[cfg(test)]
    pub(crate) fn exited(&self) -> bool {
        self.exited
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_is_idempotent_and_retires_liveness() {
        let mut guard = CloseGuard::new();
        assert!(guard.is_live());
        assert!(guard.begin_close());
        assert!(!guard.is_live());
        assert!(!guard.begin_close());
        assert!(guard.exited());
    }

    #[test]
    fn fatal_message_survives_close() {
        let mut guard = CloseGuard::new();
        guard.fail("no display".to_string());
        assert_eq!(guard.fatal_message(), Some("no display"));
        assert!(guard.begin_close());
        assert_eq!(guard.fatal_message(), Some("no display"));
    }
}
