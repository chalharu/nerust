//! Slot persistence orchestration: save/load/delete plus mapper-save flushing.
//! Ports live in [`ports`](super::ports); the filesystem backend in [`fs`](super::fs).

use std::{io::ErrorKind, path::PathBuf};

use nerust_core_traits::{
    identity::SystemIdentity,
    save_state::{SaveStateHeader, save_state_with_header},
};
use nerust_persistence::{
    error::PersistenceError, model::StateSlotSummary, time::latest_saved_slot_id,
};

use super::error::SlotOpFailure;

pub use super::fs::{FailingSlotBackend, FsSlotBackend};
pub use super::ports::{
    AutoSaveBackend, MapperSaveBackend, PersistenceBackends, SlotActivation, SlotBackend,
    StateExporter,
};

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
    states_dir: Option<PathBuf>,
    mapper_save_path: Option<PathBuf>,
    mapper_save_flush_allowed: bool,
    mapper_save_recovery_written: bool,
    slots: Vec<StateSlotSummary>,
    active_slot_id: Option<u64>,
}

impl Default for PersistenceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PersistenceManager {
    pub fn new() -> Self {
        Self::with_backend(Box::new(FsSlotBackend))
    }

    pub fn with_backend(backend: Box<dyn SlotBackend>) -> Self {
        Self {
            slot_backend: backend,
            autosave_backend: Box::new(FsSlotBackend),
            mapper_backend: Box::new(FsSlotBackend),
            states_dir: None,
            mapper_save_path: None,
            mapper_save_flush_allowed: true,
            mapper_save_recovery_written: false,
            slots: Vec::new(),
            active_slot_id: None,
        }
    }

    /// Bundle form of the backend swap: one struct instead of three
    /// positional boxes. Keeps the configured paths and slot index.
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
        Self {
            slot_backend,
            autosave_backend,
            mapper_backend,
            states_dir: None,
            mapper_save_path: None,
            mapper_save_flush_allowed: true,
            mapper_save_recovery_written: false,
            slots: Vec::new(),
            active_slot_id: None,
        }
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

    pub fn active_slot_id(&self) -> Option<u64> {
        self.active_slot_id
    }

    /// Configured save locations, if setup resolved them: cloned paths for the session boundary.
    pub(crate) fn configured_sidecars(&self) -> Option<(PathBuf, PathBuf)> {
        match (&self.states_dir, &self.mapper_save_path) {
            (Some(states), Some(mapper)) => Some((states.clone(), mapper.clone())),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn states_dir(&self) -> Option<&PathBuf> {
        self.states_dir.as_ref()
    }

    #[cfg(test)]
    pub fn mapper_save_path(&self) -> Option<&PathBuf> {
        self.mapper_save_path.as_ref()
    }

    pub fn save_slot(
        &mut self,
        slot_id: u64,
        emu: &dyn StateExporter,
        activation: SlotActivation,
    ) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("save_slot: no states_dir configured; cannot save slot {slot_id}");
            return Err(SlotOpFailure::Unavailable);
        };
        let Some(identity) = emu.export_identity() else {
            log::warn!("save_slot: no persistence identity available; cannot save slot {slot_id}");
            return Err(SlotOpFailure::Unavailable);
        };
        log::info!(
            "save_slot: writing slot {slot_id} (activation={activation:?}) to {}",
            dir.display()
        );
        let core_bytes = emu.export_state().inspect_err(|failure| {
            log::warn!("state export failed: {failure:?}");
        })?;
        let state_blob = save_state_with_header(core_bytes);
        let preview = emu.export_preview();
        match self
            .slot_backend
            .write_slot(dir, slot_id, &state_blob, &identity, preview.as_ref())
        {
            Ok(_summary) => {
                if activation == SlotActivation::Activate {
                    self.active_slot_id = Some(slot_id);
                }
                self.refresh_slots_inner(Some(&identity));
                log::info!("save_slot: saved slot {slot_id}");
                Ok(())
            }
            Err(error) => {
                log::warn!("saving state slot failed: {error}");
                Err(classify(&error))
            }
        }
    }

