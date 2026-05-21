use nerust_console::{Console, ConsoleError, ConsoleMetrics, ControllerInputs, PreviewFrame};
use nerust_core::CoreOptions;
pub use nerust_persistence::StateSlotSummary;
use nerust_persistence::{
    SidecarPaths, ThumbnailSource, allocate_next_slot_id, delete_state_slot, format_slot_saved_at,
    latest_saved_slot_id, load_mapper_save, load_state_slot, resolve_sidecars,
    scan_state_slots_for_target, state_slot_path, write_mapper_save, write_recovery_mapper_save,
    write_state_slot,
};
use nerust_screen_filter::FilterType;
pub use nerust_screen_filter::presentation::VideoPresentation;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;
use std::path::PathBuf;

const DEFAULT_FILTER_TYPE: FilterType = FilterType::NtscComposite;
const DEFAULT_SOURCE_LOGICAL_SIZE: LogicalSize = LogicalSize {
    width: 256,
    height: 240,
};

pub use nerust_console::ControllerPort;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerInput {
    A,
    B,
    Select,
    Start,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCommand {
    Pause,
    Resume,
    TogglePause,
    Reset,
    CreateSlot,
    SaveActiveSlotOrNew,
    LoadActiveSlot,
    SelectActiveSlot(u64),
    SaveSlot(u64),
    LoadSlot(u64),
    DeleteSlot(u64),
    SelectNextSlot,
    SelectPreviousSlot,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionCommandOutcome {
    pub executed: bool,
    pub needs_redraw: bool,
}

#[derive(Debug)]
pub struct GuiSession {
    paused: bool,
    loaded: bool,
    console: Console,
    physical_size: PhysicalSize,
    rom_path: Option<PathBuf>,
    sidecars: Option<SidecarPaths>,
    mapper_save_flush_allowed: bool,
    mapper_save_recovery_written: bool,
    slots: Vec<StateSlotSummary>,
    active_slot_id: Option<u64>,
    held_inputs: [ControllerInputs; 2],
}

impl GuiSession {
    pub fn new(filter_type: FilterType, source_logical_size: LogicalSize) -> Self {
        let speaker = OpenAl::new(48_000, CLOCK_RATE as i32, 128, 20);
        let console = Console::new_gpu(speaker, filter_type, source_logical_size);
        let physical_size = console.video().presentation().physical_size();
        let paused = console.metrics().paused;
        Self {
            paused,
            loaded: false,
            console,
            physical_size,
            rom_path: None,
            sidecars: None,
            mapper_save_flush_allowed: true,
            mapper_save_recovery_written: false,
            slots: Vec::new(),
            active_slot_id: None,
            held_inputs: [ControllerInputs::empty(); 2],
        }
    }

    pub fn presentation(&self) -> &VideoPresentation {
        self.console.video().presentation()
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.console.video().frame_buffer().with_bytes(f)
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.physical_size
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        self.console.metrics()
    }

    pub fn window_title(&self) -> String {
        window_title(self.paused, self.console.metrics())
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn can_pause(&self) -> bool {
        self.loaded && !self.paused
    }

    pub fn can_resume(&self) -> bool {
        self.loaded && self.paused
    }

    pub fn slots(&self) -> &[StateSlotSummary] {
        &self.slots
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.active_slot_id
    }

    pub fn reset(&self) -> Result<(), ConsoleError> {
        self.console.reset()
    }

    pub fn pause(&mut self) {
        self.console.pause();
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.console.resume();
        self.paused = false;
    }

    pub fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
        match command {
            SessionCommand::Pause => {
                if self.paused {
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
                if self.paused {
                    self.resume();
                    SessionCommandOutcome {
                        executed: true,
                        needs_redraw: self.loaded,
                    }
                } else {
                    SessionCommandOutcome::default()
                }
            }
            SessionCommand::TogglePause => {
                if self.paused {
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
                let was_paused = self.paused;
                let executed = self.load_active_slot();
                SessionCommandOutcome {
                    executed,
                    needs_redraw: redraw_needed_after_pause_change(
                        executed,
                        was_paused,
                        self.paused,
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
                let was_paused = self.paused;
                let executed = self.load_slot(slot_id);
                SessionCommandOutcome {
                    executed,
                    needs_redraw: redraw_needed_after_pause_change(
                        executed,
                        was_paused,
                        self.paused,
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

    pub fn handle_controller_input(
        &mut self,
        port: ControllerPort,
        input: ControllerInput,
        state: InputState,
    ) {
        update_held_buttons(
            &mut self.held_inputs[controller_port_index(port)],
            input,
            state,
        );
        self.apply_controller_state(port);
    }

    pub fn clear_controller_input(&mut self) {
        self.held_inputs = [ControllerInputs::empty(); 2];
        self.apply_controller_state(ControllerPort::One);
        self.apply_controller_state(ControllerPort::Two);
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
        if let Err(error) = self.console.load_with_options(data, options) {
            log::warn!("ROM load failed: {error}");
            return false;
        }
        self.loaded = true;
        self.rom_path = rom_path;
        self.sidecars = self.rom_path.as_deref().map(resolve_sidecars);
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.active_slot_id = None;
        self.clear_controller_input();
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
        let _ = self.console.unload();
        self.loaded = false;
        self.rom_path = None;
        self.sidecars = None;
        self.mapper_save_flush_allowed = true;
        self.mapper_save_recovery_written = false;
        self.active_slot_id = None;
        self.slots.clear();
        self.clear_controller_input();
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
        match self.console.export_state() {
            Ok(export) => {
                let preview = export.preview.as_ref().map(preview_to_thumbnail_source);
                match write_state_slot(
                    &sidecars.states_dir,
                    slot_id,
                    &export.machine_state,
                    export.rom_identity,
                    export.options,
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
                if let Err(error) = self.console.import_state(slot.machine_state) {
                    log::warn!("state import failed: {error}");
                    false
                } else {
                    self.active_slot_id = Some(slot_id);
                    self.sync_paused_from_console();
                    // Preserve the controller snapshot from the save state until
                    // new host input arrives.
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

    fn sync_paused_from_console(&mut self) {
        self.paused = self.console.metrics().paused;
    }

    fn apply_controller_state(&self, port: ControllerPort) {
        self.console
            .set_port_inputs(port, self.held_inputs[controller_port_index(port)]);
    }

    fn refresh_slots(&mut self) {
        self.slots = if let Some(sidecars) = self.sidecars.as_ref() {
            match self.console.persistence_target() {
                Ok((rom_identity, options)) => {
                    match scan_state_slots_for_target(&sidecars.states_dir, rom_identity, options) {
                        Ok(slots) => slots,
                        Err(error) => {
                            log::warn!("slot scan failed: {error}");
                            Vec::new()
                        }
                    }
                }
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
            self.console
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
                .console
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
        let bytes = self
            .console
            .export_mapper_save()
            .map_err(|error| error.to_string())?;
        match bytes {
            Some(bytes) => write_mapper_save(&sidecars.mapper_save_path, &bytes)
                .map_err(|error| error.to_string()),
            None => Ok(()),
        }
    }
}

impl Default for GuiSession {
    fn default() -> Self {
        Self::new(DEFAULT_FILTER_TYPE, DEFAULT_SOURCE_LOGICAL_SIZE)
    }
}

impl Drop for GuiSession {
    fn drop(&mut self) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush during shutdown failed: {error}");
        }
    }
}

pub fn window_title(paused: bool, console_metrics: ConsoleMetrics) -> String {
    let state = if paused { "Nes -- Paused" } else { "Nes" };
    if console_metrics.loaded {
        format!(
            "{state} | FPS {:.1} | Speed x{:.2}",
            console_metrics.emulation_fps, console_metrics.speed_multiplier
        )
    } else {
        format!("{state} | No ROM")
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

fn controller_port_index(port: ControllerPort) -> usize {
    match port {
        ControllerPort::One => 0,
        ControllerPort::Two => 1,
    }
}

fn controller_input_flag(input: ControllerInput) -> ControllerInputs {
    match input {
        ControllerInput::A => ControllerInputs::A,
        ControllerInput::B => ControllerInputs::B,
        ControllerInput::Select => ControllerInputs::SELECT,
        ControllerInput::Start => ControllerInputs::START,
        ControllerInput::Up => ControllerInputs::UP,
        ControllerInput::Down => ControllerInputs::DOWN,
        ControllerInput::Left => ControllerInputs::LEFT,
        ControllerInput::Right => ControllerInputs::RIGHT,
    }
}

fn update_held_buttons(inputs: &mut ControllerInputs, input: ControllerInput, state: InputState) {
    let flag = controller_input_flag(input);
    *inputs = match state {
        InputState::Pressed => *inputs | flag,
        InputState::Released => *inputs & !flag,
    };
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
        ControllerInput, ControllerPort, InputState, adjacent_slot_id, controller_input_flag,
        controller_port_index, redraw_needed_after_pause_change, slot_label, update_held_buttons,
        window_title,
    };
    use nerust_console::{ConsoleMetrics, ControllerInputs};
    use nerust_persistence::StateSlotSummary;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

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
    fn controller_intents_map_to_console_input_flags() {
        assert_eq!(
            controller_input_flag(ControllerInput::A),
            ControllerInputs::A
        );
        assert_eq!(
            controller_input_flag(ControllerInput::B),
            ControllerInputs::B
        );
        assert_eq!(
            controller_input_flag(ControllerInput::Select),
            ControllerInputs::SELECT
        );
        assert_eq!(
            controller_input_flag(ControllerInput::Start),
            ControllerInputs::START
        );
        assert_eq!(
            controller_input_flag(ControllerInput::Up),
            ControllerInputs::UP
        );
        assert_eq!(
            controller_input_flag(ControllerInput::Down),
            ControllerInputs::DOWN
        );
        assert_eq!(
            controller_input_flag(ControllerInput::Left),
            ControllerInputs::LEFT
        );
        assert_eq!(
            controller_input_flag(ControllerInput::Right),
            ControllerInputs::RIGHT
        );
    }

    #[test]
    fn held_controller_state_tracks_press_release_and_ports() {
        let mut port1 = ControllerInputs::empty();
        let mut port2 = ControllerInputs::empty();

        update_held_buttons(&mut port1, ControllerInput::A, InputState::Pressed);
        update_held_buttons(&mut port1, ControllerInput::Right, InputState::Pressed);
        update_held_buttons(&mut port2, ControllerInput::B, InputState::Pressed);
        update_held_buttons(&mut port1, ControllerInput::A, InputState::Released);

        assert_eq!(port1, ControllerInputs::RIGHT);
        assert_eq!(port2, ControllerInputs::B);
        assert_eq!(controller_port_index(ControllerPort::One), 0);
        assert_eq!(controller_port_index(ControllerPort::Two), 1);
    }

    #[test]
    fn redraw_is_only_requested_when_a_command_resumes_emulation() {
        assert!(redraw_needed_after_pause_change(true, true, false));
        assert!(!redraw_needed_after_pause_change(true, false, false));
        assert!(!redraw_needed_after_pause_change(true, true, true));
        assert!(!redraw_needed_after_pause_change(false, true, false));
    }
}
