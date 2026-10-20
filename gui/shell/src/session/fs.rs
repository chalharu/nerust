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

/// Filesystem slot-I/O leg: state slot scan/read/write/delete only.
pub struct FsSlots;

/// Filesystem autosave leg: hidden-slot read/write/delete/quarantine only.
pub struct FsAutosave;

/// Filesystem mapper-save leg: sidecar save/load/recovery only.
pub struct FsMapper;

impl SlotBackend for FsSlots {
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

impl AutoSaveBackend for FsAutosave {
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
        // Atomic rename: a missing source maps to `None`, no TOCTOU probe first.
        let path = autosave_state_slot_path(dir);
        let quarantined = dir.join(".autosave_slot.quarantined");
        let _ = std::fs::remove_file(&quarantined);
        match std::fs::rename(&path, &quarantined) {
            Ok(()) => Ok(Some(quarantined)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

impl MapperSaveBackend for FsMapper {
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

#[cfg(test)]
mod fs_cov_tests {
    use crate::session::fs::*;
    use crate::session::ports::{AutoSaveBackend, MapperSaveBackend, SlotBackend};
    use crate::test_helpers::DummySystemId;

    fn identity() -> SystemIdentity {
        SystemIdentity::new(Box::new(DummySystemId), vec![1, 2])
    }

    /// Each narrow leg serves its own round-trip.
    #[test]
    fn narrow_slots_leg_round_trips() {
        let back = FsSlots;
        let d = dir("narrow-slots");
        let id = identity();
        let next = back.allocate_next_id(&d).unwrap();
        let summary = back.write_slot(&d, next, &[7, 8, 9], &id, None).unwrap();
        assert_eq!(summary.slot_id, next);
        assert!(
            back.scan(&d, &id)
                .unwrap()
                .iter()
                .any(|s| s.slot_id == next)
        );
        assert!(back.read_slot(&d, next).unwrap().is_some());
        back.delete_slot(&d, next).unwrap();
        assert!(back.read_slot(&d, next).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Each narrow leg serves its own round-trip.
    #[test]
    fn narrow_autosave_leg_round_trips() {
        let back = FsAutosave;
        let d = dir("narrow-autosave");
        let id = identity();
        back.delete_autosave(&d).ok();
        assert_eq!(back.quarantine_autosave(&d).unwrap(), None);
        back.write_autosave(&d, &[1, 2, 3], &id).unwrap();
        assert!(back.read_autosave(&d, &id).unwrap().is_some());
        assert!(back.quarantine_autosave(&d).unwrap().is_some());
        assert!(back.read_autosave(&d, &id).unwrap_or(None).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Each narrow leg serves its own round-trip.
    #[test]
    fn narrow_mapper_leg_round_trips() {
        let back = FsMapper;
        let d = dir("narrow-mapper");
        let path = d.join("game.sav");
        assert_eq!(back.read_mapper_save(&path).unwrap(), None);
        back.write_mapper_save(&path, &[4, 5]).unwrap();
        assert_eq!(back.read_mapper_save(&path).unwrap(), Some(vec![4, 5]));
        let recovery = back.write_recovery_mapper_save(&path, &[6]).unwrap();
        assert!(recovery.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Atomic quarantine maps a missing source to `None` without probing first.
    #[test]
    fn quarantine_missing_source_is_none_without_probe() {
        let d = dir("quarantine-missing");
        assert_eq!(FsAutosave.quarantine_autosave(&d).unwrap(), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    fn dir(label: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "nerust-fs-cov-{}-{}",
            label,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&base).expect("temp dir");
        base
    }

    #[test]
    fn failing_backend_errors_every_leg() {
        let back = FailingSlotBackend;
        let dir = Path::new("/nonexistent-probe-dir");
        let id = identity();
        assert!(back.scan(dir, &id).is_err());
        assert!(back.allocate_next_id(dir).is_err());
        assert!(back.write_slot(dir, 1, &[], &id, None).is_err());
        assert!(back.read_slot(dir, 1).is_err());
        assert!(back.delete_slot(dir, 1).is_err());
        assert!(back.write_autosave(dir, &[], &id).is_err());
        assert!(back.read_autosave(dir, &id).is_err());
        assert!(back.delete_autosave(dir).is_err());
        assert!(back.read_mapper_save(dir).is_err());
        assert!(back.write_mapper_save(dir, &[]).is_err());
        assert!(back.write_recovery_mapper_save(dir, &[]).is_err());
    }

    #[test]
    fn read_missing_slot_returns_none() {
        let back = FsSlots;
        let d = dir("missing");
        assert!(back.read_slot(&d, 999).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
