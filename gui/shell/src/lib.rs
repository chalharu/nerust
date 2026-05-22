use nerust_console::{Console, ControllerInputs};
use nerust_gui_runtime::SessionCore;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::LogicalSize;
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;
use std::time::{Duration, Instant};

const DEFAULT_FILTER_TYPE: FilterType = FilterType::NtscComposite;
const DEFAULT_SOURCE_LOGICAL_SIZE: LogicalSize = LogicalSize {
    width: 256,
    height: 240,
};

#[derive(Debug)]
pub struct NativeShellState {
    pub needs_redraw: bool,
    last_presented_frame_counter: u64,
    last_title_update: Instant,
}

impl NativeShellState {
    pub const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);
    pub const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);

    pub fn new() -> Self {
        Self {
            needs_redraw: true,
            last_presented_frame_counter: 0,
            last_title_update: Instant::now(),
        }
    }

    pub fn on_frame_presented(&mut self, frame_counter: u64) {
        self.last_presented_frame_counter = frame_counter;
        self.needs_redraw = false;
    }

    pub fn wants_redraw(&self, current_frame_counter: u64) -> bool {
        self.needs_redraw || current_frame_counter != self.last_presented_frame_counter
    }

    pub fn wants_poll(&self, loaded: bool, paused: bool) -> bool {
        self.needs_redraw || (loaded && !paused)
    }

    pub fn should_refresh_title(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last_title_update) >= Self::TITLE_UPDATE_INTERVAL {
            self.last_title_update = now;
            true
        } else {
            false
        }
    }
}

impl Default for NativeShellState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NesConsoleDescriptor {
    pub filter_type: FilterType,
    pub source_logical_size: LogicalSize,
}

impl NesConsoleDescriptor {
    pub fn build_console(self) -> Console {
        let speaker = OpenAl::new(48_000, CLOCK_RATE as i32, 128, 20);
        Console::new_gpu(speaker, self.filter_type, self.source_logical_size)
    }
}

impl Default for NesConsoleDescriptor {
    fn default() -> Self {
        Self {
            filter_type: DEFAULT_FILTER_TYPE,
            source_logical_size: DEFAULT_SOURCE_LOGICAL_SIZE,
        }
    }
}

impl ConsoleSessionFactory for NesConsoleDescriptor {
    fn build_session(&self) -> GuiSession {
        let core = SessionCore::from_console(self.build_console());
        GuiSession::from_session_core(core)
    }
}

#[derive(Debug)]
pub struct NesInputAdapter {
    held: [ControllerInputs; 2],
}

impl Default for NesInputAdapter {
    fn default() -> Self {
        Self {
            held: [ControllerInputs::empty(); 2],
        }
    }
}

impl NesInputAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_input(
        &mut self,
        port: ControllerPort,
        input: ControllerInput,
        state: InputState,
    ) {
        let flag = Self::nes_flag(input);
        let i = Self::port_index(port);
        self.held[i] = match state {
            InputState::Pressed => self.held[i] | flag,
            InputState::Released => self.held[i] & !flag,
        };
    }

    pub fn flush_to_session(&self, session: &mut GuiSession) {
        session.set_port_inputs(ControllerPort::One, self.held[0]);
        session.set_port_inputs(ControllerPort::Two, self.held[1]);
    }

    pub fn clear(&mut self, session: &mut GuiSession) {
        self.held = [ControllerInputs::empty(); 2];
        self.flush_to_session(session);
    }

    fn nes_flag(input: ControllerInput) -> ControllerInputs {
        match input {
            ControllerInput::Primary => ControllerInputs::A,
            ControllerInput::Secondary => ControllerInputs::B,
            ControllerInput::Select => ControllerInputs::SELECT,
            ControllerInput::Start => ControllerInputs::START,
            ControllerInput::Up => ControllerInputs::UP,
            ControllerInput::Down => ControllerInputs::DOWN,
            ControllerInput::Left => ControllerInputs::LEFT,
            ControllerInput::Right => ControllerInputs::RIGHT,
        }
    }

    fn port_index(port: ControllerPort) -> usize {
        match port {
            ControllerPort::One => 0,
            ControllerPort::Two => 1,
        }
    }
}

pub use nerust_gui_runtime::{
    ConsoleSessionFactory, GuiSession, SessionCommand, SessionCommandOutcome, StateSlotSummary,
    VideoPresentation, slot_label, window_title,
};
pub use nerust_gui_session::{ConsoleMetrics, ControllerInput, ControllerPort, InputState};
pub use nerust_screen_filter::NesVideoAssets;

#[cfg(test)]
mod tests {
    use super::{NativeShellState, NesInputAdapter};
    use nerust_console::ControllerInputs;
    use nerust_gui_session::{ControllerInput, ControllerPort, InputState};
    use std::time::Instant;

    #[test]
    fn nes_input_adapter_maps_primary_to_a_and_secondary_to_b() {
        assert_eq!(
            NesInputAdapter::nes_flag(ControllerInput::Primary),
            ControllerInputs::A
        );
        assert_eq!(
            NesInputAdapter::nes_flag(ControllerInput::Secondary),
            ControllerInputs::B
        );
    }

    #[test]
    fn nes_input_adapter_tracks_pressed_and_released() {
        let mut adapter = NesInputAdapter::new();
        adapter.handle_input(
            ControllerPort::One,
            ControllerInput::Primary,
            InputState::Pressed,
        );
        adapter.handle_input(
            ControllerPort::One,
            ControllerInput::Right,
            InputState::Pressed,
        );
        adapter.handle_input(
            ControllerPort::One,
            ControllerInput::Primary,
            InputState::Released,
        );
        assert_eq!(adapter.held[0], ControllerInputs::RIGHT);
    }

    #[test]
    fn native_shell_state_tracks_frame_presentation() {
        let mut shell = NativeShellState::new();
        assert!(shell.wants_redraw(0));
        shell.on_frame_presented(1);
        assert!(!shell.wants_redraw(1));
        assert!(shell.wants_redraw(2));
    }

    #[test]
    fn native_shell_state_refreshes_title_after_interval() {
        let mut shell = NativeShellState::new();
        let now = Instant::now();
        assert!(!shell.should_refresh_title(now));
        assert!(shell.should_refresh_title(now + NativeShellState::TITLE_UPDATE_INTERVAL));
    }
}
