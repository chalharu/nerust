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

        cell.store(&[0x01, 0x00]); // P1=A pressed
        device.write(1); // strobe=1 → latch
        device.write(0); // strobe=0 → enable shift

        assert_eq!(device.read(0).data & 1, 1);
        assert_eq!(device.read(0).data & 1, 0); // next bit (no more buttons)
        assert_eq!(device.read(0).data & 1, 1); // open bus after 8 bits
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
