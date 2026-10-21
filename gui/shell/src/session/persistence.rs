//! Slot persistence orchestration: save/load/delete plus mapper-save flushing.
//! Ports live in [`ports`](super::ports); the filesystem backend in [`fs`](super::fs).

use std::{io::ErrorKind, sync::Arc};

use nerust_core_traits::{
    identity::SystemIdentity,
    save_state::{SaveStateHeader, save_state_with_header},
};
use nerust_persistence::{
    error::PersistenceError, model::StateSlotSummary, sidecar::StorageLocation,
    time::latest_saved_slot_id,
};

use super::error::SlotOpFailure;

pub use super::fs::{FailingSlotBackend, FsAutosave, FsMapper, FsSlots};
pub use super::ports::{
    AutoSaveBackend, MapperSaveBackend, PersistenceBackends, SlotActivation, SlotBackend,
    StateExporter,
};

/// Typed mapper-flush failure: the fault travels typed, detail renders once at the UI boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapperFlushError {
    /// Classified fault for UI text and tests.
    pub fault: SlotOpFailure,
}

impl std::fmt::Display for MapperFlushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mapper save export failed: {:?}", self.fault)
    }
}

impl std::error::Error for MapperFlushError {}

/// Classify a persistence-layer failure for user-facing messages.
/// Single conversion point; see [`SlotOpFailure`].
fn classify(error: &PersistenceError) -> SlotOpFailure {
    SlotOpFailure::from(error)
}

/// Outcome of a mapper-save flush. Callers distinguish "nothing
/// to do" from "wrote" without re-reading manager state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapperFlush {
    /// Battery save exported and written to the sidecar path.
    Flushed,
    /// Auto-load failed earlier, so the flush was degraded: the
    /// in-core save went to a recovery file instead.
    DegradedRecoveryWritten,
    /// No sidecar path configured (ROM-less switch or reset state).
    SkippedNoPath,
}

pub struct PersistenceManager {
    slot_backend: Box<dyn SlotBackend>,
    autosave_backend: Box<dyn AutoSaveBackend>,
    mapper_backend: Box<dyn MapperSaveBackend>,
    states_dir: Option<StorageLocation>,
    mapper_save_path: Option<StorageLocation>,
    mapper_save_flush_allowed: bool,
    mapper_save_recovery_written: bool,
    slots: Arc<[StateSlotSummary]>,
    active_slot_id: Option<u64>,
}

impl Default for PersistenceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// UI-thread state budget: one envelope round-trip per slot op.
/// Oversized exports are corrupt payloads; saves refuse them in all builds.
pub const MAX_SLOT_BLOB_BYTES: usize = 64 << 20;

/// Refuse an oversized core export before it reaches the slot backend.
fn check_blob_budget(core_bytes: &[u8]) -> Result<(), SlotOpFailure> {
    if core_bytes.len() > MAX_SLOT_BLOB_BYTES {
        log::warn!(
            "slot blob exceeds UI-thread budget: {} bytes",
            core_bytes.len()
        );
        return Err(SlotOpFailure::Corrupt);
    }
    Ok(())
}

impl PersistenceManager {
    pub fn new() -> Self {
        Self {
            slot_backend: Box::new(FsSlots),
            autosave_backend: Box::new(FsAutosave),
            mapper_backend: Box::new(FsMapper),
            states_dir: None,
            mapper_save_path: None,
            mapper_save_flush_allowed: true,
            mapper_save_recovery_written: false,
            slots: Arc::from([]),
            active_slot_id: None,
        }
    }

    pub fn with_backend(backend: Box<dyn SlotBackend>) -> Self {
        let mut manager = Self::new();
        manager.slot_backend = backend;
        manager
    }

    /// Single wiring path: every constructor funnels through here.
    pub fn configure_backends(&mut self, backends: PersistenceBackends) {
        self.slot_backend = backends.slot;
        self.autosave_backend = backends.autosave;
        self.mapper_backend = backends.mapper;
    }

    pub fn with_all_backends(
        slot_backend: Box<dyn SlotBackend>,
        autosave_backend: Box<dyn AutoSaveBackend>,
        mapper_backend: Box<dyn MapperSaveBackend>,
    ) -> Self {
        let mut manager = Self::new();
        manager.configure_backends(PersistenceBackends {
            slot: slot_backend,
            autosave: autosave_backend,
            mapper: mapper_backend,
        });
        manager
    }