    pub fn save_active_slot_or_new(
        &mut self,
        emu: &dyn StateExporter,
    ) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("save_active_slot_or_new: no states_dir configured; cannot save state");
            return Err(SlotOpFailure::Unavailable);
        };
        let slot_id = match self.active_slot_id {
            Some(slot_id) => slot_id,
            None => match self.slot_backend.allocate_next_id(dir) {
                Ok(slot_id) => slot_id,
                Err(error) => {
                    log::warn!("allocating state slot failed: {error}");
                    return Err(classify(&error));
                }
            },
        };
        log::info!("save_active_slot_or_new: saving to slot {slot_id}");
        self.save_slot(slot_id, emu, SlotActivation::Activate)
    }

    pub fn create_slot(&mut self, emu: &dyn StateExporter) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("create_slot: no states_dir configured; cannot create slot");
            return Err(SlotOpFailure::Unavailable);
        };
        match self.slot_backend.allocate_next_id(dir) {
            Ok(slot_id) => self.save_slot(slot_id, emu, SlotActivation::Activate),
            Err(error) => {
                log::warn!("allocating state slot failed: {error}");
                Err(classify(&error))
            }
        }
    }

    /// Load a slot through `emu`; stored bytes reach the core via the caller-supplied `resolve`.
    /// The envelope lives at the session boundary, never in this module.
    pub fn load_slot(
        &mut self,
        slot_id: u64,
        emu: &dyn StateExporter,
        resolve: fn(&[u8]) -> Vec<u8>,
    ) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("load_slot: no states_dir configured; cannot load slot {slot_id}");
            return Err(SlotOpFailure::Unavailable);
        };
        match self.slot_backend.read_slot(dir, slot_id) {
            Ok(Some(slot)) => {
                if let Err(failure) = emu.import_state(resolve(&slot.machine_state)) {
                    log::warn!("state import failed: {failure:?}");
                    Err(failure)
                } else {
                    self.active_slot_id = Some(slot_id);
                    let identity = emu.export_identity();
                    self.refresh_slots_inner(identity.as_ref());
                    Ok(())
                }
            }
            Ok(None) => {
                log::warn!("load_slot: slot {slot_id} not found in {}", dir.display());
                Err(SlotOpFailure::Missing)
            }
            Err(error) => {
                log::warn!("loading state slot failed: {error}");
                Err(classify(&error))
            }
        }
    }

    pub fn load_active_slot(
        &mut self,
        emu: &dyn StateExporter,
        resolve: fn(&[u8]) -> Vec<u8>,
    ) -> Result<(), SlotOpFailure> {
        match self.active_slot_id {
            Some(slot_id) => self.load_slot(slot_id, emu, resolve),
            None => {
                log::debug!("load_active_slot: no active slot");
                Err(SlotOpFailure::Empty)
            }
        }
    }

    pub fn delete_slot(
        &mut self,
        slot_id: u64,
        emu: &dyn StateExporter,
    ) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("delete_slot: no states_dir configured; cannot delete slot {slot_id}");
            return Err(SlotOpFailure::Unavailable);
        };
        match self.slot_backend.delete_slot(dir, slot_id) {
            Ok(()) => {
                if self.active_slot_id == Some(slot_id) {
                    self.active_slot_id = None;
                }
                let identity = emu.export_identity();
                self.refresh_slots_inner(identity.as_ref());
                Ok(())
            }
            Err(error) => {
                log::warn!("deleting state slot failed: {error}");
                Err(classify(&error))
            }
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
            match self.slot_backend.scan(dir, identity) {
                Ok(slots) => slots,
                Err(error) => {
                    log::warn!("slot scan failed: {error}");
                    Vec::new()
                }
            }
        } else {
            log::warn!("states_dir not configured, slot list unavailable");
            Vec::new()
        };
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

    pub fn configure(&mut self, states_dir: PathBuf, mapper_save_path: PathBuf) {
        self.states_dir = Some(states_dir);
        self.mapper_save_path = Some(mapper_save_path);
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.active_slot_id = None;
    }

    pub fn reset(&mut self) {
        self.states_dir = None;
        self.mapper_save_path = None;
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.slots = Vec::new();
        self.active_slot_id = None;
    }

    pub fn flush_mapper_save(
        &mut self,
        emu: &dyn StateExporter,
    ) -> Result<MapperFlush, PersistenceError> {
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
                .inspect_err(|failure| {
                    log::warn!("mapper save failed: {failure:?}");
                })
                .ok()
                .flatten()
            {
                let recovery_path = self
                    .mapper_backend
                    .write_recovery_mapper_save(path, &bytes)?;
                self.mapper_save_recovery_written = true;
                log::warn!(
                    "mapper save auto-load failed earlier; wrote recovery save to {}",
                    recovery_path.display()
                );
                return Ok(MapperFlush::DegradedRecoveryWritten);
            }
            return Ok(MapperFlush::SkippedNoPath);
        }
        let flushed = match emu.export_mapper() {
            Ok(Some(bytes)) => self
                .mapper_backend
                .write_mapper_save(path, &bytes)
                .map(|_| MapperFlush::Flushed)?,
            Ok(None) => MapperFlush::Flushed,
            Err(failure) => {
                // Fail loudly: callers propagate this to the UI instead
                // of silently dropping the in-core battery save.
                let error = PersistenceError::Export(format!("{failure:?}"));
                log::warn!("{error}");
                return Err(error);
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
            Ok(Some(bytes)) => {
                if let Err(failure) = emu.import_mapper(bytes) {
                    log::warn!("mapper save load failed: {failure:?}");
                }
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(error) => {
                self.mapper_save_flush_allowed = false;
                log::warn!("mapper save auto-load failed: {error}");
                Ok(())
            }
        }
    }

    pub fn save_hidden(&self, emu: &dyn StateExporter) -> bool {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("save_hidden: no states_dir configured; cannot save hidden lifecycle state");
            return false;
        };
        let Some(identity) = emu.export_identity() else {
            return false;
        };
        match emu.export_state() {
            Ok(core_bytes) => {
                let state_blob = save_state_with_header(core_bytes);
                match self
                    .autosave_backend
                    .write_autosave(dir, &state_blob, &identity)
                {
                    Ok(_) => true,
                    Err(error) => {
                        log::warn!("saving hidden lifecycle state failed: {error}");
                        false
                    }
                }
            }
            Err(failure) => {
                log::warn!("hidden lifecycle state export failed: {failure:?}");
                false
            }
        }
    }

    /// Load the hidden autosave through `emu`. Stored bytes reach the
    /// core through `resolve` (see [`load_slot`](Self::load_slot)).
    pub fn load_hidden(&self, emu: &dyn StateExporter, resolve: fn(&[u8]) -> Vec<u8>) -> bool {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("load_hidden: no states_dir configured; cannot load hidden lifecycle state");
            return false;
        };
        let Some(identity) = emu.canonical_media_identity() else {
            return false;
        };
        match self.autosave_backend.read_autosave(dir, &identity) {
            Ok(Some(slot)) => {
                if state_header_version(&slot.machine_state)
                    != Some(SaveStateHeader::new(&[]).version)
                {
                    log::warn!("ignoring hidden lifecycle state with unsupported state version");
                    if let Err(error) = self.autosave_backend.quarantine_autosave(dir) {
                        log::warn!("failed to quarantine hidden lifecycle state: {error}");
                    }
                    return false;
                }
                if let Err(failure) = emu.import_state(resolve(&slot.machine_state)) {
                    log::warn!("hidden lifecycle state import failed: {failure:?}");
                    if let Err(error) = self.autosave_backend.quarantine_autosave(dir) {
                        log::warn!("failed to quarantine hidden lifecycle state: {error}");
                    }
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
            Err(error) => {
                log::warn!("loading hidden lifecycle state failed: {error}");
                let _ = self.autosave_backend.delete_autosave(dir);
                false
            }
        }
    }

    pub fn clear_hidden(&mut self) {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!(
                "clear_hidden: no states_dir configured; cannot clear hidden lifecycle state"
            );
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
