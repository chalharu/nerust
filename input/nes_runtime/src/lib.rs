use nerust_contract_controller_runtime::ControllerRuntime;
use nerust_input_nes::codec::{decode_input_state, encode_input_state as encode_frame_input_state};
use nerust_input_nes::frame::{Buttons, NesInputFrame};
use nerust_nes_core::OpenBusReadResult;
use nerust_nes_core::controller::Controller;

const STANDARD_CONTROLLER_MAX_INDEX: usize = 8;
const CONTROLLER_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandardControllerSnapshot {
    pub buttons: [Buttons; 2],
    pub microphone: bool,
    pub index1: usize,
    pub index2: usize,
    pub strobe: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct ControllerStatePayload {
    schema_version: u32,
    buttons: [Buttons; 2],
    microphone: bool,
    index1: u64,
    index2: u64,
    strobe: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy)]
pub struct StandardController {
    buttons: [Buttons; 2],
    microphone: bool,
    index1: usize,
    index2: usize,
    strobe: bool,
}

pub fn input_frame_from_snapshot(snapshot: StandardControllerSnapshot) -> NesInputFrame {
    NesInputFrame {
        player_one: snapshot.buttons[0],
        player_two: snapshot.buttons[1],
        microphone: snapshot.microphone,
    }
}

pub fn snapshot_with_input_frame(
    snapshot: StandardControllerSnapshot,
    frame: NesInputFrame,
) -> StandardControllerSnapshot {
    StandardControllerSnapshot {
        buttons: [frame.player_one, frame.player_two],
        microphone: frame.microphone,
        ..snapshot
    }
}

pub fn encode_controller_state(snapshot: StandardControllerSnapshot) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec_named(&ControllerStatePayload {
        schema_version: CONTROLLER_STATE_SCHEMA_VERSION,
        buttons: snapshot.buttons,
        microphone: snapshot.microphone,
        index1: snapshot.index1 as u64,
        index2: snapshot.index2 as u64,
        strobe: snapshot.strobe,
    })
    .map_err(|error| error.to_string())
}

pub fn decode_controller_state(bytes: &[u8]) -> Result<StandardControllerSnapshot, String> {
    let payload = rmp_serde::from_slice::<ControllerStatePayload>(bytes)
        .map_err(|error| error.to_string())?;
    if payload.schema_version != CONTROLLER_STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported controller state schema version: {}",
            payload.schema_version
        ));
    }
    let index1 = usize::try_from(payload.index1).map_err(|_| "controller port1 index overflow")?;
    let index2 = usize::try_from(payload.index2).map_err(|_| "controller port2 index overflow")?;
    if index1 > STANDARD_CONTROLLER_MAX_INDEX {
        return Err("controller port1 index out of range".into());
    }
    if index2 > STANDARD_CONTROLLER_MAX_INDEX {
        return Err("controller port2 index out of range".into());
    }
    Ok(StandardControllerSnapshot {
        buttons: payload.buttons,
        microphone: payload.microphone,
        index1,
        index2,
        strobe: payload.strobe,
    })
}

pub fn encode_input_state(snapshot: StandardControllerSnapshot) -> Result<Vec<u8>, String> {
    encode_frame_input_state(input_frame_from_snapshot(snapshot))
}

pub fn apply_input_state(
    snapshot: StandardControllerSnapshot,
    bytes: &[u8],
) -> Result<StandardControllerSnapshot, String> {
    let frame = decode_input_state(bytes)?;
    Ok(snapshot_with_input_frame(snapshot, frame))
}

impl StandardController {
    pub fn new() -> Self {
        Self {
            buttons: [Buttons::empty(); 2],
            microphone: false,
            index1: 0,
            index2: 0,
            strobe: false,
        }
    }

    pub fn reset(&mut self) {
        self.buttons = [Buttons::empty(); 2];
        self.microphone = false;
        self.index1 = 0;
        self.index2 = 0;
        self.strobe = false;
    }

    pub fn export_snapshot(&self) -> StandardControllerSnapshot {
        StandardControllerSnapshot {
            buttons: self.buttons,
            microphone: self.microphone,
            index1: self.index1,
            index2: self.index2,
            strobe: self.strobe,
        }
    }

    pub fn import_snapshot(&mut self, snapshot: StandardControllerSnapshot) {
        self.buttons = snapshot.buttons;
        self.microphone = snapshot.microphone;
        self.index1 = snapshot.index1;
        self.index2 = snapshot.index2;
        self.strobe = snapshot.strobe;
    }

    pub fn set_pad1(&mut self, buttons: Buttons) {
        self.buttons[0] = buttons;
    }

    pub fn set_pad2(&mut self, buttons: Buttons) {
        self.buttons[1] = buttons;
    }

    pub fn set_microphone(&mut self, microphone: bool) {
        self.microphone = microphone;
    }
}

