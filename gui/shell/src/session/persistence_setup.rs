//! Persistence setup for the session: sidecar resolution, one-time
//! reporting, and mapper-save wiring. `SessionHandle` delegates here so
//! save-location policy lives next to the persistence backend.

use std::path::Path;

use crate::session::SessionHandle;

/// Persistence setup outcome. Callers map `Unavailable` to one
/// `PersistenceUnavailable` notice; the game still runs without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PersistenceSetup {
    Ready,
    Unavailable,
}

/// Whether the mapper save loads from disk during setup. `Load` on
/// fresh ROM loads, `Skip` when the caller already restored runtime
/// state (settings re-apply) and must not clobber it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MapperRestore {
    Load,
    Skip,
}

/// One-call persistence wiring: resolve sidecars, configure the
/// manager, refresh slots, optionally load the mapper save, and
/// report unavailability through the session notifications.
/// Callers never touch the two phases separately, so a bound session
/// cannot forget its notice.
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
            .core()
            .ok()
            .and_then(|c| c.canonical_media_identity())
            .and_then(|identity| self.resolve_persistence_paths(rom_path, &identity));
        let Some(sidecars) = sidecars else {
            return PersistenceSetup::Unavailable;
        };
        self.media
            .persistence
            .configure(sidecars.states_dir, sidecars.mapper_save_path);
        if let Ok(core) = self.emu.core() {
            self.media.persistence.refresh_slots(core);
            if mapper_restore == MapperRestore::Load
                && let Err(e) = self.media.persistence.load_mapper_save_if_needed(core)
            {
                log::warn!("load_mapper_save_if_needed failed: {e}");
            }
        }
        PersistenceSetup::Ready
    }
}
