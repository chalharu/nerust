//! Persistence setup for the session: sidecar resolution and mapper-save wiring.
//! `SessionHandle` delegates here so save-location policy lives next to the backend.

use std::path::Path;

use crate::session::{
    SessionHandle,
    error::SlotOpFailure,
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
    /// Single wiring entry: optional exporter, path, and restore mode.
    /// `None` probes the live core inside, so callers never juggle its borrow.
    /// Returns the setup; the caller pushes the notice via [`SessionHandle::note_persistence_setup`].
    pub(crate) fn bind(
        session: &mut SessionHandle,
        exporter: Option<&dyn StateExporter>,
        rom_path: Option<&Path>,
        mapper_restore: MapperRestore,
    ) -> PersistenceSetup {
        let live;
        let exporter = match exporter {
            Some(exporter) => Some(exporter),
            None => {
                live = session.emu.exporter().ok();
                live
            }
        };
        let (setup, scan_fault) = setup_persistence(
            &session.settings.manager,
            &mut session.media.persistence,
            exporter,
            rom_path,
            mapper_restore,
        );
        // A scan failure keeps the previous list but is not silent: the
        // notice distinguishes a backend failure from an empty list (NER-322).
        if let Some(fault) = scan_fault {
            session
                .notifications
                .push(crate::session::notification::Message::slot_failed(
                    None, fault,
                ));
        }
        setup
    }
}

impl SessionHandle {
    pub(crate) fn note_persistence_setup(&mut self, setup: PersistenceSetup) {
        if setup == PersistenceSetup::Unavailable {
            self.notifications
                .push(crate::session::notification::Message::persistence_unavailable());
        }
    }
}

fn setup_persistence(
    settings: &nerust_gui_runtime::settings::manager::SettingsManager,
    persistence: &mut PersistenceManager,
    exporter: Option<&dyn StateExporter>,
    rom_path: Option<&Path>,
    mapper_restore: MapperRestore,
) -> (PersistenceSetup, Option<SlotOpFailure>) {
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
        return (PersistenceSetup::Unavailable, None);
    };
    persistence.configure(sidecars.states_dir, sidecars.mapper_save_path);
    let mut scan_fault = None;
    if let Some(exporter) = exporter {
        // A failed scan keeps the previous list but is reported: the setup
        // stays Ready while the notice distinguishes failure from empty (NER-322).
        if let Err(error) = persistence.refresh_slots(exporter) {
            scan_fault = Some(SlotOpFailure::from(&error));
        }
        if mapper_restore == MapperRestore::Load
            && let Err(e) = persistence.load_mapper_save_if_needed(exporter)
        {
            log::warn!("load_mapper_save_if_needed failed: {e}");
            return (PersistenceSetup::Unavailable, None);
        }
    }
    (PersistenceSetup::Ready, scan_fault)
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
        let setup = PersistenceBinder::bind(&mut session, None, None, MapperRestore::Skip);
        session.note_persistence_setup(setup);
        assert_eq!(session.drain_notifications().len(), 1);
        // Unavailable setup is noted once via the binder path below.
        let setup = PersistenceBinder::bind(&mut session, None, None, MapperRestore::Load);
        session.note_persistence_setup(setup);
        assert_eq!(session.drain_notifications().len(), 1);
        session.note_persistence_setup(PersistenceSetup::Ready);
        assert_eq!(session.drain_notifications().len(), 0);
        session.note_persistence_setup(PersistenceSetup::Unavailable);
        assert_eq!(session.drain_notifications().len(), 1);
    }

    /// NER-322 AC-2: a failed slot scan keeps the previous list but is not
    /// silent. Setup stays Ready while a storage-failure notice tells an
    /// empty-looking list apart from a backend failure.
    #[test]
    fn failed_scan_stays_ready_but_notifies() {
        use crate::session::{
            error::SlotOpFailure,
            persistence::{PersistenceBackends, StateExporter},
        };
        struct IdentityExporter;
        impl StateExporter for IdentityExporter {
            fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure> {
                Ok(vec![])
            }
            fn import_state(&self, _: Vec<u8>) -> Result<(), SlotOpFailure> {
                Ok(())
            }
            fn export_preview(&self) -> Option<crate::emu_core::PreviewRgba> {
                None
            }
            fn export_identity(&self) -> Option<nerust_core_traits::identity::SystemIdentity> {
                Some(nerust_core_traits::identity::SystemIdentity::new(
                    Box::new(crate::test_helpers::DummySystemId),
                    vec![7],
                ))
            }
            fn export_mapper(&self) -> Result<Option<Vec<u8>>, SlotOpFailure> {
                Ok(None)
            }
            fn import_mapper(&self, _: Vec<u8>) -> Result<(), SlotOpFailure> {
                Ok(())
            }
        }
        let mut session = test_session();
        session
            .media
            .persistence
            .configure_backends(PersistenceBackends {
                slot: Box::new(crate::session::persistence::FailingSlotBackend),
                autosave: Box::new(crate::session::persistence::FsAutosave),
                mapper: Box::new(crate::session::persistence::FsMapper),
            });
        let dir = crate::test_helpers::unique_temp_dir("scan-notice");
        let exporter = IdentityExporter;
        let setup = PersistenceBinder::bind(
            &mut session,
            Some(&exporter),
            Some(&dir.join("game.nes")),
            MapperRestore::Skip,
        );
        session.note_persistence_setup(setup);
        assert_eq!(setup, PersistenceSetup::Ready);
        let notices = session.drain_notifications();
        assert_eq!(notices.len(), 1, "scan failure is notified, not silent");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn binder_without_rom_reports_unavailable() {
        let mut session = test_session();
        let setup = PersistenceBinder::bind(&mut session, None, None, MapperRestore::Skip);
        session.note_persistence_setup(setup);
        assert_eq!(session.drain_notifications().len(), 1);
    }
}
