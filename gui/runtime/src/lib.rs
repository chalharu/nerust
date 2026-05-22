use nerust_console::{ControllerInputs, PreviewFrame};
use nerust_core::CoreOptions;
pub use nerust_gui_session::{
    ConsoleError, ControllerInput, ControllerPort, InputState, SessionCommand,
    SessionCommandOutcome, SessionCore, window_title,
};
pub use nerust_persistence::StateSlotSummary;
use nerust_persistence::{
    SidecarPaths, ThumbnailSource, allocate_next_slot_id, delete_state_slot, format_slot_saved_at,
    latest_saved_slot_id, load_mapper_save, load_state_slot, resolve_sidecars,
    scan_state_slots_for_target, state_slot_path, write_mapper_save, write_recovery_mapper_save,
    write_state_slot,
};
pub use nerust_screen_filter::NesVideoAssets;
pub use nerust_screen_traits::VideoPresentation;
use std::path::PathBuf;

pub trait ConsoleSessionFactory {
    fn build_session(&self) -> GuiSession;
}

#[derive(Debug)]
pub struct GuiSession {
    core: SessionCore,
    rom_path: Option<PathBuf>,
    sidecars: Option<SidecarPaths>,
    mapper_save_flush_allowed: bool,
    mapper_save_recovery_written: bool,
    slots: Vec<StateSlotSummary>,
    active_slot_id: Option<u64>,
}

impl GuiSession {
    pub fn from_session_core(core: SessionCore) -> Self {
        Self {
            core,
            rom_path: None,
            sidecars: None,
            mapper_save_flush_allowed: true,
            mapper_save_recovery_written: false,
            slots: Vec::new(),
            active_slot_id: None,
        }
    }

    pub fn presentation(&self) -> &VideoPresentation {
        self.core.presentation()
    }

    pub fn nes_video_assets(&self) -> Option<&NesVideoAssets> {
        self.core.video().nes_video_assets()
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.core.with_frame_buffer(f)
    }

    pub fn physical_size(&self) -> nerust_screen_traits::PhysicalSize {
        self.core.physical_size()
    }

    pub fn metrics(&self) -> nerust_gui_session::ConsoleMetrics {
        self.core.metrics()
    }

    pub fn window_title(&self) -> String {
        window_title(self.paused(), self.metrics())
    }

    pub fn paused(&self) -> bool {
        self.core.paused()
    }

    pub fn loaded(&self) -> bool {
        self.core.loaded()
    }

    pub fn can_pause(&self) -> bool {
        self.core.can_pause()
    }

    pub fn can_resume(&self) -> bool {
        self.core.can_resume()
    }

    pub fn slots(&self) -> &[StateSlotSummary] {
        &self.slots
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.active_slot_id
    }

    pub fn reset(&self) -> Result<(), ConsoleError> {
        self.core.reset()
    }

    pub fn pause(&mut self) {
        self.core.pause();
    }

    pub fn resume(&mut self) {
        self.core.resume();
    }

