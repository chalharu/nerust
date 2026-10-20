//! Persistence setup for the session: sidecar resolution and mapper-save wiring.
//! `SessionHandle` delegates here so save-location policy lives next to the backend.

use std::path::Path;

use crate::session::SessionHandle;

/// Persistence setup outcome. Callers map `Unavailable` to one
/// `PersistenceUnavailable` notice; the game still runs without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceSetup {
    Ready,
    Unavailable,
}

/// Whether the mapper save loads from disk during setup.
/// `Load` on fresh ROM loads; `Skip` when the caller already restored runtime state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapperRestore {
    Load,
    Skip,
}

/// One-call persistence wiring: resolve sidecars, configure the manager, refresh slots.
/// Optionally loads the mapper save and reports unavailability through notifications.
pub(crate) struct PersistenceBinder;

impl PersistenceBinder {
    pub(crate) fn bind(
        session: &mut SessionHandle,
        rom_path: Option<&Path>,
        mapper_restore: MapperRestore,
    ) {
        let setup = session.setup_persistence(rom_path, mapper_restore);
        session.note_persistence_setup(setup);
    }
}

impl SessionHandle {
    fn resolve_persistence_paths(
        &self,
        rom_path: Option<&Path>,
        identity: &nerust_core_traits::identity::SystemIdentity,
    ) -> Option<nerust_persistence::sidecar::SidecarPaths> {
        self.settings
            .manager
            .resolve_persistence_paths_with_import(identity.system_id.as_ref(), rom_path, identity)
            .map_err(|error| {
                log::warn!("failed to resolve persistence paths: {error}");
                error
            })
            .ok()
    }

    pub(crate) fn note_persistence_setup(&mut self, setup: PersistenceSetup) {
        if setup == PersistenceSetup::Unavailable {
            self.notifications
                .push(crate::session::notification::NotificationKind::PersistenceUnavailable);
        }
    }

    pub(crate) fn setup_persistence(
        &mut self,
        rom_path: Option<&Path>,
        mapper_restore: MapperRestore,
    ) -> PersistenceSetup {
        let sidecars = self
            .emu
            .exporter()
            .ok()
            .and_then(|exporter| exporter.export_identity())
            .and_then(|identity| self.resolve_persistence_paths(rom_path, &identity));
        let Some(sidecars) = sidecars else {
            return PersistenceSetup::Unavailable;
        };
        self.media
            .persistence
            .configure(sidecars.states_dir, sidecars.mapper_save_path);
        if let Ok(exporter) = self.emu.exporter() {
            self.media.persistence.refresh_slots(exporter);
            if mapper_restore == MapperRestore::Load
                && let Err(e) = self.media.persistence.load_mapper_save_if_needed(exporter)
            {
                log::warn!("load_mapper_save_if_needed failed: {e}");
            }
        }
        PersistenceSetup::Ready
    }
}

#[cfg(test)]
mod persistence_setup_cov_tests {
    use crate::session::persistence_setup::*;
    use crate::session::test_util::test_session;

    #[test]
    fn coreless_setup_is_unavailable_and_noted() {
        use std::sync::Arc;
        let caps = test_session().capabilities;
        let registry = Arc::new(crate::registry::SystemRegistry::new(vec![]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let mut session = SessionHandle::new_ephemeral(caps, registry, audio);
        assert_eq!(
            session.setup_persistence(None, MapperRestore::Skip),
            PersistenceSetup::Unavailable
        );
        assert_eq!(
            session.setup_persistence(None, MapperRestore::Load),
            PersistenceSetup::Unavailable
        );
        session.note_persistence_setup(PersistenceSetup::Ready);
        assert_eq!(session.drain_notifications().len(), 0);
        session.note_persistence_setup(PersistenceSetup::Unavailable);
        assert_eq!(session.drain_notifications().len(), 1);
    }

    #[test]
    fn binder_without_rom_reports_unavailable() {
        let mut session = test_session();
        PersistenceBinder::bind(&mut session, None, MapperRestore::Skip);
        assert_eq!(session.drain_notifications().len(), 1);
    }
}
