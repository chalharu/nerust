//! Filesystem backend for the persistence ports: Desktop fs slot/autosave/mapper I/O.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use nerust_core_traits::identity::SystemIdentity;
use nerust_persistence::{
    error::PersistenceError,
    model::{LoadedStateSlot, StateSlotSummary},
    sidecar::{load_mapper_save, write_mapper_save, write_recovery_mapper_save},
    slots::{
        allocate_next_slot_id, autosave_state_slot_path, delete_state_slot, load_state_slot,
        load_state_slot_for_identity, scan_state_slots_for_identity, state_slot_path,
        write_autosave_state_slot, write_state_slot,
    },
    thumbnail::ThumbnailSource,
};

use super::ports::{AutoSaveBackend, MapperSaveBackend, SlotBackend};

/// Desktop filesystem backend for all three persistence legs.
pub struct FsSlotBackend;

/// A failing backend for testing error paths: always returns `Err` for all operations.
#[derive(Debug)]
pub struct FailingSlotBackend;

impl SlotBackend for FailingSlotBackend {
    fn scan(
        &self,
        _dir: &Path,
        _identity: &SystemIdentity,
    ) -> Result<Vec<StateSlotSummary>, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
    fn allocate_next_id(&self, _dir: &Path) -> Result<u64, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
    fn write_slot(
        &self,
        _dir: &Path,
        _slot_id: u64,
        _data: &[u8],
        _identity: &SystemIdentity,
        _thumbnail: Option<&ThumbnailSource>,
    ) -> Result<StateSlotSummary, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
    fn read_slot(
        &self,
        _dir: &Path,
        _slot_id: u64,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
    fn delete_slot(&self, _dir: &Path, _slot_id: u64) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
}
impl AutoSaveBackend for FailingSlotBackend {
    fn write_autosave(
        &self,
        _dir: &Path,
        _data: &[u8],
        _identity: &SystemIdentity,
    ) -> Result<StateSlotSummary, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
    fn read_autosave(
        &self,
        _dir: &Path,
        _identity: &SystemIdentity,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
    fn delete_autosave(&self, _dir: &Path) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
}
impl MapperSaveBackend for FailingSlotBackend {
    fn read_mapper_save(&self, _path: &Path) -> Result<Option<Vec<u8>>, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
    fn write_mapper_save(&self, _path: &Path, _data: &[u8]) -> Result<(), PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
    fn write_recovery_mapper_save(
        &self,
        _path: &Path,
        _data: &[u8],
    ) -> Result<PathBuf, PersistenceError> {
        Err(PersistenceError::Io(std::io::Error::other(
            "simulated failure",
        )))
    }
}

impl SlotBackend for FsSlotBackend {
    fn scan(
        &self,
        dir: &Path,
        identity: &SystemIdentity,
    ) -> Result<Vec<StateSlotSummary>, PersistenceError> {
        scan_state_slots_for_identity(dir, identity)
    }
    fn allocate_next_id(&self, dir: &Path) -> Result<u64, PersistenceError> {
        allocate_next_slot_id(dir)
    }
    fn write_slot(
        &self,
        dir: &Path,
        slot_id: u64,
        data: &[u8],
        identity: &SystemIdentity,
        thumbnail: Option<&ThumbnailSource>,
    ) -> Result<StateSlotSummary, PersistenceError> {
        write_state_slot(dir, slot_id, data, identity, thumbnail)
    }
    fn read_slot(
        &self,
        dir: &Path,
        slot_id: u64,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
        match load_state_slot(&state_slot_path(dir, slot_id)) {
            Ok(slot) => Ok(Some(slot)),
            Err(PersistenceError::Io(e)) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn delete_slot(&self, dir: &Path, slot_id: u64) -> Result<(), PersistenceError> {
        delete_state_slot(&state_slot_path(dir, slot_id))
    }
}
impl AutoSaveBackend for FsSlotBackend {
    fn write_autosave(
        &self,
        dir: &Path,
        data: &[u8],
        identity: &SystemIdentity,
    ) -> Result<StateSlotSummary, PersistenceError> {
        write_autosave_state_slot(dir, data, identity, None)
    }
    fn read_autosave(
        &self,
        dir: &Path,
        identity: &SystemIdentity,
    ) -> Result<Option<LoadedStateSlot>, PersistenceError> {
        load_state_slot_for_identity(&autosave_state_slot_path(dir), identity)
    }
    fn delete_autosave(&self, dir: &Path) -> Result<(), PersistenceError> {
        delete_state_slot(&autosave_state_slot_path(dir))
    }
    fn quarantine_autosave(&self, dir: &Path) -> Result<Option<PathBuf>, PersistenceError> {
        let path = autosave_state_slot_path(dir);
        if !path.exists() {
            return Ok(None);
        }
        let quarantined = dir.join(".autosave_slot.quarantined");
        let _ = std::fs::remove_file(&quarantined);
        match std::fs::rename(&path, &quarantined) {
            Ok(()) => Ok(Some(quarantined)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}
impl MapperSaveBackend for FsSlotBackend {
    fn read_mapper_save(&self, path: &Path) -> Result<Option<Vec<u8>>, PersistenceError> {
        load_mapper_save(path)
    }
    fn write_mapper_save(&self, path: &Path, data: &[u8]) -> Result<(), PersistenceError> {
        write_mapper_save(path, data)
    }
    fn write_recovery_mapper_save(
        &self,
        path: &Path,
        data: &[u8],
    ) -> Result<PathBuf, PersistenceError> {
        write_recovery_mapper_save(path, data)
    }
}
