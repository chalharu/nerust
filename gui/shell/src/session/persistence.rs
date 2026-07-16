use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use nerust_core_traits::{identity::SystemIdentity, save_state::save_state_with_header};
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
    time::latest_saved_slot_id,
};
use thiserror::Error;

use crate::state::resolve_state_format;

/// Errors from core operations invoked by the persistence layer.
#[derive(Debug, Error)]
pub(crate) enum CorePersistenceError {
    #[error("emu thread channel unavailable")]
    WorkerUnavailable,
    #[error("emu thread reply channel closed")]
    NoReply,
    #[error("{0}")]
    Core(String),
}

/// The persistence-relevant subset of EmuCore's interface.
pub(crate) trait CorePersistence {
    fn save_state_raw(&self) -> Result<Vec<u8>, CorePersistenceError>;
    fn load_state_raw(&self, data: Vec<u8>) -> Result<(), CorePersistenceError>;
    fn generate_preview(&self) -> Option<crate::state::PreviewFrame>;
    fn canonical_media_identity(&self) -> Option<SystemIdentity>;
    fn save_mapper_raw(&self) -> Result<Option<Vec<u8>>, CorePersistenceError>;
    fn load_mapper_raw(&self, bytes: Vec<u8>) -> Result<(), CorePersistenceError>;
}

/// Platform abstraction for all file I/O (Desktop fs / Android SAF).
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

    fn read_mapper_save(&self, path: &Path) -> Result<Option<Vec<u8>>, PersistenceError>;
    fn write_mapper_save(&self, path: &Path, data: &[u8]) -> Result<(), PersistenceError>;
    fn write_recovery_mapper_save(
        &self,
        path: &Path,
        data: &[u8],
    ) -> Result<PathBuf, PersistenceError>;
}

pub(super) struct FsSlotBackend;

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

pub(crate) struct PersistenceManager {
    backend: Box<dyn SlotBackend>,
    states_dir: Option<PathBuf>,
    mapper_save_path: Option<PathBuf>,
    mapper_save_flush_allowed: bool,
    mapper_save_recovery_written: bool,
    slots: Vec<StateSlotSummary>,
    active_slot_id: Option<u64>,
}

impl PersistenceManager {
    pub(super) fn new() -> Self {
        Self {
            backend: Box::new(FsSlotBackend),
            states_dir: None,
            mapper_save_path: None,
            mapper_save_flush_allowed: true,
            mapper_save_recovery_written: false,
            slots: Vec::new(),
            active_slot_id: None,
        }
    }

    pub(super) fn slots(&self) -> &[StateSlotSummary] {
        &self.slots
    }

    pub(super) fn active_slot_id(&self) -> Option<u64> {
        self.active_slot_id
    }

    #[cfg(test)]
    fn states_dir(&self) -> Option<&PathBuf> {
        self.states_dir.as_ref()
    }

    #[cfg(test)]
    fn mapper_save_path(&self) -> Option<&PathBuf> {
        self.mapper_save_path.as_ref()
    }

