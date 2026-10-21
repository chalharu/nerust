//! Host-side half of the deferred auto-pause protocol: session
//! driving for the coordinator in [`crate::aux::focus`].

use super::HostState;
use crate::aux::focus::{FocusEffect, FocusEvent};
use nerust_frontend_kernel::pause::PauseOwner;
use nerust_gui_shell::session::access::PauseControl;

impl HostState {
    pub(crate) fn set_pause_owner(&mut self, owner: PauseOwner, active: bool) {
        let tick = self.session.tick_metrics();
        let paused = self.pause_policy.set(owner, active, tick.paused);
        self.session.set_execution_held(paused);
        if tick.loaded && tick.paused != paused {
            let _ = self.session.run_command(if paused {
                nerust_gui_shell::session::commands::SessionCommand::Pause
            } else {
                nerust_gui_shell::session::commands::SessionCommand::Resume
            });
        }
    }

    pub(super) fn reconcile_pause(&mut self) {
        let tick = self.session.tick_metrics();
        self.session.set_execution_held(self.pause_policy.paused());
        if tick.loaded && !tick.paused && self.pause_policy.paused() {
            let _ = self
                .session
                .run_command(nerust_gui_shell::session::commands::SessionCommand::Pause);
        }
    }

    pub(crate) fn release_focus_pause(&mut self) {
        self.set_pause_owner(PauseOwner::Focus, false);
    }

    pub(crate) fn resume_session(&mut self) {
        PauseControl::resume(self);
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
            self.release_focus_pause();
        }
        focused
    }

    /// Execute a still-armed auto-pause at the end of the event burst.
    pub(super) fn tick_focus_sync(&mut self) {
        if self.focus_event(FocusEvent::BurstEnd) != Some(FocusEffect::Pause) {
            return;
        }
        if self.session.tick_metrics().loaded {
            self.set_pause_owner(PauseOwner::Focus, true);
            self.note_protocol_pause();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::host::TestHost;
    use nerust_gui_shell::session::access::PauseControl;

    #[test]
    fn focus_modal_and_explicit_pause_compose_across_actual_media_replacement() {
        use nerust_frontend_kernel::pause::PauseOwner;
        use nerust_gui_shell::session::access::PauseControl;
        let mut host = TestHost::with_nes().host;
        host.load_test_rom();
        host.set_pause_owner(PauseOwner::Modal, true);
        host.note_main_departure();
        host.tick_focus_sync();
        assert!(host.pause_policy.owns(PauseOwner::Focus));
        host.load_test_rom();
        assert!(host.session().tick_metrics().paused);
        assert!(host.note_main_arrival());
        host.release_focus_pause();
        assert!(
            host.session().tick_metrics().paused,
            "modal still owns pause"
        );
        host.pause();
        host.set_pause_owner(PauseOwner::Modal, false);
        assert!(
            host.session().tick_metrics().paused,
            "explicit pause survives ACK"
        );
        host.resume();
        assert!(!host.session().tick_metrics().paused);
    }

    #[test]
    fn session_verbs_stay_total_headless() {
        let support = TestHost::new();
        let mut host = support.host;
        // No loaded core: pause and resume run commands that fail
        // closed without touching the menu.
        host.pause();
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