    pub fn with_autosave_backend(&mut self, backend: Box<dyn AutoSaveBackend>) {
        self.autosave_backend = backend;
    }

    pub fn with_mapper_backend(&mut self, backend: Box<dyn MapperSaveBackend>) {
        self.mapper_backend = backend;
    }

    pub fn slots(&self) -> &[StateSlotSummary] {
        &self.slots
    }

    pub fn slots_arc(&self) -> Arc<[StateSlotSummary]> {
        Arc::clone(&self.slots)
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.active_slot_id
    }

    /// Configured save locations, if setup resolved them: cloned paths for the session boundary.
    pub(crate) fn configured_sidecars(&self) -> Option<(StorageLocation, StorageLocation)> {
        match (&self.states_dir, &self.mapper_save_path) {
            (Some(states), Some(mapper)) => Some((states.clone(), mapper.clone())),
            _ => None,
        }
    }

    pub fn save_slot(
        &mut self,
        slot_id: u64,
        emu: &dyn StateExporter,
        activation: SlotActivation,
    ) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            return Err(SlotOpFailure::Unavailable);
        };
        let Some(identity) = emu.export_identity() else {
            return Err(SlotOpFailure::Unavailable);
        };

        let core_bytes = emu.export_state()?;
        check_blob_budget(&core_bytes)?;
        let state_blob = save_state_with_header(core_bytes);
        let preview =
            emu.export_preview()
                .map(|frame| nerust_persistence::thumbnail::ThumbnailSource {
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.rgba,
                });
        match self
            .slot_backend
            .write_slot(dir, slot_id, &state_blob, &identity, preview.as_ref())
        {
            Ok(summary) => {
                if activation == SlotActivation::Activate {
                    self.active_slot_id = Some(slot_id);
                }
                // Use the write summary; no re-scan on the mutation path.
                self.upsert_summary(summary);
                if self
                    .active_slot_id
                    .is_some_and(|active| !self.slots.iter().any(|slot| slot.slot_id == active))
                {
                    self.active_slot_id = None;
                }
                if self.active_slot_id.is_none() {
                    self.active_slot_id = latest_saved_slot_id(&self.slots);
                }

                Ok(())
            }
            Err(error) => Err(classify(&error)),
        }
    }

    /// Publish one immutable revision after a successful write.
    fn upsert_summary(&mut self, summary: StateSlotSummary) {
        let mut slots = self.slots.to_vec();
        if let Some(slot) = slots.iter_mut().find(|s| s.slot_id == summary.slot_id) {
            *slot = summary;
        } else {
            slots.push(summary);
            slots.sort_by_key(|slot| slot.slot_id);
        }
        self.slots = slots.into();
    }

    pub fn save_active_slot_or_new(
        &mut self,
        emu: &dyn StateExporter,
    ) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            return Err(SlotOpFailure::Unavailable);
        };
        let slot_id = match self.active_slot_id {
            Some(slot_id) => slot_id,
            None => match self.slot_backend.allocate_next_id(dir) {
                Ok(slot_id) => slot_id,
                Err(error) => {
                    return Err(classify(&error));
                }
            },
        };

        self.save_slot(slot_id, emu, SlotActivation::Activate)
    }

    pub fn create_slot(&mut self, emu: &dyn StateExporter) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            return Err(SlotOpFailure::Unavailable);
        };
        match self.slot_backend.allocate_next_id(dir) {
            Ok(slot_id) => self.save_slot(slot_id, emu, SlotActivation::Activate),
            Err(error) => Err(classify(&error)),
        }
    }

    /// Load a slot through `emu`; stored bytes reach the core via the exporter's `resolve_format`.
    /// The envelope lives at the session boundary, never in this module.
    pub fn load_slot(
        &mut self,
        slot_id: u64,
        emu: &dyn StateExporter,
    ) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            return Err(SlotOpFailure::Unavailable);
        };
        match self.slot_backend.read_slot(dir, slot_id) {
            Ok(Some(slot)) => {
                if let Err(failure) = emu.import_state(emu.resolve_format(&slot.machine_state)) {
                    Err(failure)
                } else {
                    // Load changes no files: point active, no re-scan.
                    self.active_slot_id = Some(slot_id);
                    Ok(())
                }
            }
            Ok(None) => Err(SlotOpFailure::Missing),
            Err(error) => Err(classify(&error)),
        }
    }

    pub fn load_active_slot(&mut self, emu: &dyn StateExporter) -> Result<(), SlotOpFailure> {
        match self.active_slot_id {
            Some(slot_id) => self.load_slot(slot_id, emu),
            None => Err(SlotOpFailure::Empty),
        }
    }

    pub fn delete_slot(&mut self, slot_id: u64) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            return Err(SlotOpFailure::Unavailable);
        };
        match self.slot_backend.delete_slot(dir, slot_id) {
            Ok(()) => {
                if self.active_slot_id == Some(slot_id) {
                    self.active_slot_id = None;
                }
                // Prune the cache; no re-scan on the mutation path.
                self.slots = self
                    .slots
                    .iter()
                    .filter(|slot| slot.slot_id != slot_id)
                    .cloned()
                    .collect();
                Ok(())
            }
            Err(error) => Err(classify(&error)),
        }
    }

    pub fn select_active_slot(&mut self, slot_id: u64) {
        self.active_slot_id = Some(slot_id);
    }

    pub fn select_adjacent_slot(&mut self, direction: SlotDirection) -> Option<u64> {
        let next_slot_id = adjacent_slot_id(&self.slots, self.active_slot_id, direction)?;
        self.active_slot_id = Some(next_slot_id);
        Some(next_slot_id)
    }

    pub fn refresh_slots(&mut self, emu: &dyn StateExporter) {
        let identity = emu.export_identity();
        self.refresh_slots_inner(identity.as_ref());
    }

    fn refresh_slots_inner(&mut self, identity: Option<&SystemIdentity>) {
        self.slots = if let (Some(dir), Some(identity)) = (self.states_dir.as_ref(), identity) {
            self.slot_backend.scan(dir, identity).unwrap_or_default()
        } else {
            Vec::new()
        }
        .into();
        if self
            .active_slot_id
            .is_some_and(|slot_id| !self.slots.iter().any(|slot| slot.slot_id == slot_id))
        {
            self.active_slot_id = None;
        }
        if self.active_slot_id.is_none() {
            self.active_slot_id = latest_saved_slot_id(&self.slots);
        }
    }

    pub fn configure(&mut self, states_dir: StorageLocation, mapper_save_path: StorageLocation) {
        if self.mapper_save_path.as_ref() != Some(&mapper_save_path) {
            self.mapper_save_flush_allowed = true;
            self.mapper_save_recovery_written = false;
        }
        if self.states_dir.as_ref() != Some(&states_dir) {
            self.active_slot_id = None;
            self.slots = Arc::from([]);
        }
        self.states_dir = Some(states_dir);
        self.mapper_save_path = Some(mapper_save_path);
    }

    pub fn reset(&mut self) {
        self.states_dir = None;
        self.mapper_save_path = None;
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.slots = Arc::from([]);
        self.active_slot_id = None;
    }

    pub fn flush_mapper_save(
        &mut self,
        emu: &dyn StateExporter,
    ) -> Result<MapperFlush, MapperFlushError> {
        // No path means no save location exists, not a failure: return Skipped, not an error.
        let Some(path) = self.mapper_save_path.as_ref() else {
            return Ok(MapperFlush::SkippedNoPath);
        };
        if !self.mapper_save_flush_allowed {
            if self.mapper_save_recovery_written {
                return Ok(MapperFlush::DegradedRecoveryWritten);
            }
            if let Some(bytes) = emu
                .export_mapper()
                .map_err(|fault| MapperFlushError { fault })?
            {
                let _recovery_path = self
                    .mapper_backend
                    .write_recovery_mapper_save(path, &bytes)
                    .map_err(|_| MapperFlushError {
                        fault: SlotOpFailure::Storage,
                    })?;
                self.mapper_save_recovery_written = true;

                return Ok(MapperFlush::DegradedRecoveryWritten);
            }
            return Ok(MapperFlush::SkippedNoPath);
        }
        let flushed = match emu.export_mapper() {
            Ok(Some(bytes)) => self
                .mapper_backend
                .write_mapper_save(path, &bytes)
                .map(|_| MapperFlush::Flushed)
                .map_err(|_| MapperFlushError {
                    fault: SlotOpFailure::Storage,
                })?,
            Ok(None) => MapperFlush::Flushed,
            Err(fault) => {
                // Fail loudly with the typed fault: callers render it at the UI boundary.
                return Err(MapperFlushError { fault });
            }
        };
        Ok(flushed)
    }

    pub fn load_mapper_save_if_needed(
        &mut self,
        emu: &dyn StateExporter,
    ) -> Result<(), PersistenceError> {
        let Some(path) = self.mapper_save_path.as_ref() else {
            return Ok(());
        };
        match self.mapper_backend.read_mapper_save(path) {
            Ok(Some(bytes)) => match emu.import_mapper(bytes) {
                Ok(()) => Ok(()),
                Err(fault) => {
                    self.mapper_save_flush_allowed = false;
                    Err(PersistenceError::Validation(format!(
                        "mapper import failed: {fault:?}"
                    )))
                }
            },
            Ok(None) => Ok(()),
            Err(error) => {
                self.mapper_save_flush_allowed = false;
                Err(error)
            }
        }
    }

    pub fn save_hidden(&self, emu: &dyn StateExporter) -> bool {
        let Some(dir) = self.states_dir.as_ref() else {
            return false;
        };
        let Some(identity) = emu.export_identity() else {
            return false;
        };
        match emu.export_state() {
            Ok(core_bytes) => {
                if check_blob_budget(&core_bytes).is_err() {
                    return false;
                }
                let state_blob = save_state_with_header(core_bytes);
                self.autosave_backend
                    .write_autosave(dir, &state_blob, &identity)
                    .is_ok()
            }
            Err(_) => false,
        }
    }

    /// Load the hidden autosave through `emu`. Stored bytes reach the
    /// core through the exporter's `resolve_format`.
    pub fn load_hidden(&self, emu: &dyn StateExporter) -> bool {
        let Some(dir) = self.states_dir.as_ref() else {
            return false;
        };
        let Some(identity) = emu.export_identity() else {
            return false;
        };
        match self.autosave_backend.read_autosave(dir, &identity) {
            Ok(Some(slot)) => {
                if state_header_version(&slot.machine_state)
                    != Some(SaveStateHeader::new(&[]).version)
                {
                    let _ = self.autosave_backend.quarantine_autosave(dir);
                    return false;
                }
                if emu
                    .import_state(emu.resolve_format(&slot.machine_state))
                    .is_err()
                {
                    let _ = self.autosave_backend.quarantine_autosave(dir);
                    false
                } else {
                    true
                }
            }
            Ok(None) => {
                let _ = self.autosave_backend.delete_autosave(dir);
                false
            }
            Err(PersistenceError::Io(error)) if error.kind() == ErrorKind::NotFound => false,
            Err(_) => {
                let _ = self.autosave_backend.delete_autosave(dir);
                false
            }
        }
    }

    pub fn clear_hidden(&mut self) {
        let Some(dir) = self.states_dir.as_ref() else {
            return;
        };
        let _ = self.autosave_backend.delete_autosave(dir);
    }
}

