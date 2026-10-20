//! Persistence ports: slot/autosave/mapper backends plus the manager-facing traits.
//! Platform adapters implement these; `fs` holds the desktop filesystem one.

use std::path::{Path, PathBuf};

use nerust_core_traits::identity::SystemIdentity;
use nerust_persistence::{
    error::PersistenceError,
    model::{LoadedStateSlot, StateSlotSummary},
    thumbnail::ThumbnailSource,
};

/// Re-exported from the live-core home; ports depend on it, never define it.
pub use crate::emu_core::StateExporter;

/// Whether a save also moves the active-slot pointer. Explicit enum:
/// a bare `bool` at six call sites never says which way it flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SlotActivation {
    /// Point the active slot at the saved slot.
    #[default]
    Activate,
    /// Leave the active-slot pointer alone.
    Leave,
}

/// One-bundle backend set for the persistence manager.
/// Construct literally at the composition root or in tests.
pub struct PersistenceBackends {
    pub slot: Box<dyn SlotBackend>,
    pub autosave: Box<dyn AutoSaveBackend>,
    pub mapper: Box<dyn MapperSaveBackend>,
}

/// Loader-facing load leg: options, resolved loads, resume, activation.
pub trait LoadPorts {
    /// Default load options for the active system.
    fn default_load_options(
        &self,
    ) -> Option<Box<dyn nerust_core_traits::factory::load::DynSystemLoadOptions>>;
    /// Load resolved media.
    fn load_resolved(
        &mut self,
        media: nerust_core_traits::factory::load::MediaObject,
        resolved: nerust_core_traits::factory::load::ResolvedLoadRequest,
    ) -> Result<(), crate::load::RomLoaderError>;
    /// Resume after load.
    fn resume(&mut self);
    /// Activate a system before load.
    fn set_active_system(
        &mut self,
        system_id: &dyn nerust_core_traits::identity::SystemId,
    ) -> Result<(), crate::load::SystemActivationError>;
}

/// Loader-facing snapshot leg: current settings snapshot.
pub trait SnapshotPort {
    /// Current settings snapshot.
    fn settings_snapshot(&self) -> &nerust_gui_runtime::settings::SettingsSnapshot;
}

/// Platform abstraction for state slot I/O (Desktop fs / Android SAF).
pub trait SlotBackend: Send {
    fn scan(
        &self,
        dir: &Path,
        identity: &SystemIdentity,
    ) -> Result<Vec<StateSlotSummary>, PersistenceError>;
    fn allocate_next_id(&self, dir: &Path) -> Result<u64, PersistenceError>;
    fn write_slot(
        &self,
        dir: &Path,
        slot_id: u64,
        data: &[u8],
        identity: &SystemIdentity,
        thumbnail: Option<&ThumbnailSource>,
    ) -> Result<StateSlotSummary, PersistenceError>;
    fn read_slot(
        &self,
        dir: &Path,
        slot_id: u64,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError>;
    fn delete_slot(&self, dir: &Path, slot_id: u64) -> Result<(), PersistenceError>;
}

/// Platform abstraction for autosave I/O.
pub trait AutoSaveBackend: Send {
    fn write_autosave(
        &self,
        dir: &Path,
        data: &[u8],
        identity: &SystemIdentity,
    ) -> Result<StateSlotSummary, PersistenceError>;
    fn read_autosave(
        &self,
        dir: &Path,
        identity: &SystemIdentity,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError>;
    fn delete_autosave(&self, dir: &Path) -> Result<(), PersistenceError>;
    /// Quarantine the hidden autosave aside instead of deleting it.
    /// Returns the quarantine path, or `None` when nothing was kept.
    fn quarantine_autosave(&self, dir: &Path) -> Result<Option<PathBuf>, PersistenceError> {
        self.delete_autosave(dir).map(|_| None)
    }
}

/// Platform abstraction for mapper save I/O.
pub trait MapperSaveBackend: Send {
    fn read_mapper_save(&self, path: &Path) -> Result<Option<Vec<u8>>, PersistenceError>;
    fn write_mapper_save(&self, path: &Path, data: &[u8]) -> Result<(), PersistenceError>;
    fn write_recovery_mapper_save(
        &self,
        path: &Path,
        data: &[u8],
    ) -> Result<PathBuf, PersistenceError>;
}

#[cfg(test)]
mod ports_cov_tests {
    use crate::emu_core::SlotOpFailure;
    use crate::session::ports::*;

    #[test]
    fn default_slot_activation_is_activate() {
        assert_eq!(SlotActivation::default(), SlotActivation::Activate);
    }

    #[test]
    fn default_quarantine_deletes() {
        struct Noop;
        impl AutoSaveBackend for Noop {
            fn write_autosave(
                &self,
                _: &Path,
                _: &[u8],
                _: &SystemIdentity,
            ) -> Result<StateSlotSummary, PersistenceError> {
                panic!("unused")
            }
            fn read_autosave(
                &self,
                _: &Path,
                _: &SystemIdentity,
            ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
                Ok(None)
            }
            fn delete_autosave(&self, _: &Path) -> Result<(), PersistenceError> {
                Ok(())
            }
        }
        let dir = std::env::temp_dir().join("nerust-ports-quarantine-probe");
        assert_eq!(Noop.quarantine_autosave(&dir).unwrap(), None);
    }

    #[test]
    fn state_exporter_default_identity_matches_export() {
        struct IdOnly(Option<SystemIdentity>);
        impl StateExporter for IdOnly {
            fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure> {
                Ok(vec![1])
            }
            fn import_state(&self, _: Vec<u8>) -> Result<(), SlotOpFailure> {
                Ok(())
            }
            fn export_preview(&self) -> Option<ThumbnailSource> {
                None
            }
            fn export_identity(&self) -> Option<SystemIdentity> {
                self.0.clone()
            }
            fn export_mapper(&self) -> Result<Option<Vec<u8>>, SlotOpFailure> {
                Ok(None)
            }
            fn import_mapper(&self, _: Vec<u8>) -> Result<(), SlotOpFailure> {
                Ok(())
            }
        }
        assert!(IdOnly(None).canonical_media_identity().is_none());
    }
}
