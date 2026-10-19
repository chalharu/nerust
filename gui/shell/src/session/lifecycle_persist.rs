//! Hidden lifecycle-state commands: save/load/clear the autosave
//! slot across suspend/resume. Mutating session commands (one
//! persistence round-trip each); pure slot getters stay in
//! [`session_query`](crate::session::session_query).

use crate::session::SessionHandle;

impl SessionHandle {
    /// Snapshot runtime state into the hidden autosave slot.
    /// Best-effort for suspend: failures log and report `false`.
    pub fn save_hidden_lifecycle_state(&mut self) -> bool {
        self.emu
            .core()
            .ok()
            .is_some_and(|core| self.media.persistence.save_hidden(core))
    }

    /// Restore runtime state from the hidden autosave slot.
    /// Best-effort for resume: failures log and report `false`.
    pub fn load_hidden_lifecycle_state(&mut self) -> bool {
        self.emu
            .core()
            .ok()
            .is_some_and(|core| self.media.persistence.load_hidden(core))
    }

    /// Drop the hidden autosave slot, if any.
    pub fn clear_hidden_lifecycle_state(&mut self) {
        self.media.persistence.clear_hidden();
    }

    pub fn clear_notification_history(&mut self) {
        self.notifications.clear_history();
    }
}