    pub fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
        match command {
            SessionCommand::Pause => {
                if self.paused() {
                    SessionCommandOutcome::default()
                } else {
                    self.pause();
                    SessionCommandOutcome {
                        executed: true,
                        needs_redraw: false,
                    }
                }
            }
            SessionCommand::Resume => {
                if self.paused() {
                    self.resume();
                    SessionCommandOutcome {
                        executed: true,
                        needs_redraw: self.loaded(),
                    }
                } else {
                    SessionCommandOutcome::default()
                }
            }
            SessionCommand::TogglePause => {
                if self.paused() {
                    self.run_command(SessionCommand::Resume)
                } else {
                    self.run_command(SessionCommand::Pause)
                }
            }
            SessionCommand::Reset => {
                if let Err(error) = self.reset() {
                    log::warn!("reset failed: {error}");
                    SessionCommandOutcome::default()
                } else {
                    SessionCommandOutcome {
                        executed: true,
                        needs_redraw: false,
                    }
                }
            }
            SessionCommand::CreateSlot => {
                self.create_slot();
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::SaveActiveSlotOrNew => {
                self.save_active_slot_or_new();
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::LoadActiveSlot => {
                let was_paused = self.paused();
                let executed = self.load_active_slot();
                SessionCommandOutcome {
                    executed,
                    needs_redraw: redraw_needed_after_pause_change(
                        executed,
                        was_paused,
                        self.paused(),
                    ),
                }
            }
            SessionCommand::SelectActiveSlot(slot_id) => {
                self.select_active_slot(slot_id);
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::SaveSlot(slot_id) => {
                self.save_slot(slot_id, false);
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::LoadSlot(slot_id) => {
                let was_paused = self.paused();
                let executed = self.load_slot(slot_id);
                SessionCommandOutcome {
                    executed,
                    needs_redraw: redraw_needed_after_pause_change(
                        executed,
                        was_paused,
                        self.paused(),
                    ),
                }
            }
            SessionCommand::DeleteSlot(slot_id) => {
                self.delete_slot(slot_id);
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::SelectNextSlot => SessionCommandOutcome {
                executed: self.select_adjacent_slot(true).is_some(),
                needs_redraw: false,
            },
            SessionCommand::SelectPreviousSlot => SessionCommandOutcome {
                executed: self.select_adjacent_slot(false).is_some(),
                needs_redraw: false,
            },
        }
    }

    pub fn set_port_inputs(&mut self, port: ControllerPort, inputs: ControllerInputs) {
        self.core.set_port_inputs(port, inputs);
    }

    pub fn clear_all_inputs(&mut self) {
        self.core.clear_all_inputs();
    }

    pub fn load(&mut self, rom_path: Option<PathBuf>, data: Vec<u8>) -> bool {
        self.load_with_options(rom_path, data, CoreOptions::default())
    }

    pub fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
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
        self.rom_path = rom_path;
        self.sidecars = self.rom_path.as_deref().map(resolve_sidecars);
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.active_slot_id = None;
        self.refresh_slots();
        self.active_slot_id = latest_saved_slot_id(&self.slots);
        if let Err(error) = self.load_mapper_save_if_available() {
            self.mapper_save_flush_allowed = false;
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
        self.rom_path = None;
        self.sidecars = None;
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.active_slot_id = None;
        self.slots.clear();
        true
    }

    pub fn flush_before_exit(&mut self) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before close failed: {error}");
        }
    }

    pub fn save_active_slot_or_new(&mut self) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        let slot_id =
            self.active_slot_id
                .or_else(|| match allocate_next_slot_id(&sidecars.states_dir) {
                    Ok(slot_id) => Some(slot_id),
                    Err(error) => {
                        log::warn!("allocating state slot failed: {error}");
                        None
                    }
                });
        if let Some(slot_id) = slot_id {
            self.save_slot(slot_id, true);
        }
    }

    pub fn create_slot(&mut self) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        match allocate_next_slot_id(&sidecars.states_dir) {
            Ok(slot_id) => self.save_slot(slot_id, true),
            Err(error) => log::warn!("allocating state slot failed: {error}"),
        }
    }

