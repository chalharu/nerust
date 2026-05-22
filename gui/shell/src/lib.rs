//! Shell-facing adapter layer for NES console sessions.
//!
//! This crate provides the NES-specific adapter types that connect a generic
//! [`nerust_gui_runtime::GuiSession`] to a concrete shell binary:
//!
//! - [`NesConsoleDescriptor`] — builds a NES [`nerust_gui_runtime::GuiSession`]
//!   and describes its controller layout.
//! - [`NesInputAdapter`] — translates host key/button events to NES controller
//!   inputs and flushes them to the session.
//! - [`NativeShellState`] — tracks frame-presentation and redraw timing for
//!   native window shells.
//!
//! # Shell × Backend Composition Policy
//!
//! Backend selection is fixed at **build-time / binary level**. Each shell
//! binary links against exactly one backend crate (`nerust_backend_opengl` or
//! `nerust_backend_wgpu`) and there is **no runtime mechanism for switching
//! backends** while the application is running.
//!
//! To add a new rendering backend, create a new binary target crate that
//! composes `nerust_gui_shell` with the new backend. Do not add runtime
//! dispatch or feature-flag backend selection to this crate.
//!
//! Current shipped combinations:
//! - `nerust_gtk`    → `nerust_backend_opengl` (GTK 3 + OpenGL 3.3)
//! - `nerust_glutin` → `nerust_backend_opengl` (winit + glutin + OpenGL 3.3)
//! - `nerust_wgpu`   → `nerust_backend_wgpu`   (tao + wgpu)

use nerust_console::{Console, ControllerInputs};
use nerust_gui_runtime::{
    ButtonDescriptor, ConsoleSessionFactory, ControllerDescriptor, ControllerInput, ControllerPort,
    GuiSession, InputState, SessionCore,
};
use nerust_screen_buffer::ScreenBuffer;
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;
use std::time::{Duration, Instant};

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

#[derive(Debug, Clone, Copy, Default)]
pub struct NesConsoleDescriptor;

impl NesConsoleDescriptor {
    pub fn build_console(self) -> Console {
        let speaker = OpenAl::new(48_000, CLOCK_RATE as i32, 128, 20);
        Console::new(speaker, ScreenBuffer::new_nes_gpu_default())
    }

    /// Returns the controller descriptor for the NES.
    ///
    /// Button names use the canonical NES names: **A** and **B** (not
    /// "Primary"/"Secondary"), matching the physical labels and the key
    /// mappings in [`NesInputAdapter`].
    pub fn controller_descriptor(&self) -> ControllerDescriptor {
        ControllerDescriptor {
            port_count: 2,
            buttons: vec![
                ButtonDescriptor {
                    name: "A",
                    description: "Face button A",
                },
                ButtonDescriptor {
                    name: "B",
                    description: "Face button B",
                },
                ButtonDescriptor {
                    name: "Select",
                    description: "Select button",
                },
                ButtonDescriptor {
                    name: "Start",
                    description: "Start button",
                },
                ButtonDescriptor {
                    name: "Up",
                    description: "D-pad Up",
                },
                ButtonDescriptor {
                    name: "Down",
                    description: "D-pad Down",
                },
                ButtonDescriptor {
                    name: "Left",
                    description: "D-pad Left",
                },
                ButtonDescriptor {
                    name: "Right",
                    description: "D-pad Right",
                },
            ],
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

    fn port_index(port: ControllerPort) -> usize {
        match port {
            ControllerPort::One => 0,
            ControllerPort::Two => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NativeShellState, NesConsoleDescriptor, NesInputAdapter};
    use nerust_console::ControllerInputs;
    use nerust_gui_runtime::{ControllerInput, ControllerPort, InputState};
    use std::time::Instant;

    #[test]
    fn nes_input_adapter_maps_a_to_controller_a_and_b_to_controller_b() {
        assert_eq!(
            NesInputAdapter::nes_flag(ControllerInput::A),
            ControllerInputs::A
        );
        assert_eq!(
            NesInputAdapter::nes_flag(ControllerInput::B),
            ControllerInputs::B
        );
    }

    #[test]
    fn nes_input_adapter_tracks_pressed_and_released() {
        let mut adapter = NesInputAdapter::new();
        adapter.handle_input(ControllerPort::One, ControllerInput::A, InputState::Pressed);
        adapter.handle_input(
            ControllerPort::One,
            ControllerInput::Right,
            InputState::Pressed,
        );
        adapter.handle_input(
            ControllerPort::One,
            ControllerInput::A,
            InputState::Released,
        );
        assert_eq!(adapter.held[0], ControllerInputs::RIGHT);
    }

    #[test]
    fn nes_descriptor_has_canonical_ab_button_names() {
        let descriptor = NesConsoleDescriptor.controller_descriptor();
        let names: Vec<&str> = descriptor.buttons.iter().map(|b| b.name).collect();

        assert!(names.contains(&"A"), "expected A button in NES descriptor");
        assert!(names.contains(&"B"), "expected B button in NES descriptor");
        assert!(
            !names.contains(&"Primary"),
            "Primary is not a NES button name"
        );
        assert!(
            !names.contains(&"Secondary"),
            "Secondary is not a NES button name"
        );
        assert_eq!(descriptor.port_count, 2);
        assert_eq!(descriptor.buttons.len(), 8);
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
