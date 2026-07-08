use nerust_nes_controller::{
    ControllerState, StandardControllerSnapshot, decode_controller_state, encode_controller_state,
};
use nerust_nes_core::{OpenBusReadResult, controller::Controller, input_types::Buttons};

/// NES パッドの Device 実装。
///
/// `Controller::sync_input()` で最新の入力状態を受け取り、シフトレジスタとして
/// CPU の $4016/$4017 読み出しに応答する。mic は $4016 D2 に現れる。
pub struct NesPadDevice {
    cached: [u8; 3],
    index: [u8; 2],
    strobe: bool,
}

impl NesPadDevice {
    pub fn new() -> Self {
        Self {
            cached: [0; 3],
            index: [0; 2],
            strobe: false,
        }
    }
}

impl ControllerState for NesPadDevice {
    fn reset_runtime(&mut self) {
        self.index = [0; 2];
        self.strobe = false;
    }

    fn validate_controller_state(&self, bytes: &[u8]) -> Result<(), String> {
        decode_controller_state(bytes).map(|_| ())
    }

    fn apply_controller_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let snapshot = decode_controller_state(bytes)?;
        self.cached = [
            snapshot.buttons[0].bits(),
            snapshot.buttons[1].bits(),
            snapshot.microphone as u8,
        ];
        self.index = [snapshot.index1 as u8, snapshot.index2 as u8];
        self.strobe = snapshot.strobe;
        Ok(())
    }

    fn current_controller_state(&self) -> Result<Vec<u8>, String> {
        let s = self.cached;
        encode_controller_state(StandardControllerSnapshot {
            buttons: [
                Buttons::from_bits_truncate(s[0]),
                Buttons::from_bits_truncate(s[1]),
            ],
            microphone: s[2] != 0,
            index1: self.index[0] as usize,
            index2: self.index[1] as usize,
            strobe: self.strobe,
        })
    }
}

impl Controller for NesPadDevice {
    fn sync_input(&mut self, state: &[u8]) {
        if state.len() >= 3 {
            self.cached = [state[0], state[1], state[2]];
        }
    }

    fn read(&mut self, address: usize) -> OpenBusReadResult {
        let buttons = [self.cached[0], self.cached[1]];
        match address {
            0 => {
                let bit = if self.index[0] < 8 {
                    let b = (buttons[0] >> self.index[0]) & 1;
                    if !self.strobe {
                        self.index[0] += 1;
                    }
                    b
                } else {
                    1
                };
                let mic = if self.cached[2] != 0 { 0x04 } else { 0 };
                OpenBusReadResult::new(bit | mic, 7)
            }
            _ => {
                let bit = if self.index[1] < 8 {
                    let b = (buttons[1] >> self.index[1]) & 1;
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
            self.index = [0, 0];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_device() -> NesPadDevice {
        let mut d = NesPadDevice::new();
        d.sync_input(&[0x00, 0x00, 0]);
        d
    }

    #[test]
    fn strobe_latches_current_state() {
        let mut device = make_device();

        device.sync_input(&[0x01, 0x00, 0]);
        device.write(1);
        device.write(0);

        assert_eq!(device.read(0).data & 1, 1, "first bit (A) should be 1");
        assert_eq!(device.read(0).data & 1, 0, "second bit should be 0");
        for i in 0..6 {
            assert_eq!(device.read(0).data & 1, 0, "bit {} should be 0", i + 2);
        }
        assert_eq!(
            device.read(0).data & 1,
            1,
            "open bus after 8 bits should be 1"
        );
    }

    #[test]
    fn microphone_appears_on_port_0_d2() {
        let mut device = make_device();

        device.sync_input(&[0x00, 0x00, 1]);
        device.write(1);
        device.write(0);

        assert_eq!(
            device.read(0).data & 0x04,
            0x04,
            "mic bit (D2) should be set"
        );
    }

    #[test]
    fn microphone_does_not_affect_p2_shift_register() {
        let mut device = make_device();

        device.sync_input(&[0x00, 0x02, 1]);
        device.write(1);
        device.write(0);

        assert_eq!(device.read(0).data & 0x04, 0x04);
        assert_eq!(device.read(1).data & 1, 0);
        assert_eq!(device.read(1).data & 1, 1, "P2 B at bit 1");
    }

    #[test]
    fn updated_state_after_strobe() {
        let mut device = make_device();

        device.sync_input(&[0x80, 0x00, 0]);
        device.write(1);
        device.write(0);

        for i in 0..7 {
            assert_eq!(device.read(0).data & 1, 0, "bit {i} should be 0");
        }
        assert_eq!(device.read(0).data & 1, 1, "bit 7 (Up) should be 1");

        // State changes after sync_input don't affect already-shifted data
        device.sync_input(&[0x01, 0x00, 0]);
        assert_eq!(device.read(0).data & 1, 1, "open bus after 8 bits");
    }

    #[test]
    fn second_player_reads_from_port_1() {
        let mut device = make_device();

        device.sync_input(&[0x00, 0x02, 0]);
        device.write(1);
        device.write(0);

        assert_eq!(device.read(1).data & 1, 0);
        assert_eq!(device.read(1).data & 1, 1, "P2 B at bit 1");
    }
}