    pub fn save_slot(&mut self, slot_id: u64, make_active: bool) {
        let Some(sidecars) = self.sidecars.as_ref() else {
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
                            self.active_slot_id = Some(slot_id);
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
        self.active_slot_id
            .is_some_and(|slot_id| self.load_slot(slot_id))
    }

    pub fn load_slot(&mut self, slot_id: u64) -> bool {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return false;
        };
        match load_state_slot(&state_slot_path(&sidecars.states_dir, slot_id)) {
            Ok(slot) => {
                if let Err(error) = self.core.import_state(slot.machine_state) {
                    log::warn!("state import failed: {error}");
                    false
                } else {
                    self.active_slot_id = Some(slot_id);
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
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        match delete_state_slot(&state_slot_path(&sidecars.states_dir, slot_id)) {
            Ok(()) => {
                if self.active_slot_id == Some(slot_id) {
                    self.active_slot_id = None;
                }
                self.refresh_slots();
            }
            Err(error) => log::warn!("deleting state slot failed: {error}"),
        }
    }

    pub fn select_active_slot(&mut self, slot_id: u64) {
        self.active_slot_id = Some(slot_id);
    }

    pub fn select_adjacent_slot(&mut self, forward: bool) -> Option<u64> {
        let next_slot_id = adjacent_slot_id(&self.slots, self.active_slot_id, forward)?;
        self.active_slot_id = Some(next_slot_id);
        Some(next_slot_id)
    }

    fn refresh_slots(&mut self) {
        self.slots = if let Some(sidecars) = self.sidecars.as_ref() {
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
        if self
            .active_slot_id
            .is_some_and(|slot_id| !self.slots.iter().any(|slot| slot.slot_id == slot_id))
        {
            self.active_slot_id = None;
        }
    }

    fn load_mapper_save_if_available(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.sidecars.as_ref() else {
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
        let Some(sidecars) = self.sidecars.as_ref() else {
            return Ok(());
        };
        if !self.mapper_save_flush_allowed {
            if self.mapper_save_recovery_written {
                return Ok(());
            }
            if let Some(bytes) = self
                .core
                .export_mapper_save()
                .map_err(|error| error.to_string())?
            {
                let path = write_recovery_mapper_save(&sidecars.mapper_save_path, &bytes)
                    .map_err(|error| error.to_string())?;
                self.mapper_save_recovery_written = true;
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

pub fn slot_label(slot: &StateSlotSummary, active_slot: Option<u64>) -> String {
    let saved_at = format_slot_saved_at(slot.saved_at);
    let active = if active_slot == Some(slot.slot_id) {
        " (active)"
    } else {
        ""
    };
    format!("Slot {} — {saved_at}{active}", slot.slot_id)
}

fn adjacent_slot_id(
    slots: &[StateSlotSummary],
    active_slot: Option<u64>,
    forward: bool,
) -> Option<u64> {
    if slots.is_empty() {
        return None;
    }
    Some(
        if let Some(current) = active_slot
            && let Some(index) = slots.iter().position(|slot| slot.slot_id == current)
        {
            let offset = if forward {
                (index + 1) % slots.len()
            } else {
                (index + slots.len() - 1) % slots.len()
            };
            slots[offset].slot_id
        } else if forward {
            slots[0].slot_id
        } else {
            slots[slots.len() - 1].slot_id
        },
    )
}

fn redraw_needed_after_pause_change(executed: bool, was_paused: bool, paused: bool) -> bool {
    executed && was_paused && !paused
}

fn preview_to_thumbnail_source(preview: &PreviewFrame) -> ThumbnailSource {
    ThumbnailSource {
        width: preview.width,
        height: preview.height,
        rgba: preview.rgba.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GuiSession, SessionCore, adjacent_slot_id, redraw_needed_after_pause_change, slot_label,
        window_title,
    };
    use nerust_console::{Console, ConsoleMetrics};
    use nerust_persistence::StateSlotSummary;
    use nerust_screen_filter::FilterType;
    use nerust_screen_traits::LogicalSize;
    use nerust_sound_traits::{MixerInput, Sound};
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    #[derive(Default)]
    struct TestSpeaker;

    impl Sound for TestSpeaker {
        fn start(&mut self) {}

        fn pause(&mut self) {}
    }

    impl MixerInput for TestSpeaker {
        fn push(&mut self, _: f32) {}
    }

    fn test_session() -> GuiSession {
        GuiSession::from_session_core(SessionCore::from_console(Console::new_gpu(
            TestSpeaker,
            FilterType::NtscComposite,
            LogicalSize {
                width: 256,
                height: 240,
            },
        )))
    }

    fn slot(slot_id: u64) -> StateSlotSummary {
        StateSlotSummary {
            schema_version: 1,
            slot_id,
            path: PathBuf::from(format!("slot-{slot_id}.nst")),
            saved_at: UNIX_EPOCH + Duration::from_secs(1_700_000_000 + slot_id),
            has_thumbnail: false,
            emulator_version: "test".into(),
        }
    }

    #[test]
    fn window_title_surfaces_runtime_metrics() {
        let title = window_title(
            false,
            ConsoleMetrics {
                loaded: true,
                emulation_fps: 59.9,
                speed_multiplier: 1.01,
                ..ConsoleMetrics::default()
            },
        );

        assert!(title.contains("FPS 59.9"));
        assert!(title.contains("Speed x1.01"));
    }

    #[test]
    fn window_title_marks_no_rom() {
        assert!(window_title(true, ConsoleMetrics::default()).contains("Paused"));
        assert!(window_title(true, ConsoleMetrics::default()).contains("No ROM"));
    }

    #[test]
    fn slot_label_marks_active_slot() {
        let label = slot_label(&slot(2), Some(2));
        assert!(label.contains("Slot 2"));
        assert!(label.contains("(active)"));
    }

    #[test]
    fn adjacent_slot_selection_wraps_in_both_directions() {
        let slots = vec![slot(1), slot(3), slot(7)];

        assert_eq!(adjacent_slot_id(&slots, Some(7), true), Some(1));
        assert_eq!(adjacent_slot_id(&slots, Some(1), false), Some(7));
        assert_eq!(adjacent_slot_id(&slots, None, true), Some(1));
        assert_eq!(adjacent_slot_id(&slots, None, false), Some(7));
    }

    #[test]
    fn redraw_is_only_requested_when_a_command_resumes_emulation() {
        assert!(redraw_needed_after_pause_change(true, true, false));
        assert!(!redraw_needed_after_pause_change(true, false, false));
        assert!(!redraw_needed_after_pause_change(true, true, true));
        assert!(!redraw_needed_after_pause_change(false, true, false));
    }

    #[test]
    fn test_session_builds_gui_session() {
        let session = test_session();

        assert!(!session.loaded());
        assert!(session.paused());
        assert!(session.physical_size().width > 0.0);
    }
}
