//! Narrow session services: each owns one domain behind a trait.
//! SessionHandle delegates; callers depend on these traits.

use super::{SessionError, SessionHandle};
use crate::emu_core::LiveCore;

/// Input domain: assignments and key maps.
pub trait InputService {
    /// Rebuild the keyboard reverse map.
    fn rebuild_key_field_map(&mut self);
    /// Persistable slot/controller pairs.
    fn persistable_pairs(&self) -> Vec<(String, Option<String>)>;
}

/// Live-core domain: single typed access, mapped once at the boundary.
/// Callers depend on the port, never the concrete core.
pub trait EmuService {
    /// Shared live core; `NoCore` maps once at the boundary.
    fn require_live(&self) -> Result<&dyn LiveCore, SessionError>;
    /// Exclusive live core for swaps.
    fn require_live_mut(&mut self) -> Result<&mut Box<dyn LiveCore>, SessionError>;
    /// Run one closure against the live core.
    fn with_live<R>(&self, f: impl FnOnce(&dyn LiveCore) -> R) -> Result<R, SessionError>;
}

/// Media domain: loaded media plus persistence.
pub trait MediaService {
    /// Whether runtime media is loaded.
    fn has_loaded_media(&self) -> bool;
    /// Clear runtime media and persistence.
    fn clear_runtime(&mut self);
}

/// Settings domain: manager plus snapshot.
pub trait SettingsService {
    /// Current settings snapshot.
    fn snapshot(&self) -> &nerust_gui_runtime::settings::SettingsSnapshot;
}

/// Notification domain: push and drain.
pub trait NotificationService {
    /// Push one event.
    fn push(&mut self, kind: super::notification::NotificationKind);
    /// Pending queue length.
    fn pending_len(&self) -> usize;
}

impl InputService for SessionHandle {
    fn rebuild_key_field_map(&mut self) {
        SessionHandle::rebuild_key_field_map(self);
    }

    fn persistable_pairs(&self) -> Vec<(String, Option<String>)> {
        self.current_assignments_pairs()
    }
}

impl EmuService for SessionHandle {
    fn require_live(&self) -> Result<&dyn LiveCore, SessionError> {
        self.emu.require_live()
    }

    fn require_live_mut(&mut self) -> Result<&mut Box<dyn LiveCore>, SessionError> {
        self.emu.require_live_mut()
    }

    fn with_live<R>(&self, f: impl FnOnce(&dyn LiveCore) -> R) -> Result<R, SessionError> {
        self.emu.require_live().map(f)
    }
}

impl MediaService for SessionHandle {
    fn has_loaded_media(&self) -> bool {
        self.media.loaded.is_some()
    }

    fn clear_runtime(&mut self) {
        self.media.clear_runtime();
    }
}

impl SettingsService for SessionHandle {
    fn snapshot(&self) -> &nerust_gui_runtime::settings::SettingsSnapshot {
        self.settings_snapshot()
    }
}

impl NotificationService for SessionHandle {
    fn push(&mut self, kind: super::notification::NotificationKind) {
        self.notifications.push(kind);
    }

    fn pending_len(&self) -> usize {
        self.notifications.pending_len()
    }
}

#[cfg(test)]
mod tests {
    use crate::session::services::*;
    use crate::session::test_util::test_session;

    #[test]
    fn input_service_delegates() {
        let mut session = test_session();
        let before = session.persistable_pairs();
        session.rebuild_key_field_map();
        assert_eq!(session.persistable_pairs(), before);
    }

    #[test]
    fn emu_service_require_live_maps_once() {
        let session = test_session();
        assert!(session.require_live().is_ok());
        assert!(session.with_live(|core| core.tick_snapshot()).is_ok());
    }

    #[test]
    fn media_service_reports_loaded() {
        let session = test_session();
        assert!(!session.has_loaded_media());
    }

    #[test]
    fn settings_service_exposes_snapshot() {
        let session = test_session();
        let _ = session.snapshot();
    }

    #[test]
    fn notification_service_push_and_pending() {
        let mut session = test_session();
        session.push(crate::session::notification::NotificationKind::Paused);
        assert_eq!(session.pending_len(), 1);
    }

    #[test]
    fn emu_service_maps_coreless_once() {
        use std::sync::Arc;
        let caps = test_session().capabilities;
        let registry = Arc::new(crate::registry::SystemRegistry::new(vec![]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let mut session = super::super::SessionHandle::new_ephemeral(caps, registry, audio);
        assert!(session.require_live().is_err());
        assert!(session.require_live_mut().is_err());
        assert!(session.with_live(|_| ()).is_err());
        assert!(test_session().require_live_mut().is_ok());
    }

    #[test]
    fn media_service_clears_runtime() {
        let mut session = test_session();
        assert!(!session.has_loaded_media());
        session.clear_runtime();
        assert!(!session.has_loaded_media());
    }
}
