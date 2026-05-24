mod mapper_save;
mod slots;

use super::GuiSession;
use crate::{StateSlotSummary, options::CoreOptions};
use nerust_persistence::{SidecarPaths, latest_saved_slot_id, resolve_sidecars};

#[derive(Debug, Default)]
pub(super) struct PersistenceState {
    sidecars: Option<SidecarPaths>,
    mapper_save_flush_allowed: bool,
    mapper_save_recovery_written: bool,
    slots: Vec<StateSlotSummary>,
    active_slot_id: Option<u64>,
}

impl PersistenceState {
    pub(super) fn slots(&self) -> &[StateSlotSummary] {
        &self.slots
    }

    pub(super) fn active_slot_id(&self) -> Option<u64> {
        self.active_slot_id
    }

    pub(super) fn select_active_slot(&mut self, slot_id: u64) {
        self.active_slot_id = Some(slot_id);
    }
}

impl GuiSession {
    pub fn load(&mut self, rom_path: Option<std::path::PathBuf>, data: Vec<u8>) -> bool {
        self.load_with_options(rom_path, data, CoreOptions::default())
    }

    pub fn load_with_options(
        &mut self,
        rom_path: Option<std::path::PathBuf>,
        data: Vec<u8>,
        options: CoreOptions,
    ) -> bool {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before load failed: {error}");
            return false;
        }
        if let Err(error) = self.core.load_rom(data, options) {
            log::warn!("ROM load failed: {error}");
            return false;
        }
        self.persistence.sidecars = rom_path.as_deref().map(resolve_sidecars);
        self.persistence.mapper_save_flush_allowed = true;
        self.persistence.mapper_save_recovery_written = false;
        self.persistence.active_slot_id = None;
        self.refresh_slots();
        self.persistence.active_slot_id = latest_saved_slot_id(&self.persistence.slots);
        if let Err(error) = self.load_mapper_save_if_available() {
            self.persistence.mapper_save_flush_allowed = false;
            log::warn!("mapper save auto-load failed: {error}");
        }
        true
    }

    pub fn unload(&mut self) -> bool {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before unload failed: {error}");
            return false;
        }
        let _ = self.core.unload_rom();
        self.persistence.sidecars = None;
        self.persistence.mapper_save_flush_allowed = true;
        self.persistence.mapper_save_recovery_written = false;
        self.persistence.active_slot_id = None;
        self.persistence.slots.clear();
        true
    }

    pub fn flush_before_exit(&mut self) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before close failed: {error}");
        }
    }
}

impl Drop for GuiSession {
    fn drop(&mut self) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush during shutdown failed: {error}");
        }
    }
}