/// Decode the save-state envelope version from a stored blob.
/// `None` means pre-header (or corrupt) data, which never matches the current version.
fn state_header_version(blob: &[u8]) -> Option<u32> {
    // 12-byte `SaveStateHeader` (`NRST` magic + version + size).
    let buf: &[u8; 12] = blob.get(..12)?.try_into().ok()?;
    SaveStateHeader::decode(buf)
        .ok()
        .map(|header| header.version)
}

/// Slot navigation direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotDirection {
    Next,
    Previous,
}

pub fn adjacent_slot_id(
    slots: &[StateSlotSummary],
    active_slot_id: Option<u64>,
    direction: SlotDirection,
) -> Option<u64> {
    if slots.is_empty() {
        return None;
    }
    // No active slot: wrap around from the far end so the first step
    // lands on a real slot in either direction.
    let current_index = active_slot_id
        .and_then(|active| slots.iter().position(|slot| slot.slot_id == active))
        .unwrap_or_else(|| match direction {
            SlotDirection::Next => slots.len().saturating_sub(1),
            SlotDirection::Previous => 0,
        });
    let next_index = match direction {
        SlotDirection::Next => (current_index + 1) % slots.len(),
        SlotDirection::Previous if current_index == 0 => slots.len() - 1,
        SlotDirection::Previous => current_index - 1,
    };
    Some(slots[next_index].slot_id)
}

