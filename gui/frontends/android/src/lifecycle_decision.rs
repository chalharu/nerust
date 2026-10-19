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

/// What the foreground resume gate is asked to do. Fresh loads pass
/// unconditional want; the foreground retry passes its lifecycle flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeWant {
    FreshLoad,
    LifecycleRetry,
}

/// Foreground/license resume gate shared by the restore path and the
/// window-ready retry. An error auto-pause owns the paused state until
/// its dialog is dismissed, so a concurrent lifecycle resume stays
/// suppressed. Named fields: positional bools transposed silently.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ForegroundResume {
    pub lifecycle_paused: bool,
    pub error_paused: bool,
    pub want: Option<ResumeWant>,
}

pub(crate) fn should_resume_on_foreground(ctx: ForegroundResume) -> bool {
    ctx.want.is_some() && !(ctx.lifecycle_paused && ctx.error_paused)
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
        use super::{ForegroundResume, ResumeWant};
        let gate = |lifecycle_paused: bool, error_paused: bool, want: Option<ResumeWant>| {
            should_resume_on_foreground(ForegroundResume {
                lifecycle_paused,
                error_paused,
                want,
            })
        };
        // No want, no resume (manual pause survives; idle stays idle).
        assert!(!gate(false, false, None));
        assert!(!gate(true, false, None));
        assert!(!gate(false, true, None));
        assert!(!gate(true, true, None));
        // Fresh-load want resumes over a pure error pause (preserved
        // fresh behavior); the dismiss path still owns that resume.
        assert!(gate(false, false, Some(ResumeWant::FreshLoad)));
        assert!(gate(false, true, Some(ResumeWant::FreshLoad)));
        // Lifecycle want resumes alone ...
        assert!(gate(true, false, Some(ResumeWant::LifecycleRetry)));
        // ... but an error auto-pause vetoes the concurrent resume.
        assert!(!gate(true, true, Some(ResumeWant::LifecycleRetry)));
    }
}
