//! Focus-driven auto-pause and session suspend/resume.

use super::HostState;
use nerust_gui_shell::session::access::FrontendSession;

impl HostState {
    pub(crate) fn resume_session(&mut self) {
        FrontendSession::resume(self);
    }

    pub(crate) fn pause_session(&mut self) {
        FrontendSession::pause(self);
    }

    pub(crate) fn auto_paused(&self) -> bool {
        self.auto_paused
    }

    pub(crate) fn set_auto_paused(&mut self) {
        self.auto_paused = true;
    }

    pub(crate) fn clear_auto_paused(&mut self) {
        self.auto_paused = false;
    }

    /// Arm a deferred auto-pause on focus loss. Execution waits for
    /// `commit_pending_auto_pause` so a same-burst focus arrival on
    /// another of our windows can cancel it first.
    pub(crate) fn arm_pending_auto_pause(&mut self) {
        self.pending_auto_pause = true;
    }

    /// Cancel a deferred auto-pause: focus arrived on another of our
    /// windows, so the session stays as the user left it.
    pub(crate) fn cancel_pending_auto_pause(&mut self) {
        self.pending_auto_pause = false;
    }

    /// Execute a still-armed auto-pause at the end of the event burst.
    /// Post-pause chrome sync keeps the menu/title truthful (the blur
    /// path below only synced pre-pause truth).
    pub(crate) fn commit_pending_auto_pause(&mut self) {
        if !std::mem::take(&mut self.pending_auto_pause) {
            return;
        }
        if self.session.can_pause() {
            self.pause_session();
            self.set_auto_paused();
            self.sync_debug_chrome();
        }
    }
}
