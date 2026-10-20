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

    /// Shared aux-focus drive: every aux path routes here; sync/drain defer to burst.
    pub(crate) fn drive_aux_focus(
        &mut self,
        kind: crate::aux::focus::AuxKind,
        focused: bool,
    ) -> bool {
        let mut resume = false;
        if let Some(effect) = self.focus_event(crate::aux::focus::FocusEvent::Aux(kind, focused))
            && effect == FocusEffect::Resume
        {
            resume = true;
        }
        if resume {
            self.resume_session();
        }
        focused
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

#[cfg(test)]
mod tests {
    use crate::runtime::host::TestHost;

    #[test]
    fn session_verbs_stay_total_headless() {
        let support = TestHost::new();
        let mut host = support.host;
        // No loaded core: pause and resume run commands that fail
        // closed without touching the menu.
        host.pause_session();
        host.resume_session();
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn unarmed_burst_end_commits_nothing() {
        let support = TestHost::new();
        let mut host = support.host;
        host.tick_focus_sync();
        assert!(support.menu.updates().is_empty());
        assert!(!host.note_main_arrival());
    }

    #[test]
    fn armed_burst_end_without_core_stays_paused_off() {
        let support = TestHost::new();
        let mut host = support.host;
        host.note_main_departure();
        // Coreless tick cannot pause, so no protocol flag is set
        // and the arrival reports no resume.
        host.tick_focus_sync();
        assert!(!host.note_main_arrival());
        assert!(support.menu.updates().is_empty());
    }
}
