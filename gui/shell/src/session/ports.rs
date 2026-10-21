//! Persistence ports: slot/autosave/mapper backends plus the manager-facing traits.
//! Platform adapters implement these; `fs` holds the desktop filesystem one.

use crate::emu_core::PreviewRgba;
use crate::session::error::SlotOpFailure;
use nerust_core_traits::identity::SystemIdentity;
use nerust_persistence::{
    error::PersistenceError,
    model::{LoadedStateSlot, StateSlotSummary},
    sidecar::StorageLocation,
    thumbnail::ThumbnailSource,
};

/// Persistence-facing view of the live core: slot bytes are stored verbatim.
/// Domain-owned port; the live core implements it, never defines it.
pub trait StateExporter {
    /// Export raw core state bytes for slot storage.
    fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure>;
    /// Import raw core state bytes previously resolved by the caller.
    fn import_state(&self, data: Vec<u8>) -> Result<(), SlotOpFailure>;
    /// Preview pixels for the slot thumbnail, if a frame is available.
    fn export_preview(&self) -> Option<PreviewRgba>;
    /// Identity the slot is filed under.
    fn export_identity(&self) -> Option<SystemIdentity>;
    /// Canonical media identity for slot filing; defaults to [`export_identity`](Self::export_identity).
    fn canonical_media_identity(&self) -> Option<SystemIdentity> {
        self.export_identity()
    }
    /// Export the battery-backed mapper save, if the core holds one.
    fn export_mapper(&self) -> Result<Option<Vec<u8>>, SlotOpFailure>;
    /// Import a battery-backed mapper save from disk.
    fn import_mapper(&self, bytes: Vec<u8>) -> Result<(), SlotOpFailure>;
    /// Resolve a stored state blob to raw core bytes.
    /// Default decodes the shell envelope; backends never call the free function directly.
    fn resolve_format(&self, bytes: &[u8]) -> Vec<u8> {
        crate::state::resolve_state_format(bytes)
    }
}

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

/// Platform abstraction for state slot I/O (Desktop fs / Android SAF).
pub trait SlotBackend: Send {
    fn scan(
        &self,
        dir: &StorageLocation,
        identity: &SystemIdentity,
    ) -> Result<Vec<StateSlotSummary>, PersistenceError>;
    fn allocate_next_id(&self, dir: &StorageLocation) -> Result<u64, PersistenceError>;
    fn write_slot(
        &self,
        dir: &StorageLocation,
        slot_id: u64,
        data: &[u8],
        identity: &SystemIdentity,
        thumbnail: Option<&ThumbnailSource>,
    ) -> Result<StateSlotSummary, PersistenceError>;
    fn read_slot(
        &self,
        dir: &StorageLocation,
        slot_id: u64,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError>;
    fn delete_slot(&self, dir: &StorageLocation, slot_id: u64) -> Result<(), PersistenceError>;
}

/// Platform abstraction for autosave I/O.
pub trait AutoSaveBackend: Send {
    fn write_autosave(
        &self,
        dir: &StorageLocation,
        data: &[u8],
        identity: &SystemIdentity,
    ) -> Result<StateSlotSummary, PersistenceError>;
    fn read_autosave(
        &self,
        dir: &StorageLocation,
        identity: &SystemIdentity,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError>;
    fn delete_autosave(&self, dir: &StorageLocation) -> Result<(), PersistenceError>;
    /// Quarantine the hidden autosave aside instead of deleting it.
    /// Returns the quarantine path, or `None` when nothing was kept.
    fn quarantine_autosave(
        &self,
        dir: &StorageLocation,
    ) -> Result<Option<StorageLocation>, PersistenceError> {
        self.delete_autosave(dir).map(|_| None)
    }
}

/// Platform abstraction for mapper save I/O.
pub trait MapperSaveBackend: Send {
    fn read_mapper_save(&self, path: &StorageLocation)
    -> Result<Option<Vec<u8>>, PersistenceError>;
    fn write_mapper_save(
        &self,
        path: &StorageLocation,
        data: &[u8],
    ) -> Result<(), PersistenceError>;
    fn write_recovery_mapper_save(
        &self,
        path: &StorageLocation,
        data: &[u8],
    ) -> Result<StorageLocation, PersistenceError>;
}

#[cfg(test)]
mod ports_cov_tests {
    use crate::session::error::SlotOpFailure;
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
                _: &StorageLocation,
                _: &[u8],
                _: &SystemIdentity,
            ) -> Result<StateSlotSummary, PersistenceError> {
                panic!("unused")
            }
            fn read_autosave(
                &self,
                _: &StorageLocation,
                _: &SystemIdentity,
            ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
                Ok(None)
            }
            fn delete_autosave(&self, _: &StorageLocation) -> Result<(), PersistenceError> {
                Ok(())
            }
        }
        let dir = std::env::temp_dir().join("nerust-ports-quarantine-probe");
        assert_eq!(Noop.quarantine_autosave(&dir.into()).unwrap(), None);
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
            fn export_preview(&self) -> Option<crate::emu_core::PreviewRgba> {
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
