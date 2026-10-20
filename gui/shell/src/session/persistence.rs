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
use crate::emu_core::{EmuCore, LivePersistPort};

pub use super::fs::{FailingSlotBackend, FsSlotBackend};
pub use super::ports::{
    AutoSaveBackend, MapperSaveBackend, PersistenceBackends, SlotActivation, SlotBackend,
    StateExporter,
};

/// Session-side adapter: narrow persist port plus fault mapping.
impl StateExporter for EmuCore {
    fn export_state(&self) -> Result<Vec<u8>, SlotOpFailure> {
        LivePersistPort::save_state_raw(self).map_err(|error| SlotOpFailure::from(error.fault()))
    }

    fn import_state(&self, data: Vec<u8>) -> Result<(), SlotOpFailure> {
        LivePersistPort::load_state_raw(self, data)
            .map_err(|error| SlotOpFailure::from(error.fault()))
    }

    fn export_preview(&self) -> Option<nerust_persistence::thumbnail::ThumbnailSource> {
        LivePersistPort::generate_preview(self).map(|frame| {
            nerust_persistence::thumbnail::ThumbnailSource {
                width: frame.width,
                height: frame.height,
                rgba: frame.rgba,
            }
        })
    }

    fn export_identity(&self) -> Option<SystemIdentity> {
        LivePersistPort::canonical_media_identity(self)
    }

    fn export_mapper(&self) -> Result<Option<Vec<u8>>, SlotOpFailure> {
        LivePersistPort::save_mapper_raw(self).map_err(|error| SlotOpFailure::from(error.fault()))
    }

    fn import_mapper(&self, bytes: Vec<u8>) -> Result<(), SlotOpFailure> {
        LivePersistPort::load_mapper_raw(self, bytes)
            .map_err(|error| SlotOpFailure::from(error.fault()))
    }
}

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
        Self {
            slot_backend: Box::new(FsSlotBackend),
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
            return Err(SlotOpFailure::Unavailable);
        };
        let Some(identity) = emu.export_identity() else {
            return Err(SlotOpFailure::Unavailable);
        };

        let core_bytes = emu.export_state()?;
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

                Ok(())
            }
            Err(error) => Err(classify(&error)),
        }
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

    /// Load a slot through `emu`; stored bytes reach the core via the caller-supplied `resolve`.
    /// The envelope lives at the session boundary, never in this module.
    pub fn load_slot(
        &mut self,
        slot_id: u64,
        emu: &dyn StateExporter,
        resolve: fn(&[u8]) -> Vec<u8>,
    ) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
            return Err(SlotOpFailure::Unavailable);
        };
        match self.slot_backend.read_slot(dir, slot_id) {
            Ok(Some(slot)) => {
                if let Err(failure) = emu.import_state(resolve(&slot.machine_state)) {
                    Err(failure)
                } else {
                    self.active_slot_id = Some(slot_id);
                    let identity = emu.export_identity();
                    self.refresh_slots_inner(identity.as_ref());
                    Ok(())
                }
            }
            Ok(None) => Err(SlotOpFailure::Missing),
            Err(error) => Err(classify(&error)),
        }
    }

    pub fn load_active_slot(
        &mut self,
        emu: &dyn StateExporter,
        resolve: fn(&[u8]) -> Vec<u8>,
    ) -> Result<(), SlotOpFailure> {
        match self.active_slot_id {
            Some(slot_id) => self.load_slot(slot_id, emu, resolve),
            None => Err(SlotOpFailure::Empty),
        }
    }

    pub fn delete_slot(
        &mut self,
        slot_id: u64,
        emu: &dyn StateExporter,
    ) -> Result<(), SlotOpFailure> {
        let Some(dir) = self.states_dir.as_ref() else {
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
            if let Some(bytes) = emu.export_mapper().ok().flatten() {
                let _recovery_path = self
                    .mapper_backend
                    .write_recovery_mapper_save(path, &bytes)?;
                self.mapper_save_recovery_written = true;

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
                let _ = emu.import_mapper(bytes);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(_) => {
                self.mapper_save_flush_allowed = false;

                Ok(())
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
                let state_blob = save_state_with_header(core_bytes);
                self.autosave_backend
                    .write_autosave(dir, &state_blob, &identity)
                    .is_ok()
            }
            Err(_) => false,
        }
    }

    /// Load the hidden autosave through `emu`. Stored bytes reach the
    /// core through `resolve` (see [`load_slot`](Self::load_slot)).
    pub fn load_hidden(&self, emu: &dyn StateExporter, resolve: fn(&[u8]) -> Vec<u8>) -> bool {
        let Some(dir) = self.states_dir.as_ref() else {
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
                    let _ = self.autosave_backend.quarantine_autosave(dir);
                    return false;
                }
                if emu.import_state(resolve(&slot.machine_state)).is_err() {
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
