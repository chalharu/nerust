//! Host-side half of the deferred auto-pause protocol: session
//! driving for the state machine in [`crate::aux::focus`].

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
        if self.session.tick_metrics().can_pause() {
            self.pause_session();
            self.focus.mark_auto_paused();
            self.sync_debug_chrome();
        }
    }
}
