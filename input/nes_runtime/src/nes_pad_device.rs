use crate::ControllerState;
use nerust_contract_core::input::InputState;
use nerust_nes_core::OpenBusReadResult;
use nerust_nes_core::controller::Controller;

/// NES パッドの Device 実装。
///
/// `InputState` 経由で最新のボタン状態を取得し、シフトレジスタとして
/// CPU の $4016/$4017 読み出しに応答する。
pub struct NesPadDevice<S: InputState<2>> {
    state: S,
    buttons: [u8; 2],
    index: [u8; 2],
    strobe: bool,
}

impl<S: InputState<2>> NesPadDevice<S> {
    pub fn new(state: S) -> Self {
        Self {
            state,
            buttons: [0; 2],
            index: [0; 2],
            strobe: false,
        }
    }
}

impl<S: InputState<2>> NesPadDevice<S> {
    fn export_inner(&self) -> [u8; 5] {
        [self.buttons[0], self.buttons[1], self.index[0], self.index[1], self.strobe as u8]
    }

    fn import_inner(&mut self, state: &[u8; 5]) {
        self.buttons = [state[0], state[1]];
        self.index = [state[2], state[3]];
        self.strobe = state[4] != 0;
    }
}

impl<S: InputState<2> + Send + 'static> ControllerState for NesPadDevice<S> {
    fn reset_runtime(&mut self) {
        self.buttons = [0; 2];
        self.index = [0; 2];
        self.strobe = false;
    }

    fn validate_controller_state(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() != 5 {
            return Err("invalid controller state length".into());
        }
        Ok(())
    }

    fn apply_controller_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let arr: [u8; 5] = bytes.try_into().map_err(|_| "invalid controller state length")?;
        self.import_inner(&arr);
        Ok(())
    }

    fn current_controller_state(&self) -> Result<Vec<u8>, String> {
        Ok(self.export_inner().to_vec())
    }
}

impl<S: InputState<2>> Controller for NesPadDevice<S> {
    fn read(&mut self, address: usize) -> OpenBusReadResult {
        match address {
            0 => {
                let bit = if self.index[0] < 8 {
                    let b = (self.buttons[0] >> self.index[0]) & 1;
                    if !self.strobe {
                        self.index[0] += 1;
                    }
                    b
                } else {
                    1
                };
                OpenBusReadResult::new(bit, 7)
            }
            _ => {
                let bit = if self.index[1] < 8 {
                    let b = (self.buttons[1] >> self.index[1]) & 1;
                    if !self.strobe {
                        self.index[1] += 1;
                    }
                    b
                } else {
                    1
                };
                OpenBusReadResult::new(bit, 0x1F)
            }
        }
    }

    fn write(&mut self, value: u8) {
        self.strobe = value & 1 == 1;
        if self.strobe {
            self.buttons = self.state.sample();
            self.index = [0, 0];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use nerust_contract_core::input::InputCell;

    #[test]
    fn strobe_latches_current_state() {
        let cell = Arc::new(InputCell::new());
        let mut device = NesPadDevice::new(cell.clone());

        // Verify the cell stores correctly
        assert_eq!(cell.load(), [0, 0]);
        cell.store(&[0x01, 0x00]);
        assert_eq!(cell.load(), [0x01, 0x00]);

        device.write(1); // strobe=1 → latch
        device.write(0); // strobe=0 → enable shift
        assert_eq!(device.buttons, [0x01, 0x00]);

        let result = device.read(0);
        assert_eq!(result.data & 1, 1, "first bit (A) should be 1, got data={}", result.data);
        assert_eq!(device.read(0).data & 1, 0, "second bit should be 0");
        for i in 0..6 {
            assert_eq!(device.read(0).data & 1, 0, "bit {} should be 0", i + 2);
        }
        assert_eq!(device.read(0).data & 1, 1, "open bus after 8 bits should be 1");
    }

    #[test]
    fn updated_state_after_strobe() {
        let cell = Arc::new(InputCell::new());
        let mut device = NesPadDevice::new(cell.clone());

        cell.store(&[0x80, 0x00]); // P1=Up
        device.write(1);
        device.write(0); // latch 0x80

        // Read first bit (should be 0, then Up=1 at bit 7)
        for i in 0..7 {
            assert_eq!(device.read(0).data & 1, 0, "bit {i} should be 0");
        }
        assert_eq!(device.read(0).data & 1, 1, "bit 7 (Up) should be 1");

        // Update state while shift in progress
        cell.store(&[0x01, 0x00]); // P1=A pressed
        // Shift continues with latched old state
        assert_eq!(device.read(0).data & 1, 1, "open bus after 8 bits");
    }

    #[test]
    fn second_player_reads_from_port_1() {
        let cell = Arc::new(InputCell::new());
        let mut device = NesPadDevice::new(cell.clone());

        cell.store(&[0x00, 0x02]); // P2=B pressed
        device.write(1);
        device.write(0); // latch

        assert_eq!(device.read(1).data & 1, 0);
        assert_eq!(device.read(1).data & 1, 1, "P2 B at bit 1");
    }
}
