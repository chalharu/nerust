//! Host-side half of the deferred auto-pause protocol: session
//! driving for the coordinator in [`crate::aux::focus`].

use super::HostState;
use crate::aux::focus::{FocusEffect, FocusEvent};
use nerust_gui_shell::session::access::PauseControl;

impl HostState {
    pub(crate) fn resume_session(&mut self) {
        PauseControl::resume(self);
    }

    pub(crate) fn pause_session(&mut self) {
        PauseControl::pause(self);
    }

    /// Execute a still-armed auto-pause at the end of the event burst.
    pub(super) fn tick_focus_sync(&mut self) {
        if self.focus_event(FocusEvent::BurstEnd) != Some(FocusEffect::Pause) {
            return;
        }
        if self.session.tick_metrics().can_pause() {
            self.pause_session();
            self.note_protocol_pause();
            let ctx = self.frame_ctx();
            self.sync_debug_chrome(&ctx);
        }
    }
}
