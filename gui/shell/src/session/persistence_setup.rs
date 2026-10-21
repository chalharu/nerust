//! Persistence setup for the session: sidecar resolution and mapper-save wiring.
//! `SessionHandle` delegates here so save-location policy lives next to the backend.

use std::path::Path;

use crate::session::{
    SessionHandle,
    persistence::{PersistenceManager, StateExporter},
};

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
    /// Restore persistence on a parked candidate before it becomes the live runtime.
    pub(crate) fn prepare(
        session: &mut SessionHandle,
        exporter: &dyn StateExporter,
        rom_path: Option<&Path>,
    ) {
        let setup = setup_persistence(
            &session.settings.manager,
            &mut session.media.persistence,
            Some(exporter),
            rom_path,
            MapperRestore::Load,
        );
        session.note_persistence_setup(setup);
    }

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
        setup_persistence(
            &self.settings.manager,
            &mut self.media.persistence,
            self.emu.exporter().ok(),
            rom_path,
            mapper_restore,
        )
    }
}

fn setup_persistence(
    settings: &nerust_gui_runtime::settings::manager::SettingsManager,
    persistence: &mut PersistenceManager,
    exporter: Option<&dyn StateExporter>,
    rom_path: Option<&Path>,
    mapper_restore: MapperRestore,
) -> PersistenceSetup {
    let sidecars = exporter
        .and_then(|exporter| exporter.export_identity())
        .and_then(|identity| {
            settings
                .resolve_persistence_paths_with_import(
                    identity.system_id.as_ref(),
                    rom_path,
                    &identity,
                )
                .map_err(|error| log::warn!("failed to resolve persistence paths: {error}"))
                .ok()
        });
    let Some(sidecars) = sidecars else {
        // A new ROM must never inherit the previous ROM's save targets.
        persistence.reset();
        return PersistenceSetup::Unavailable;
    };
    persistence.configure(sidecars.states_dir, sidecars.mapper_save_path);
    if let Some(exporter) = exporter {
        persistence.refresh_slots(exporter);
        if mapper_restore == MapperRestore::Load
            && let Err(e) = persistence.load_mapper_save_if_needed(exporter)
        {
            log::warn!("load_mapper_save_if_needed failed: {e}");
            return PersistenceSetup::Unavailable;
        }
    }
    PersistenceSetup::Ready
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
