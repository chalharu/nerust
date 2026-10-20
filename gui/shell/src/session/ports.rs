//! Persistence ports: slot/autosave/mapper backends plus the manager-facing traits.
//! Platform adapters implement these; `fs` holds the desktop filesystem one.

use std::path::{Path, PathBuf};

use nerust_core_traits::identity::SystemIdentity;
use nerust_persistence::{
    error::PersistenceError,
    model::{LoadedStateSlot, StateSlotSummary},
    thumbnail::ThumbnailSource,
};

use super::error::SlotOpFailure;

/// Persistence-facing view of the live core: slot bytes are stored verbatim.
/// The caller resolves the envelope at the session boundary; implemented for the live core.
pub trait StateExporter {
    /// Export raw core state bytes for slot storage.
    fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure>;
    /// Import raw core state bytes previously resolved by the caller.
    fn import_state(&self, data: Vec<u8>) -> Result<(), SlotOpFailure>;
    /// Preview pixels for the slot thumbnail, if a frame is available.
    fn export_preview(&self) -> Option<ThumbnailSource>;
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

/// One-bundle backend set for [`PersistenceManager`](super::persistence::PersistenceManager::configure_backends).
/// Replaces the three-`Box` parameter list, which read as positional noise at every call site.
pub struct PersistenceBackends {
    pub slot: Box<dyn SlotBackend>,
    pub autosave: Box<dyn AutoSaveBackend>,
    pub mapper: Box<dyn MapperSaveBackend>,
}

impl PersistenceBackends {
    /// Desktop filesystem set for all three legs.
    pub fn filesystem() -> Self {
        Self {
            slot: Box::new(super::fs::FsSlotBackend),
            autosave: Box::new(super::fs::FsSlotBackend),
            mapper: Box::new(super::fs::FsSlotBackend),
        }
    }
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
