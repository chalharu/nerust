//! Window liveness plus close idempotence in one owned guard.
//! `HostState` delegates instead of threading three flags through
//! every close-adjacent method.

/// Fatal window-creation failure, tracked apart from liveness. The
/// detail explains *why* the loop must exit; liveness says whether a
/// native handle may still be touched.
#[derive(Debug, Default)]
pub(crate) struct FatalError {
    detail: Option<String>,
}

impl FatalError {
    fn set(&mut self, detail: String) {
        self.detail = Some(detail);
    }

    /// Creation failure, if any.
    pub(crate) fn message(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    fn is_set(&self) -> bool {
        self.detail.is_some()
    }
}

/// Fatal creation failure, close idempotence, and native liveness.
/// Constructed live; `fail` and `begin_close` both retire liveness.
/// Query liveness only through [`CloseGuard::is_usable`].
#[derive(Debug)]
pub(crate) struct CloseGuard {
    fatal: FatalError,
    exited: bool,
    live: bool,
}

impl CloseGuard {
    pub(crate) fn new() -> Self {
        Self {
            fatal: FatalError::default(),
            exited: false,
            live: true,
        }
    }

    /// Record a window-creation failure and retire liveness: no dead
    /// handle may be touched afterwards. The loop exits gracefully.
    pub(crate) fn fail(&mut self, detail: String) {
        self.fatal.set(detail);
        self.live = false;
    }

    /// Creation failure, if any.
    pub(crate) fn fatal_message(&self) -> Option<&str> {
        self.fatal.message()
    }

    /// Native window still usable for redraws and surfaces: live, not
    /// yet closed, and never failed. Single liveness predicate for
    /// every close-adjacent path.
    pub(crate) fn is_usable(&self) -> bool {
        self.live && !self.exited && !self.fatal.is_set()
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
        assert!(guard.is_usable());
        assert!(guard.begin_close());
        assert!(!guard.is_usable());
        assert!(!guard.begin_close());
        assert!(guard.exited());
    }

    #[test]
    fn fail_retires_liveness_but_keeps_detail() {
        let mut guard = CloseGuard::new();
        guard.fail("no display".to_string());
        assert_eq!(guard.fatal_message(), Some("no display"));
        assert!(!guard.is_usable());
        // A later close still runs exactly once; the detail survives.
        assert!(guard.begin_close());
        assert!(!guard.is_usable());
        assert_eq!(guard.fatal_message(), Some("no display"));
    }
}
