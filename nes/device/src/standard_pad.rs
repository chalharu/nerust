use nerust_nes_core::{OpenBusReadResult, controller::Controller};
use nerust_nes_core::input_types::Buttons;
use nerust_nes_controller::{
    ControllerState, StandardControllerSnapshot, decode_controller_state, encode_controller_state,
};

use crate::pad_common;

/// NES Standard Controller: full 8-button pad on both ports.
/// $4016=P1, $4017=P2. No microphone.
#[derive(Debug, Clone)]
pub struct StandardPad {
    cached: [u8; 2],
    index: [u8; 2],
    strobe: bool,
}

impl StandardPad {
    pub fn new() -> Self {
        Self { cached: [0; 2], index: [0; 2], strobe: false }
    }
}

impl Default for StandardPad {
    fn default() -> Self { Self::new() }
}

impl Controller for StandardPad {
    fn sync_input(&mut self, state: &[u8]) {
        if state.len() >= 2 { self.cached = [state[0], state[1]]; }
    }
    fn read(&mut self, address: usize) -> OpenBusReadResult {
        pad_common::read(&self.cached, &mut self.index, self.strobe, address, false)
    }
    fn write(&mut self, value: u8) {
        pad_common::write(&mut self.strobe, &mut self.index, value);
    }
}

impl ControllerState for StandardPad {
    fn reset_runtime(&mut self) {
        self.index = [0; 2];
        self.strobe = false;
    }
    fn validate_controller_state(&self, bytes: &[u8]) -> Result<(), String> {
        decode_controller_state(bytes).map(|_| ())
    }
    fn apply_controller_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let s = decode_controller_state(bytes)?;
        self.cached = [s.buttons[0].bits(), s.buttons[1].bits()];
        self.index = [s.index1 as u8, s.index2 as u8];
        self.strobe = s.strobe;
        Ok(())
    }
    fn current_controller_state(&self) -> Result<Vec<u8>, String> {
        encode_controller_state(StandardControllerSnapshot {
            buttons: [Buttons::from_bits_truncate(self.cached[0]), Buttons::from_bits_truncate(self.cached[1])],
            microphone: false,
            index1: self.index[0] as usize,
            index2: self.index[1] as usize,
            strobe: self.strobe,
        })
    }
}
