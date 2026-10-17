//! Deferred auto-pause protocol as a behavior-carrying type.
//! Invariants: a pause lands only via [`HostState::commit_pending_auto_pause`]
//! after arming (`leave_main`/`leave_aux`) without an intervening cancel
//! (`cancel_pending`/`arrive_*`); arrival reports a resume at most once
//! per armed pause (the flag is consumed). Explicit user pauses never
//! set the flag.

use super::HostState;
use nerust_gui_shell::session::access::PauseControl;

impl HostState {
    pub(crate) fn resume_session(&mut self) {
        PauseControl::resume(self);
    }

    pub(crate) fn pause_session(&mut self) {
        PauseControl::pause(self);
    }

    /// Execute a still-armed auto-pause at the end of the event burst.
    /// Post-pause chrome sync keeps the menu/title truthful (the blur
    /// path below only synced pre-pause truth).
    pub(crate) fn commit_pending_auto_pause(&mut self) {
        if !self.focus.take_armed() {
            return;
        }
        if self.session.can_pause() {
            self.pause_session();
            self.focus.mark_auto_paused();
            self.sync_debug_chrome();
        }
    }
}

/// Main-window focus truth plus the deferred auto-pause protocol.
pub(crate) struct FocusState {
    /// Main window holds focus.
    main_active: bool,
    /// Session paused by the protocol, never by the user directly.
    auto_paused: bool,
    /// Blur armed a pause; committed at burst end unless cancelled.
    pending: bool,
}

impl FocusState {
    pub(super) fn new() -> Self {
        Self {
            main_active: true,
            auto_paused: false,
            pending: false,
        }
    }

    pub(crate) fn main_active(&self) -> bool {
        self.main_active
    }

    /// Main-window arrival: activate, cancel any pending pause, and
    /// report whether a system-paused session must resume.
    pub(crate) fn arrive_main(&mut self) -> bool {
        self.main_active = true;
        self.arrive()
    }

    /// Main-window loss: deactivate and arm the deferred pause.
    pub(crate) fn leave_main(&mut self) {
        self.main_active = false;
        self.pending = true;
    }

    /// Aux-window arrival (same-app move, not away): cancel any
    /// pending pause and report whether to resume.
    pub(crate) fn arrive_aux(&mut self) -> bool {
        self.arrive()
    }

    /// Aux-window loss: arm the deferred pause.
    pub(crate) fn leave_aux(&mut self) {
        self.pending = true;
    }

    /// Cancel a pending pause without further effect (e.g. settings
    /// arrival, which never resumes).
    pub(crate) fn cancel_pending(&mut self) {
        self.pending = false;
    }

    /// Burst-end commit point: take the armed flag. False when
    /// cancelled meanwhile or never armed.
    fn take_armed(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// Record a protocol-driven pause (user pauses never set this).
    fn mark_auto_paused(&mut self) {
        self.auto_paused = true;
    }

    /// Shared arrival core: cancel pending, consume the resume flag.
    fn arrive(&mut self) -> bool {
        self.pending = false;
        std::mem::replace(&mut self.auto_paused, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unarmed_commit_reports_false() {
        let mut focus = FocusState::new();
        assert!(!focus.take_armed());
        assert!(!focus.arrive_main());
        assert!(!focus.arrive_aux());
    }

    #[test]
    fn arm_commit_cycle_arms_once() {
        let mut focus = FocusState::new();
        focus.leave_aux();
        assert!(focus.take_armed());
        // Consumed: a second commit without re-arm is quiet.
        assert!(!focus.take_armed());
    }

    #[test]
    fn cancel_disarms_before_commit() {
        let mut focus = FocusState::new();
        focus.leave_aux();
        focus.cancel_pending();
        assert!(!focus.take_armed());
    }

    #[test]
    fn arrive_reports_resume_exactly_once() {
        let mut focus = FocusState::new();
        focus.leave_aux();
        assert!(focus.take_armed());
        focus.mark_auto_paused();
        assert!(focus.arrive_main());
        // Flag consumed: no double resume.
        assert!(!focus.arrive_main());
        assert!(!focus.arrive_aux());
    }

    #[test]
    fn main_leave_deactivates_and_arms() {
        let mut focus = FocusState::new();
        assert!(focus.main_active());
        focus.leave_main();
        assert!(!focus.main_active());
        assert!(focus.take_armed());
    }

    #[test]
    fn aux_arrival_cancels_pending_without_resume_flag() {
        let mut focus = FocusState::new();
        focus.leave_main();
        focus.arrive_aux();
        assert!(!focus.take_armed());
    }

    #[test]
    fn settings_arrival_preserves_resume_for_main_return() {
        // Settings arrival cancels without resuming; a prior
        // protocol pause still resumes on the next main arrival.
        let mut focus = FocusState::new();
        focus.leave_aux();
        assert!(focus.take_armed());
        focus.mark_auto_paused();
        focus.cancel_pending();
        assert!(focus.arrive_main());
    }
}
