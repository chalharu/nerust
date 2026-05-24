use nerust_console::console::{ControllerInputs, ControllerPort};
use nerust_gui_runtime::session::GuiSession;
use nerust_gui_session::input::{ControllerInput, InputState};

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
    use super::NesInputAdapter;
    use nerust_console::console::{ControllerInputs, ControllerPort};
    use nerust_gui_session::input::{ControllerInput, InputState};

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
}