    pub(super) fn save_slot(
        &mut self,
        slot_id: u64,
        emu: &impl CorePersistence,
        make_active: bool,
    ) {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("save_slot: no states_dir configured; cannot save slot {slot_id}");
            return;
        };
        let Some(identity) = emu.canonical_media_identity() else {
            log::warn!("save_slot: no persistence identity available; cannot save slot {slot_id}");
            return;
        };
        log::info!(
            "save_slot: writing slot {slot_id} (make_active={make_active}) to {}",
            dir.display()
        );
        match emu.save_state_raw() {
            Ok(core_bytes) => {
                let state_blob = save_state_with_header(core_bytes);
                let preview = emu.generate_preview().map(|p| ThumbnailSource {
                    width: p.width,
                    height: p.height,
                    rgba: p.rgba,
                });
                match self.backend.write_slot(
                    dir,
                    slot_id,
                    &state_blob,
                    &identity,
                    preview.as_ref(),
                ) {
                    Ok(_) => {
                        if make_active {
                            self.active_slot_id = Some(slot_id);
                        }
                        self.refresh_slots_inner(Some(&identity));
                        log::info!("save_slot: saved slot {slot_id}");
                    }
                    Err(error) => log::warn!("saving state slot failed: {error}"),
                }
            }
            Err(error) => log::warn!("state export failed: {error}"),
        }
    }

    pub(super) fn save_active_slot_or_new(&mut self, emu: &impl CorePersistence) {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("save_active_slot_or_new: no states_dir configured; cannot save state");
            return;
        };
        let slot_id = self.active_slot_id.or_else(|| {
            self.backend
                .allocate_next_id(dir)
                .map_err(|error| {
                    log::warn!("allocating state slot failed: {error}");
                    error
                })
                .ok()
        });
        match slot_id {
            Some(slot_id) => {
                log::info!("save_active_slot_or_new: saving to slot {slot_id}");
                self.save_slot(slot_id, emu, true);
            }
            None => {
                log::warn!("save_active_slot_or_new: failed to allocate slot id");
            }
        }
    }

    pub(super) fn create_slot(&mut self, emu: &impl CorePersistence) {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("create_slot: no states_dir configured; cannot create slot");
            return;
        };
        match self.backend.allocate_next_id(dir) {
            Ok(slot_id) => self.save_slot(slot_id, emu, true),
            Err(error) => log::warn!("allocating state slot failed: {error}"),
        }
    }

    pub(super) fn load_slot(&mut self, slot_id: u64, emu: &impl CorePersistence) -> bool {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("load_slot: no states_dir configured; cannot load slot {slot_id}");
            return false;
        };
        match self.backend.read_slot(dir, slot_id) {
            Ok(Some(slot)) => {
                if let Err(error) = emu.load_state_raw(resolve_state_format(&slot.machine_state)) {
                    log::warn!("state import failed: {error}");
                    false
                } else {
                    self.active_slot_id = Some(slot_id);
                    let identity = emu.canonical_media_identity();
                    self.refresh_slots_inner(identity.as_ref());
                    true
                }
            }
            Ok(None) => false,
            Err(error) => {
                log::warn!("loading state slot failed: {error}");
                false
            }
        }
    }

    pub(super) fn load_active_slot(&mut self, emu: &impl CorePersistence) -> bool {
        self.active_slot_id
            .is_some_and(|slot_id| self.load_slot(slot_id, emu))
    }

    pub(super) fn delete_slot(&mut self, slot_id: u64, emu: &impl CorePersistence) {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("delete_slot: no states_dir configured; cannot delete slot {slot_id}");
            return;
        };
        match self.backend.delete_slot(dir, slot_id) {
            Ok(()) => {
                if self.active_slot_id == Some(slot_id) {
                    self.active_slot_id = None;
                }
                let identity = emu.canonical_media_identity();
                self.refresh_slots_inner(identity.as_ref());
            }
            Err(error) => log::warn!("deleting state slot failed: {error}"),
        }
    }

    pub(super) fn select_active_slot(&mut self, slot_id: u64) {
        self.active_slot_id = Some(slot_id);
    }

    pub(super) fn select_adjacent_slot(&mut self, forward: bool) -> Option<u64> {
        let next_slot_id = adjacent_slot_id(&self.slots, self.active_slot_id, forward)?;
        self.active_slot_id = Some(next_slot_id);
        Some(next_slot_id)
    }

    pub(super) fn refresh_slots(&mut self, emu: &impl CorePersistence) {
        let identity = emu.canonical_media_identity();
        self.refresh_slots_inner(identity.as_ref());
    }

    fn refresh_slots_inner(&mut self, identity: Option<&SystemIdentity>) {
        self.slots = if let (Some(dir), Some(identity)) = (self.states_dir.as_ref(), identity) {
            match self.backend.scan(dir, identity) {
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

    pub(super) fn configure(&mut self, states_dir: PathBuf, mapper_save_path: PathBuf) {
        self.states_dir = Some(states_dir);
        self.mapper_save_path = Some(mapper_save_path);
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.active_slot_id = None;
    }

    pub(super) fn reset(&mut self) {
        self.states_dir = None;
        self.mapper_save_path = None;
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.slots = Vec::new();
        self.active_slot_id = None;
    }

    pub(super) fn flush_mapper_save(
        &mut self,
        emu: &impl CorePersistence,
    ) -> Result<(), PersistenceError> {
        let Some(path) = self.mapper_save_path.as_ref() else {
            log::warn!("mapper_save_path not set, mapper save skipped");
            return Ok(());
        };
        if !self.mapper_save_flush_allowed {
            if self.mapper_save_recovery_written {
                return Ok(());
            }
            if let Some(bytes) = emu
                .save_mapper_raw()
                .map_err(|e| {
                    log::warn!("mapper save failed: {e}");
                    e
                })
                .ok()
                .flatten()
            {
                let recovery_path = self.backend.write_recovery_mapper_save(path, &bytes)?;
                self.mapper_save_recovery_written = true;
                log::warn!(
                    "mapper save auto-load failed earlier; wrote recovery save to {}",
                    recovery_path.display()
                );
            }
            return Ok(());
        }
        match emu.save_mapper_raw() {
            Ok(Some(bytes)) => self.backend.write_mapper_save(path, &bytes)?,
            Ok(None) => {}
            Err(e) => log::warn!("mapper save failed: {e}"),
        }
        Ok(())
    }

    pub(super) fn load_mapper_save_if_needed(
        &mut self,
        emu: &impl CorePersistence,
    ) -> Result<(), PersistenceError> {
        let Some(path) = self.mapper_save_path.as_ref() else {
            log::warn!("mapper_save_path not set, mapper save load skipped");
            return Ok(());
        };
        match self.backend.read_mapper_save(path) {
            Ok(Some(bytes)) => {
                if let Err(e) = emu.load_mapper_raw(bytes) {
                    log::warn!("mapper save load failed: {e}");
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

    pub(super) fn save_hidden(&mut self, emu: &impl CorePersistence) -> bool {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("save_hidden: no states_dir configured; cannot save hidden lifecycle state");
            return false;
        };
        let Some(identity) = emu.canonical_media_identity() else {
            return false;
        };
        match emu.save_state_raw() {
            Ok(core_bytes) => {
                let state_blob = save_state_with_header(core_bytes);
                match self.backend.write_autosave(dir, &state_blob, &identity) {
                    Ok(_) => true,
                    Err(error) => {
                        log::warn!("saving hidden lifecycle state failed: {error}");
                        false
                    }
                }
            }
            Err(error) => {
                log::warn!("hidden lifecycle state export failed: {error}");
                false
            }
        }
    }

    pub(super) fn load_hidden(&mut self, emu: &impl CorePersistence) -> bool {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!("load_hidden: no states_dir configured; cannot load hidden lifecycle state");
            return false;
        };
        let Some(identity) = emu.canonical_media_identity() else {
            return false;
        };
        match self.backend.read_autosave(dir, &identity) {
            Ok(Some(slot)) => {
                if slot.summary.emulator_version != env!("CARGO_PKG_VERSION") {
                    log::warn!(
                        "ignoring hidden lifecycle state from emulator version {} on {}",
                        slot.summary.emulator_version,
                        env!("CARGO_PKG_VERSION")
                    );
                    let _ = self.backend.delete_autosave(dir);
                    return false;
                }
                if let Err(error) = emu.load_state_raw(resolve_state_format(&slot.machine_state)) {
                    log::warn!("hidden lifecycle state import failed: {error}");
                    let _ = self.backend.delete_autosave(dir);
                    false
                } else {
                    true
                }
            }
            Ok(None) => {
                let _ = self.backend.delete_autosave(dir);
                false
            }
            Err(PersistenceError::Io(error)) if error.kind() == ErrorKind::NotFound => false,
            Err(error) => {
                log::warn!("loading hidden lifecycle state failed: {error}");
                let _ = self.backend.delete_autosave(dir);
                false
            }
        }
    }

    pub(super) fn clear_hidden(&mut self) {
        let Some(dir) = self.states_dir.as_ref() else {
            log::warn!(
                "clear_hidden: no states_dir configured; cannot clear hidden lifecycle state"
            );
            return;
        };
        let _ = self.backend.delete_autosave(dir);
    }
}

pub fn adjacent_slot_id(
    slots: &[StateSlotSummary],
    active_slot_id: Option<u64>,
    forward: bool,
) -> Option<u64> {
    if slots.is_empty() {
        return None;
    }
    let current_index = active_slot_id
        .and_then(|active| slots.iter().position(|slot| slot.slot_id == active))
        .unwrap_or_else(|| {
            if forward {
                slots.len().saturating_sub(1)
            } else {
                0
            }
        });
    let next_index = if forward {
        (current_index + 1) % slots.len()
    } else if current_index == 0 {
        slots.len() - 1
    } else {
        current_index - 1
    };
    Some(slots[next_index].slot_id)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use nerust_core_traits::factory::load::MediaObject;
    use nerust_persistence::slots::autosave_state_slot_path;

    use super::super::test_helpers::*;

    #[test]
    fn rebuild_preserves_restored_runtime_state_without_reloading_mapper_save() {
        let temp_dir = unique_temp_dir("rebuild");
        let rom_path = temp_dir.join("test.nes");

        let mut session = test_session();
        let options = session.factory().default_load_options();
        let resolved = session
            .factory()
            .resolve_load_request(&test_view(&session), options)
            .unwrap();
        session
            .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
            .unwrap();

        let mapper_save_path = session
            .persistence
            .mapper_save_path()
            .expect("load should configure mapper_save_path")
            .clone();
        fs::write(&mapper_save_path, [9, 8, 7, 6]).expect("mapper save should write");

        let mut next = session.settings_snapshot().clone();
        next.local.audio.latency_ms = 90;
        let plan = session.apply_settings(next).unwrap();

        assert!(plan.session_rebuild_required);
        assert!(mapper_save_path.exists());
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn hidden_lifecycle_state_round_trips_without_visible_slot() {
        let temp_dir = unique_temp_dir("hidden-lifecycle-state");
        let rom_path = temp_dir.join("test.nes");

        let mut session = test_session();
        let options = session.factory().default_load_options();
        let resolved = session
            .factory()
            .resolve_load_request(&test_view(&session), options)
            .unwrap();
        session
            .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
            .unwrap();

        assert!(session.save_hidden_lifecycle_state());
        let autosave_path = autosave_state_slot_path(
            session
                .persistence
                .states_dir()
                .expect("load should configure states_dir"),
        );
        assert!(autosave_path.is_file());
        assert!(session.slots().is_empty());
        assert_eq!(session.active_slot_id(), None);

        assert!(session.load_hidden_lifecycle_state());
        assert_eq!(session.slots().len(), 0);
        assert_eq!(session.active_slot_id(), None);

        drop(session);
        assert!(autosave_path.exists());
        fs::remove_file(&autosave_path).ok();
        assert!(!autosave_path.exists());
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn hidden_lifecycle_state_is_deleted_after_import_failure() {
        let temp_dir = unique_temp_dir("hidden-lifecycle-import");
        let rom_path = temp_dir.join("test.nes");

        let mut session = test_session();
        let options = session.factory().default_load_options();
        let resolved = session
            .factory()
            .resolve_load_request(&test_view(&session), options)
            .unwrap();
        session
            .load_resolved(MediaObject::new(Some(rom_path), test_rom()), resolved)
            .unwrap();

        assert!(session.save_hidden_lifecycle_state());
        let autosave_path = autosave_state_slot_path(
            session
                .persistence
                .states_dir()
                .expect("load should configure states_dir"),
        );
        assert!(autosave_path.is_file());

        fs::write(&autosave_path, [0xFF, 0xFF, 0xFF]).expect("corrupt state");
        assert!(!session.load_hidden_lifecycle_state());
        assert!(!autosave_path.exists());
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn hidden_lifecycle_state_is_deleted_after_identity_mismatch() {
        let temp_dir = unique_temp_dir("hidden-lifecycle-identity");
        let rom_path = temp_dir.join("test.nes");

        let mut session = test_session();
        let options = session.factory().default_load_options();
        let resolved = session
            .factory()
            .resolve_load_request(&test_view(&session), options)
            .unwrap();
        session
            .load_resolved(
                MediaObject::new(Some(rom_path.clone()), test_rom()),
                resolved,
            )
            .unwrap();
        assert!(session.save_hidden_lifecycle_state());

        let autosave_path = autosave_state_slot_path(
            session
                .persistence
                .states_dir()
                .expect("load should configure states_dir"),
        );
        assert!(autosave_path.is_file());
        drop(session);

        let mut session2 = test_session();
        let options = session2.factory().default_load_options();
        let resolved = session2
            .factory()
            .resolve_load_request(&test_view(&session2), options)
            .unwrap();
        session2
            .load_resolved(
                MediaObject::new(Some(rom_path), test_rom_with_mapper4()),
                resolved,
            )
            .unwrap();
        assert!(!session2.load_hidden_lifecycle_state());
        assert!(!autosave_path.exists());
        let _ = fs::remove_dir_all(temp_dir);
    }
}