impl Default for StandardController {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller for StandardController {
    fn read(&mut self, address: usize) -> OpenBusReadResult {
        match address {
            0 => OpenBusReadResult::new(
                if self.index1 < 8 {
                    let result = self.buttons[0].bits() >> self.index1;
                    if !self.strobe {
                        self.index1 += 1;
                    }
                    result & 1
                } else {
                    1
                } | (if self.microphone { 0x04 } else { 0 }),
                7,
            ),
            1 => OpenBusReadResult::new(
                if self.index2 < 8 {
                    let result = self.buttons[1].bits() >> self.index2;
                    if !self.strobe {
                        self.index2 += 1;
                    }
                    result & 1
                } else {
                    1
                },
                0x1F,
            ),
            _ => {
                log::error!("unhandled controller read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        }
    }

    fn write(&mut self, value: u8) {
        self.strobe = value & 1 == 1;
        if self.strobe {
            self.index1 = 0;
            self.index2 = 0;
        }
    }
}

impl ControllerRuntime for StandardController {
    fn reset_runtime(&mut self) {
        self.reset();
    }

    fn apply_input_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.import_snapshot(crate::apply_input_state(self.export_snapshot(), bytes)?);
        Ok(())
    }

    fn validate_controller_state(&self, bytes: &[u8]) -> Result<(), String> {
        decode_controller_state(bytes).map(|_| ())
    }

    fn apply_controller_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.import_snapshot(decode_controller_state(bytes)?);
        Ok(())
    }

    fn current_controller_state(&self) -> Result<Vec<u8>, String> {
        encode_controller_state(self.export_snapshot())
    }

    fn current_input_state(&self) -> Result<Vec<u8>, String> {
        encode_input_state(self.export_snapshot())
    }
}

pub fn standard_controller_runtime() -> Box<dyn ControllerRuntime> {
    Box::new(StandardController::new())
}

#[cfg(test)]
mod tests {
    use super::{
        StandardController, StandardControllerSnapshot, apply_input_state, decode_controller_state,
        encode_controller_state, encode_input_state,
    };
    use nerust_input_nes::codec::decode_input_state;
    use nerust_input_nes::frame::{Buttons, NesInputFrame};
    use nerust_nes_core::controller::Controller;

    #[test]
    fn controller_state_round_trips() {
        let snapshot = StandardControllerSnapshot {
            buttons: [Buttons::A | Buttons::START, Buttons::LEFT],
            microphone: true,
            index1: 3,
            index2: 5,
            strobe: true,
        };

        let decoded =
            decode_controller_state(&encode_controller_state(snapshot).expect("encode succeeds"))
                .expect("decode succeeds");

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn encoded_input_state_round_trips_from_snapshot() {
        let snapshot = StandardControllerSnapshot {
            buttons: [Buttons::A | Buttons::START, Buttons::LEFT],
            microphone: true,
            index1: 3,
            index2: 5,
            strobe: true,
        };

        let decoded = decode_input_state(&encode_input_state(snapshot).expect("encode succeeds"))
            .expect("decode succeeds");

        assert_eq!(
            decoded,
            NesInputFrame {
                player_one: Buttons::A | Buttons::START,
                player_two: Buttons::LEFT,
                microphone: true,
            }
        );
    }

    #[test]
    fn applying_input_state_preserves_shift_register_state() {
        let base = StandardControllerSnapshot {
            buttons: [Buttons::empty(), Buttons::LEFT],
            microphone: true,
            index1: 3,
            index2: 5,
            strobe: true,
        };

        let snapshot = apply_input_state(
            base,
            &nerust_input_nes::codec::encode_input_state(NesInputFrame {
                player_one: Buttons::A,
                player_two: Buttons::RIGHT,
                microphone: false,
            })
            .expect("encode succeeds"),
        )
        .expect("input state should apply");

        assert_eq!(
            snapshot,
            StandardControllerSnapshot {
                buttons: [Buttons::A, Buttons::RIGHT],
                microphone: false,
                index1: 3,
                index2: 5,
                strobe: true,
            }
        );
    }

    #[test]
    fn standard_controller_returns_one_after_eight_bits() {
        let mut controller = StandardController::new();
        controller.import_snapshot(StandardControllerSnapshot {
            buttons: [Buttons::A, Buttons::empty()],
            microphone: false,
            index1: 0,
            index2: 0,
            strobe: false,
        });

        controller.write(1);
        controller.write(0);

        for _ in 0..8 {
            let _ = controller.read(0);
        }

        assert_eq!(controller.read(0).data & 1, 1);
        assert_eq!(controller.read(0).data & 1, 1);
    }

    #[test]
    fn standard_controller_reports_microphone_on_port_zero_d2() {
        let mut controller = StandardController::new();

        controller.import_snapshot(StandardControllerSnapshot {
            buttons: [Buttons::empty(), Buttons::empty()],
            microphone: true,
            index1: 0,
            index2: 0,
            strobe: false,
        });
        assert_eq!(controller.read(0).data & 0x04, 0x04);

        controller.import_snapshot(StandardControllerSnapshot {
            buttons: [Buttons::empty(), Buttons::empty()],
            microphone: false,
            index1: 0,
            index2: 0,
            strobe: false,
        });
        assert_eq!(controller.read(0).data & 0x04, 0x00);
    }

    #[test]
    fn setter_helpers_update_runtime_state() {
        let mut controller = StandardController::new();

        controller.set_pad1(Buttons::A | Buttons::START);
        controller.set_pad2(Buttons::LEFT);
        controller.set_microphone(true);

        assert_eq!(
            controller.export_snapshot(),
            StandardControllerSnapshot {
                buttons: [Buttons::A | Buttons::START, Buttons::LEFT],
                microphone: true,
                index1: 0,
                index2: 0,
                strobe: false,
            }
        );
    }
}
