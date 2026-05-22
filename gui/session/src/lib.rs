use nerust_console::{Console, ControllerInputs};
use nerust_contract::CoreOptions;
use nerust_screen_traits::PhysicalSize;

pub use nerust_console::ControllerPort;
pub use nerust_console::{ConsoleError, ConsoleMetrics, ConsoleVideo, PreviewFrame, StateExport};
pub use nerust_screen_traits::VideoPresentation;

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

/// A host-facing descriptor of a single controller button.
///
/// Button names use the console's canonical naming (e.g. "A", "B" for NES)
/// rather than generic names, so that UI labels match what users see on the
/// physical hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonDescriptor {
    /// The canonical button name shown to users (e.g. `"A"`, `"Start"`).
    pub name: &'static str,
    /// Short human-readable description of the button's function.
    pub description: &'static str,
}

/// A host-facing descriptor of a console's controller configuration.
///
/// This type lives in the shared session layer so that any shell or UI layer
/// can inspect the controller layout without depending on NES-specific
/// implementation details. NES-specific values are provided by
/// `NesConsoleDescriptor::controller_descriptor` in the `nerust_gui_shell`
/// crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerDescriptor {
    /// Number of supported controller ports.
    pub port_count: usize,
    /// Ordered list of buttons present on each controller.
    pub buttons: Vec<ButtonDescriptor>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionCommandOutcome {
    pub executed: bool,
    pub needs_redraw: bool,
}

#[derive(Debug)]
pub struct SessionCore {
    paused: bool,
    loaded: bool,
    console: Console,
    physical_size: PhysicalSize,
}

impl SessionCore {
    pub fn from_console(console: Console) -> Self {
        let metrics = console.metrics();
        let physical_size = console.video().presentation().physical_size();
        Self {
            paused: metrics.paused,
            loaded: metrics.loaded,
            console,
            physical_size,
        }
    }

    pub fn presentation(&self) -> &VideoPresentation {
        self.video().presentation()
    }

    pub fn video(&self) -> &ConsoleVideo {
        self.console.video()
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.console.with_frame_buffer(f)
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.physical_size
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        self.console.metrics()
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

    pub fn set_port_inputs(&mut self, port: ControllerPort, inputs: ControllerInputs) {
        self.console.set_port_inputs(port, inputs);
    }

    pub fn clear_all_inputs(&mut self) {
        self.console
            .set_port_inputs(ControllerPort::One, ControllerInputs::empty());
        self.console
            .set_port_inputs(ControllerPort::Two, ControllerInputs::empty());
    }

    pub fn load_rom(&mut self, data: Vec<u8>, options: CoreOptions) -> Result<(), ConsoleError> {
        self.console.load_with_options(data, options)?;
        self.loaded = true;
        self.clear_all_inputs();
        Ok(())
    }

    pub fn unload_rom(&mut self) -> Result<(), ConsoleError> {
        let result = self.console.unload();
        self.loaded = false;
        self.clear_all_inputs();
        result
    }

    pub fn export_state(&self) -> Result<StateExport, ConsoleError> {
        self.console.export_state()
    }

    pub fn import_state(&self, bytes: Vec<u8>) -> Result<(), ConsoleError> {
        self.console.import_state(bytes)
    }

    pub fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, ConsoleError> {
        self.console.export_mapper_save()
    }

    pub fn import_mapper_save(&self, bytes: Vec<u8>) -> Result<(), ConsoleError> {
        self.console.import_mapper_save(bytes)
    }

    pub fn persistence_target(&self) -> Result<nerust_console::PersistenceTarget, ConsoleError> {
        self.console.persistence_target()
    }

    pub fn sync_paused_from_console(&mut self) {
        self.paused = self.console.metrics().paused;
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

#[cfg(test)]
mod tests {
    use super::{ButtonDescriptor, ControllerDescriptor, SessionCore, window_title};
    use nerust_console::{Console, ConsoleMetrics};
    use nerust_screen_buffer::ScreenBuffer;
    use nerust_sound_traits::{MixerInput, Sound};

    #[derive(Default)]
    struct TestSpeaker;

    impl Sound for TestSpeaker {
        fn start(&mut self) {}

        fn pause(&mut self) {}
    }

    impl MixerInput for TestSpeaker {
        fn push(&mut self, _data: f32) {}
    }

    fn test_core() -> SessionCore {
        SessionCore::from_console(Console::new(
            TestSpeaker,
            ScreenBuffer::new_nes_gpu_default(),
        ))
    }

    #[test]
    fn button_descriptor_has_expected_fields() {
        let btn = ButtonDescriptor {
            name: "A",
            description: "Face button A",
        };

        assert_eq!(btn.name, "A");
        assert_eq!(btn.description, "Face button A");
    }

    #[test]
    fn controller_descriptor_holds_button_list() {
        let desc = ControllerDescriptor {
            port_count: 2,
            buttons: vec![
                ButtonDescriptor {
                    name: "A",
                    description: "A",
                },
                ButtonDescriptor {
                    name: "B",
                    description: "B",
                },
            ],
        };

        assert_eq!(desc.port_count, 2);
        assert_eq!(desc.buttons.len(), 2);
    }

    #[test]
    fn session_core_from_console_preserves_video_shape() {
        let core = test_core();

        assert!(core.paused());
        assert!(!core.loaded());
        assert!(!core.can_pause());
        assert!(!core.can_resume());
        assert_eq!(
            core.presentation().physical_size().width,
            core.physical_size().width
        );
        assert!(core.with_frame_buffer(|buffer| !buffer.is_empty()));
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
}
