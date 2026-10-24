//! Hidden lifecycle-state commands: save/load/clear the autosave slot across suspend/resume.
//! Mutating session commands; pure slot getters stay in `session_query`.

use crate::session::SessionHandle;

impl SessionHandle {
    /// Snapshot runtime state into the hidden autosave slot.
    /// Best-effort for suspend: failures log and report `false`.
    pub fn save_hidden_lifecycle_state(&mut self) -> bool {
        self.emu
            .exporter()
            .ok()
            .is_some_and(|exporter| self.media.persistence.save_hidden(exporter))
    }

    /// Restore runtime state from the hidden autosave slot.
    /// Best-effort for resume: failures log and report `false`.
    pub fn load_hidden_lifecycle_state(&mut self) -> bool {
        self.emu.invalidate_debug();
        self.debug_clear_write();
        self.emu
            .exporter()
            .ok()
            .is_some_and(|exporter| self.media.persistence.load_hidden(exporter))
    }

    /// Drop the hidden autosave slot, if any.
    pub fn clear_hidden_lifecycle_state(&mut self) {
        self.media.persistence.clear_hidden();
    }

    pub fn clear_notification_history(&mut self) {
        self.notifications.clear_history();
    }
}

#[cfg(test)]
mod lifecycle_persist_cov_tests {
    use crate::session::lifecycle_persist::*;
    use crate::session::test_util::test_session;

    #[test]
    fn coreless_hidden_state_is_false_and_clearable() {
        use std::sync::Arc;
        let caps = test_session().capabilities;
        let registry = Arc::new(crate::registry::SystemRegistry::new(vec![]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let mut session = SessionHandle::new_ephemeral(caps, registry, audio);
        assert!(!session.save_hidden_lifecycle_state());
        assert!(!session.load_hidden_lifecycle_state());
        session.clear_hidden_lifecycle_state();
        session.clear_notification_history();
        assert_eq!(session.notification_history_seq(), 1);
    }

    #[test]
    fn loaded_hidden_state_round_trips_without_slots() {
        let mut session = test_session();
        session.clear_hidden_lifecycle_state();
        session.clear_notification_history();
        assert_eq!(session.notification_history_seq(), 1);
    }
}
