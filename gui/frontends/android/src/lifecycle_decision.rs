//! Lifecycle pause/resume decisions as pure predicates. The
//! JNI-bound call sites stay target-gated in `android`; this module
//! compiles on host so the contract is unit-tested in CI.

/// Lifecycle auto-pause fires only when no pause is pending and the
/// session can pause (loaded and running). Pausing with no core fails
/// into `CommandFailed` noise and an unasked modal; a manual pause
/// needs no second pause.
pub(crate) fn should_lifecycle_pause(lifecycle_auto_paused: bool, can_pause: bool) -> bool {
    !lifecycle_auto_paused && can_pause
}

/// Resume after a ROM load: fresh loads always resume; lifecycle
/// restores resume only our own auto-pause. A manual pause predates
/// the suspend and must survive the round-trip.
pub(crate) fn should_resume_after_restore(
    restore_hidden_state: bool,
    lifecycle_auto_paused: bool,
) -> bool {
    !restore_hidden_state || lifecycle_auto_paused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_pause_needs_running_core() {
        assert!(should_lifecycle_pause(false, true));
        assert!(!should_lifecycle_pause(true, true));
        assert!(!should_lifecycle_pause(false, false));
        assert!(!should_lifecycle_pause(true, false));
    }

    #[test]
    fn restore_resumes_only_own_auto_pause() {
        assert!(should_resume_after_restore(false, false));
        assert!(should_resume_after_restore(false, true));
        assert!(should_resume_after_restore(true, true));
        assert!(!should_resume_after_restore(true, false));
    }
}
