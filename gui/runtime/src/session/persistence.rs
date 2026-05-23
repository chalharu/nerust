use super::GuiSession;
use crate::{CoreOptions, StateSlotSummary};
use nerust_console::PreviewFrame;
use nerust_persistence::{
    SidecarPaths, ThumbnailSource, allocate_next_slot_id, delete_state_slot, latest_saved_slot_id,
    load_mapper_save, load_state_slot, resolve_sidecars, scan_state_slots_for_target,
    state_slot_path, write_mapper_save, write_recovery_mapper_save, write_state_slot,
};

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

    pub fn save_active_slot_or_new(&mut self) {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return;
        };
        let slot_id = self.persistence.active_slot_id.or_else(|| {
            match allocate_next_slot_id(&sidecars.states_dir) {
                Ok(slot_id) => Some(slot_id),
                Err(error) => {
                    log::warn!("allocating state slot failed: {error}");
                    None
                }
            }
        });
        if let Some(slot_id) = slot_id {
            self.save_slot(slot_id, true);
        }
    }

    pub fn create_slot(&mut self) {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return;
        };
        match allocate_next_slot_id(&sidecars.states_dir) {
            Ok(slot_id) => self.save_slot(slot_id, true),
            Err(error) => log::warn!("allocating state slot failed: {error}"),
        }
    }

    pub fn save_slot(&mut self, slot_id: u64, make_active: bool) {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return;
        };
        match self.core.export_state() {
            Ok(export) => {
                let preview = export.preview.as_ref().map(preview_to_thumbnail_source);
                match write_state_slot(
                    &sidecars.states_dir,
                    slot_id,
                    &export.machine_state,
                    export.target,
                    preview.as_ref(),
                ) {
                    Ok(_) => {
                        if make_active {
                            self.persistence.active_slot_id = Some(slot_id);
                        }
                        self.refresh_slots();
                    }
                    Err(error) => log::warn!("saving state slot failed: {error}"),
                }
            }
            Err(error) => log::warn!("state export failed: {error}"),
        }
    }

    pub fn load_active_slot(&mut self) -> bool {
        self.persistence
            .active_slot_id
            .is_some_and(|slot_id| self.load_slot(slot_id))
    }

    pub fn load_slot(&mut self, slot_id: u64) -> bool {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return false;
        };
        match load_state_slot(&state_slot_path(&sidecars.states_dir, slot_id)) {
            Ok(slot) => {
                if let Err(error) = self.core.import_state(slot.machine_state) {
                    log::warn!("state import failed: {error}");
                    false
                } else {
                    self.persistence.active_slot_id = Some(slot_id);
                    self.core.sync_paused_from_console();
                    self.refresh_slots();
                    true
                }
            }
            Err(error) => {
                log::warn!("loading state slot failed: {error}");
                false
            }
        }
    }

    pub fn delete_slot(&mut self, slot_id: u64) {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return;
        };
        match delete_state_slot(&state_slot_path(&sidecars.states_dir, slot_id)) {
            Ok(()) => {
                if self.persistence.active_slot_id == Some(slot_id) {
                    self.persistence.active_slot_id = None;
                }
                self.refresh_slots();
            }
            Err(error) => log::warn!("deleting state slot failed: {error}"),
        }
    }

    fn refresh_slots(&mut self) {
        self.persistence.slots = if let Some(sidecars) = self.persistence.sidecars.as_ref() {
            match self.core.persistence_target() {
                Ok(target) => match scan_state_slots_for_target(&sidecars.states_dir, target) {
                    Ok(slots) => slots,
                    Err(error) => {
                        log::warn!("slot scan failed: {error}");
                        Vec::new()
                    }
                },
                Err(error) => {
                    log::warn!("state slot target unavailable: {error}");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if self.persistence.active_slot_id.is_some_and(|slot_id| {
            !self
                .persistence
                .slots
                .iter()
                .any(|slot| slot.slot_id == slot_id)
        }) {
            self.persistence.active_slot_id = None;
        }
    }

    fn load_mapper_save_if_available(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return Ok(());
        };
        if let Some(bytes) =
            load_mapper_save(&sidecars.mapper_save_path).map_err(|error| error.to_string())?
        {
            self.core
                .import_mapper_save(bytes)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn flush_mapper_save(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.persistence.sidecars.as_ref() else {
            return Ok(());
        };
        if !self.persistence.mapper_save_flush_allowed {
            if self.persistence.mapper_save_recovery_written {
                return Ok(());
            }
            if let Some(bytes) = self
                .core
                .export_mapper_save()
                .map_err(|error| error.to_string())?
            {
                let path = write_recovery_mapper_save(&sidecars.mapper_save_path, &bytes)
                    .map_err(|error| error.to_string())?;
                self.persistence.mapper_save_recovery_written = true;
                log::warn!(
                    "mapper save auto-load failed earlier; wrote recovery save to {}",
                    path.display()
                );
            }
            return Ok(());
        }
        match self
            .core
            .export_mapper_save()
            .map_err(|error| error.to_string())?
        {
            Some(bytes) => write_mapper_save(&sidecars.mapper_save_path, &bytes)
                .map_err(|error| error.to_string()),
            None => Ok(()),
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

fn preview_to_thumbnail_source(preview: &PreviewFrame) -> ThumbnailSource {
    ThumbnailSource {
        width: preview.width,
        height: preview.height,
        rgba: preview.rgba.clone(),
    }
}
