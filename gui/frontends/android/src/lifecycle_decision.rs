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

/// Foreground/license resume gate shared by the restore path and the
/// window-ready retry. An error auto-pause owns the paused state until
/// its dialog is dismissed, so a concurrent lifecycle resume stays
/// suppressed; the site's own want arrives via `can_resume` (fresh
/// loads pass unconditional want, the foreground retry passes its
/// lifecycle flag).
pub(crate) fn should_resume_on_foreground(
    lifecycle_auto_paused: bool,
    error_auto_paused: bool,
    can_resume: bool,
) -> bool {
    can_resume && !(lifecycle_auto_paused && error_auto_paused)
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

    #[test]
    fn foreground_resume_needs_want_without_error_conflict() {
        // No want, no resume (manual pause survives; idle stays idle).
        assert!(!should_resume_on_foreground(false, false, false));
        assert!(!should_resume_on_foreground(true, false, false));
        assert!(!should_resume_on_foreground(false, true, false));
        assert!(!should_resume_on_foreground(true, true, false));
        // Fresh-load want resumes over a pure error pause (preserved
        // fresh behavior); the dismiss path still owns that resume.
        assert!(should_resume_on_foreground(false, false, true));
        assert!(should_resume_on_foreground(false, true, true));
        // Lifecycle want resumes alone ...
        assert!(should_resume_on_foreground(true, false, true));
        // ... but an error auto-pause vetoes the concurrent resume.
        assert!(!should_resume_on_foreground(true, true, true));
    }
}