#[cfg(test)]
mod persistence_cov_tests {
    use crate::session::persistence::*;

    struct OkExporter {
        identity: Option<SystemIdentity>,
        state: Result<Vec<u8>, SlotOpFailure>,
        mapper: Result<Option<Vec<u8>>, SlotOpFailure>,
    }

    impl StateExporter for OkExporter {
        fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure> {
            self.state.clone()
        }
        fn import_state(&self, _: Vec<u8>) -> Result<(), SlotOpFailure> {
            self.state.clone().map(|_| ())
        }
        fn export_preview(&self) -> Option<crate::emu_core::PreviewRgba> {
            None
        }
        fn export_identity(&self) -> Option<SystemIdentity> {
            self.identity.clone()
        }
        fn export_mapper(&self) -> Result<Option<Vec<u8>>, SlotOpFailure> {
            self.mapper.clone()
        }
        fn import_mapper(&self, _: Vec<u8>) -> Result<(), SlotOpFailure> {
            Ok(())
        }
    }

    fn identity() -> SystemIdentity {
        SystemIdentity::new(Box::new(crate::test_helpers::DummySystemId), vec![9, 9])
    }

    fn failing_manager() -> PersistenceManager {
        PersistenceManager::with_all_backends(
            Box::new(FailingSlotBackend),
            Box::new(FailingSlotBackend),
            Box::new(FailingSlotBackend),
        )
    }

