use super::GuiSession;
use nerust_console::state::PreviewFrame;
use nerust_persistence::slots::{
    allocate_next_slot_id, delete_state_slot, load_state_slot, scan_state_slots_for_target,
    state_slot_path, write_state_slot,
};
use nerust_persistence::thumbnail::ThumbnailSource;

impl GuiSession {
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

    pub(super) fn refresh_slots(&mut self) {
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
}

fn preview_to_thumbnail_source(preview: &PreviewFrame) -> ThumbnailSource {
    ThumbnailSource {
        width: preview.width,
        height: preview.height,
        rgba: preview.rgba.clone(),
    }
}
