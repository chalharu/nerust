//! Lifecycle pause/resume decisions as pure predicates.
//! Host-compiled so the contract is unit-tested.

/// Lifecycle auto-pause fires only with no pending pause and a pausable session.
/// A manual pause needs no second pause.
pub(crate) fn should_lifecycle_pause(lifecycle_auto_paused: bool, can_pause: bool) -> bool {
    !lifecycle_auto_paused && can_pause
}

/// A live core already retained its state. A user picker response outranks automatic restore.
pub(crate) fn should_fetch_restore(pending: bool, loaded: bool, picker_pending: bool) -> bool {
    pending && !loaded && !picker_pending
}

/// Suspend file outcome for the storage marker.
/// Saved hidden state touches the marker; a miss clears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuspendFileAction {
    Touch,
    Clear,
}

/// Pure suspend decision: the frontend executes the returned marker I/O.
pub(crate) fn suspend_file_action(saved_hidden: bool) -> SuspendFileAction {
    if saved_hidden {
        SuspendFileAction::Touch
    } else {
        SuspendFileAction::Clear
    }
}

/// Unified restore policy: one struct in, fetch and resume out.
/// Replaces the granular predicates at the foreground boundary.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LifecyclePolicy {
    pub restore_hidden_state: bool,
    pub lifecycle_auto_paused: bool,
    pub error_paused: bool,
    pub has_document_reference: bool,
    pub has_library_id: bool,
}

/// Single foreground-restore decision for the fetch lane only.
/// Resume stays separate: retry and after-restore gates differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestoreAction {
    FreshNone,
    FetchDocument,
    FetchLibrary,
    Discard,
}

impl LifecyclePolicy {
    /// Fetch lane: fresh loads fetch nothing; restores prefer documents,
    /// then the library id; with neither recorded the restore drops.
    pub(crate) fn restore_action(&self) -> RestoreAction {
        if !self.restore_hidden_state {
            return RestoreAction::FreshNone;
        }
        match restore_fetch_route(self.has_document_reference, self.has_library_id) {
            RestoreFetch::Document => RestoreAction::FetchDocument,
            RestoreFetch::Library => RestoreAction::FetchLibrary,
            RestoreFetch::None => RestoreAction::Discard,
        }
    }

    /// After-restore resume: fresh loads always resume; lifecycle restores
    /// resume only our own auto-pause; manual pauses survive either way.
    pub(crate) fn should_resume(&self) -> bool {
        (!self.restore_hidden_state || self.lifecycle_auto_paused) && !self.error_paused
    }

    /// Foreground-retry resume: only our own lifecycle auto-pause resumes.
    pub(crate) fn should_resume_retry(&self) -> bool {
        self.lifecycle_auto_paused && !self.error_paused
    }
}

/// Lifecycle-restore fetch route: document URIs and library ids both
/// fetch on the worker; the event thread only dispatches the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestoreFetch {
    Document,
    Library,
    None,
}

/// Restore prefers the last document URI, then the library id.
/// Neither blocks the event thread; both routes publish through the worker.
pub(crate) fn restore_fetch_route(
    has_document_reference: bool,
    has_library_id: bool,
) -> RestoreFetch {
    if has_document_reference {
        RestoreFetch::Document
    } else if has_library_id {
        RestoreFetch::Library
    } else {
        RestoreFetch::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_never_replaces_a_live_core_or_waiting_user_selection() {
        assert!(should_fetch_restore(true, false, false));
        assert!(!should_fetch_restore(true, true, false));
        assert!(!should_fetch_restore(true, false, true));
        assert!(!should_fetch_restore(false, false, false));
    }

    #[test]
    fn lifecycle_pause_needs_running_core() {
        assert!(should_lifecycle_pause(false, true));
        assert!(!should_lifecycle_pause(true, true));
        assert!(!should_lifecycle_pause(false, false));
        assert!(!should_lifecycle_pause(true, false));
    }

    #[test]
    fn restore_routes_document_first_then_library() {
        use super::{RestoreFetch, restore_fetch_route};
        assert_eq!(restore_fetch_route(true, true), RestoreFetch::Document);
        assert_eq!(restore_fetch_route(true, false), RestoreFetch::Document);
        assert_eq!(restore_fetch_route(false, true), RestoreFetch::Library);
        assert_eq!(restore_fetch_route(false, false), RestoreFetch::None);
    }

    #[test]
    fn suspend_marker_follows_saved_hidden_state() {
        use super::{SuspendFileAction, suspend_file_action};
        assert_eq!(suspend_file_action(true), SuspendFileAction::Touch);
        assert_eq!(suspend_file_action(false), SuspendFileAction::Clear);
    }

    #[test]
    fn unified_policy_covers_every_restore_lane() {
        use super::{LifecyclePolicy, RestoreAction};
        let policy = |restore_hidden_state: bool,
                      lifecycle_auto_paused: bool,
                      error_paused: bool,
                      has_document_reference: bool,
                      has_library_id: bool| {
            LifecyclePolicy {
                restore_hidden_state,
                lifecycle_auto_paused,
                error_paused,
                has_document_reference,
                has_library_id,
            }
        };
        let fetch = |p: LifecyclePolicy| p.restore_action();
        let resume = |p: LifecyclePolicy| p.should_resume();
        let retry = |p: LifecyclePolicy| p.should_resume_retry();
        // Fresh load: nothing to fetch, always resumes without error.
        let fresh = policy(false, false, false, false, false);
        assert_eq!(fetch(fresh), RestoreAction::FreshNone);
        assert!(resume(fresh));
        assert!(!retry(fresh));
        // Restore with our auto-pause: fetch first, resume after.
        let own = policy(true, true, false, true, true);
        assert_eq!(fetch(own), RestoreAction::FetchDocument);
        assert!(resume(own));
        assert!(retry(own));
        // Manual pause survives: fetch runs, resume does not.
        let manual = policy(true, false, false, false, true);
        assert_eq!(fetch(manual), RestoreAction::FetchLibrary);
        assert!(!resume(manual));
        assert!(!retry(manual));
        // Nothing recorded: the restore drops.
        let empty = policy(true, false, false, false, false);
        assert_eq!(fetch(empty), RestoreAction::Discard);
        assert!(!resume(empty));
        // Error auto-pause suppresses resume while the fetch still runs.
        let errored = policy(true, true, true, true, false);
        assert_eq!(fetch(errored), RestoreAction::FetchDocument);
        assert!(!resume(errored));
        assert!(!retry(errored));
    }
}