    fn exporter_ok() -> OkExporter {
        OkExporter {
            identity: Some(identity()),
            state: Ok(vec![1, 2, 3]),
            mapper: Ok(Some(vec![4])),
        }
    }

    #[test]
    fn unconfigured_manager_reports_unavailable() {
        let mut manager = PersistenceManager::new();
        let exporter = exporter_ok();
        assert_eq!(
            manager
                .save_slot(1, &exporter, SlotActivation::Activate)
                .unwrap_err(),
            SlotOpFailure::Unavailable
        );
        assert_eq!(
            manager.save_active_slot_or_new(&exporter).unwrap_err(),
            SlotOpFailure::Unavailable
        );
        assert_eq!(
            manager.create_slot(&exporter).unwrap_err(),
            SlotOpFailure::Unavailable
        );
        assert_eq!(
            manager.load_slot(1, &exporter).unwrap_err(),
            SlotOpFailure::Unavailable
        );
        assert_eq!(
            manager.load_active_slot(&exporter).unwrap_err(),
            SlotOpFailure::Empty
        );
        assert_eq!(
            manager.delete_slot(1).unwrap_err(),
            SlotOpFailure::Unavailable
        );
        assert_eq!(
            manager.flush_mapper_save(&exporter).unwrap(),
            MapperFlush::SkippedNoPath
        );
        assert!(manager.load_mapper_save_if_needed(&exporter).is_ok());
        assert!(!manager.save_hidden(&exporter));
        assert!(!manager.load_hidden(&exporter));
        manager.clear_hidden();
    }

    #[test]
    fn no_identity_reports_unavailable_when_configured() {
        let dir = std::env::temp_dir().join("nerust-persist-cov-noid");
        std::fs::create_dir_all(&dir).ok();
        let mut manager = PersistenceManager::new();
        manager.configure(dir.clone().into(), dir.join("g.sav").into());
        let exporter = OkExporter {
            identity: None,
            state: Ok(vec![1]),
            mapper: Ok(None),
        };
        assert_eq!(
            manager
                .save_slot(1, &exporter, SlotActivation::Leave)
                .unwrap_err(),
            SlotOpFailure::Unavailable
        );
        assert!(!manager.save_hidden(&exporter));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_failure_short_circuits_before_backend() {
        let dir = std::env::temp_dir().join("nerust-persist-cov-export");
        std::fs::create_dir_all(&dir).ok();
        let mut manager = PersistenceManager::new();
        manager.configure(dir.clone().into(), dir.join("g.sav").into());
        let exporter = OkExporter {
            identity: Some(identity()),
            state: Err(SlotOpFailure::Corrupt),
            mapper: Ok(None),
        };
        assert_eq!(
            manager
                .save_slot(1, &exporter, SlotActivation::Activate)
                .unwrap_err(),
            SlotOpFailure::Corrupt
        );
        assert!(!manager.save_hidden(&exporter));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mapper_flush_covers_degraded_and_skipped() {
        use crate::session::fs::FsMapper;
        let dir = std::env::temp_dir().join("nerust-persist-cov-mapper");
        std::fs::create_dir_all(&dir).ok();
        let mut manager = PersistenceManager::with_all_backends(
            Box::new(FailingSlotBackend),
            Box::new(FailingSlotBackend),
            Box::new(FsMapper),
        );
        manager.configure(dir.clone().into(), dir.join("g.sav").into());
        manager.mapper_save_flush_allowed = false;
        let exporter = exporter_ok();
        assert_eq!(
            manager.flush_mapper_save(&exporter).unwrap(),
            MapperFlush::DegradedRecoveryWritten
        );
        assert_eq!(
            manager.flush_mapper_save(&exporter).unwrap(),
            MapperFlush::DegradedRecoveryWritten
        );
        let none_mapper = OkExporter {
            identity: Some(identity()),
            state: Ok(vec![1]),
            mapper: Ok(None),
        };
        let mut fresh = failing_manager();
        fresh.configure(dir.clone().into(), dir.join("h.sav").into());
        fresh.mapper_save_flush_allowed = false;
        assert_eq!(
            fresh.flush_mapper_save(&none_mapper).unwrap(),
            MapperFlush::SkippedNoPath
        );
        assert!(fresh.load_mapper_save_if_needed(&exporter).is_err());
        assert!(!fresh.mapper_save_flush_allowed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adjacent_slot_id_covers_empty_wrap_and_step() {
        use nerust_persistence::model::StateSlotSummary as S;
        assert_eq!(adjacent_slot_id(&[], None, SlotDirection::Next), None);
        let mk = |id: u64| S {
            schema_version: 1,
            slot_id: id,
            path: std::path::PathBuf::from("/tmp/x").into(),
            saved_at: std::time::UNIX_EPOCH,
            has_thumbnail: false,
            emulator_version: String::new(),
        };
        let slots = vec![mk(1), mk(2), mk(3)];
        assert_eq!(adjacent_slot_id(&slots, None, SlotDirection::Next), Some(1));
        assert_eq!(
            adjacent_slot_id(&slots, None, SlotDirection::Previous),
            Some(3)
        );
        assert_eq!(
            adjacent_slot_id(&slots, Some(2), SlotDirection::Next),
            Some(3)
        );
        assert_eq!(
            adjacent_slot_id(&slots, Some(1), SlotDirection::Previous),
            Some(3)
        );
        assert_eq!(
            adjacent_slot_id(&slots, Some(99), SlotDirection::Next),
            Some(1)
        );
    }

    #[test]
    fn save_uses_write_summary_without_rescan() {
        use crate::session::fs::FsSlots;
        let dir = std::env::temp_dir().join("nerust-persist-cov-upsert");
        std::fs::create_dir_all(&dir).ok();
        let mut manager = PersistenceManager::new();
        manager.configure(dir.clone().into(), dir.join("g.sav").into());
        let exporter = exporter_ok();
        // Two saves: the cache upserts from the write summary both times.
        manager
            .save_slot(1, &exporter, SlotActivation::Activate)
            .expect("save ok");
        assert_eq!(manager.slots().len(), 1);
        assert_eq!(manager.active_slot_id(), Some(1));
        let first = manager.slots_arc();
        assert!(Arc::ptr_eq(&first, &manager.slots_arc()));
        manager
            .save_slot(1, &exporter, SlotActivation::Leave)
            .expect("resave ok");
        assert_eq!(manager.slots().len(), 1);
        assert!(!Arc::ptr_eq(&first, &manager.slots_arc()));
        // Delete prunes the cache without a backend scan.
        manager.delete_slot(1).expect("delete ok");
        assert!(manager.slots().is_empty());
        assert_eq!(first.len(), 1, "published revisions remain immutable");
        assert_eq!(manager.active_slot_id(), None);
        // Mapper-flush errors keep the typed fault for the UI boundary.
        let error = crate::session::persistence::MapperFlushError {
            fault: SlotOpFailure::Unavailable,
        };
        assert!(error.to_string().contains("Unavailable"));
        // StateExporter resolves the shell envelope by default.
        let resolved = exporter.resolve_format(&save_state_with_header(vec![1, 2, 3]));
        assert_eq!(resolved, vec![1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = FsSlots;
    }

    #[test]
    fn header_version_rejects_short_and_preheader_blobs() {
        assert_eq!(state_header_version(&[]), None);
        assert_eq!(state_header_version(&[0u8; 5]), None);
        assert_eq!(state_header_version(&[0xFF; 12]), None);
    }

    #[test]
    fn with_backend_replaces_only_slot_leg() {
        let mut manager = PersistenceManager::with_backend(Box::new(FailingSlotBackend));
        assert!(manager.slots().is_empty());
        assert_eq!(manager.active_slot_id(), None);
        assert_eq!(manager.configured_sidecars(), None);
        manager.configure(
            std::path::PathBuf::from("/tmp/a").into(),
            std::path::PathBuf::from("/tmp/b").into(),
        );
        assert!(manager.configured_sidecars().is_some());
        manager.reset();
        assert!(manager.configured_sidecars().is_none());
        let mut selected = PersistenceManager::new();
        selected.select_active_slot(5);
        assert_eq!(selected.active_slot_id(), Some(5));
        assert_eq!(selected.select_adjacent_slot(SlotDirection::Next), None);
    }

    #[test]
    fn refresh_and_select_prune_missing_active() {
        let mut manager = PersistenceManager::new();
        let exporter = exporter_ok();
        manager.refresh_slots(&exporter);
        assert!(manager.slots().is_empty());
        manager.select_active_slot(9);
        manager.refresh_slots(&exporter);
        assert_eq!(manager.active_slot_id(), None);
    }

    #[test]
    fn rejected_mapper_import_and_same_path_rebind_preserve_original() {
        struct RejectImport;
        impl StateExporter for RejectImport {
            fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure> {
                Ok(vec![])
            }
            fn import_state(&self, _: Vec<u8>) -> Result<(), SlotOpFailure> {
                Ok(())
            }
            fn export_preview(&self) -> Option<crate::emu_core::PreviewRgba> {
                None
            }
            fn export_identity(&self) -> Option<SystemIdentity> {
                None
            }
            fn export_mapper(&self) -> Result<Option<Vec<u8>>, SlotOpFailure> {
                Ok(Some(b"replacement".to_vec()))
            }
            fn import_mapper(&self, _: Vec<u8>) -> Result<(), SlotOpFailure> {
                Err(SlotOpFailure::Incompatible)
            }
        }
        let dir = crate::test_helpers::unique_temp_dir("rejected-mapper");
        let path = dir.join("game.sav");
        std::fs::write(&path, b"original").unwrap();
        let mut manager = PersistenceManager::new();
        manager.configure(dir.clone().into(), path.clone().into());
        assert!(manager.load_mapper_save_if_needed(&RejectImport).is_err());
        manager.configure(dir.clone().into(), path.clone().into());
        assert_eq!(
            manager.flush_mapper_save(&RejectImport).unwrap(),
            MapperFlush::DegradedRecoveryWritten
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        let recoveries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(Result::unwrap)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".recovered."))
            .collect();
        assert_eq!(recoveries.len(), 1);
        assert_eq!(std::fs::read(recoveries[0].path()).unwrap(), b"replacement");
        manager.configure(dir.clone().into(), path.into());
        manager.flush_mapper_save(&RejectImport).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod blob_budget_tests {
    use crate::session::error::SlotOpFailure;
    use crate::session::persistence::{MAX_SLOT_BLOB_BYTES, check_blob_budget};

    #[test]
    fn budget_accepts_at_cap_and_rejects_above() {
        assert!(check_blob_budget(&vec![0u8; MAX_SLOT_BLOB_BYTES]).is_ok());
        assert!(check_blob_budget(&[]).is_ok());
        assert_eq!(
            check_blob_budget(&vec![0u8; MAX_SLOT_BLOB_BYTES + 1]).unwrap_err(),
            SlotOpFailure::Corrupt
        );
    }
}
