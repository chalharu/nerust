use nerust_input_nes::codec::{decode_input_state, encode_input_state as encode_frame_input_state};
use nerust_input_nes::frame::{Buttons, NesInputFrame};
use nerust_nes_core::OpenBusReadResult;

const STANDARD_CONTROLLER_MAX_INDEX: usize = 8;
const CONTROLLER_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandardControllerState {
    pub buttons: [Buttons; 2],
    pub microphone: bool,
    pub index1: usize,
    pub index2: usize,
    pub strobe: bool,
}

impl Default for StandardControllerState {
    fn default() -> Self {
        Self {
            buttons: [Buttons::empty(); 2],
            microphone: false,
            index1: 0,
            index2: 0,
            strobe: false,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ControllerStatePayload {
    schema_version: u32,
    buttons: [Buttons; 2],
    microphone: bool,
    index1: u64,
    index2: u64,
    strobe: bool,
}

pub fn input_frame_from_controller_state(snapshot: StandardControllerState) -> NesInputFrame {
    NesInputFrame {
        player_one: snapshot.buttons[0],
        player_two: snapshot.buttons[1],
        microphone: snapshot.microphone,
    }
}

pub fn controller_state_with_input_frame(
    snapshot: StandardControllerState,
    frame: NesInputFrame,
) -> StandardControllerState {
    StandardControllerState {
        buttons: [frame.player_one, frame.player_two],
        microphone: frame.microphone,
        ..snapshot
    }
}

pub fn encode_standard_controller_state(
    snapshot: StandardControllerState,
) -> Result<Vec<u8>, String> {
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

pub fn decode_standard_controller_state(bytes: &[u8]) -> Result<StandardControllerState, String> {
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
    Ok(StandardControllerState {
        buttons: payload.buttons,
        microphone: payload.microphone,
        index1,
        index2,
        strobe: payload.strobe,
    })
}

pub fn encode_standard_input_state(snapshot: StandardControllerState) -> Result<Vec<u8>, String> {
    encode_frame_input_state(input_frame_from_controller_state(snapshot))
}

pub fn apply_standard_input_state(
    snapshot: StandardControllerState,
    bytes: &[u8],
) -> Result<StandardControllerState, String> {
    let frame = decode_input_state(bytes)?;
    Ok(controller_state_with_input_frame(snapshot, frame))
}

pub fn read_standard_controller_port(
    state: &mut StandardControllerState,
    address: usize,
) -> OpenBusReadResult {
    match address {
        0 => OpenBusReadResult::new(
            if state.index1 < 8 {
                let result = state.buttons[0].bits() >> state.index1;
                if !state.strobe {
                    state.index1 += 1;
                }
                result & 1
            } else {
                1
            } | (if state.microphone { 0x04 } else { 0 }),
            7,
        ),
        1 => OpenBusReadResult::new(
            if state.index2 < 8 {
                let result = state.buttons[1].bits() >> state.index2;
                if !state.strobe {
                    state.index2 += 1;
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

pub fn write_standard_controller_port(state: &mut StandardControllerState, value: u8) {
    state.strobe = value & 1 == 1;
    if state.strobe {
        state.index1 = 0;
        state.index2 = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        StandardControllerState, apply_standard_input_state, decode_standard_controller_state,
        encode_standard_controller_state, encode_standard_input_state,
        read_standard_controller_port, write_standard_controller_port,
    };
    use nerust_input_nes::codec::decode_input_state;
    use nerust_input_nes::frame::{Buttons, NesInputFrame};

    #[test]
    fn controller_state_round_trips() {
        let snapshot = StandardControllerState {
            buttons: [Buttons::A | Buttons::START, Buttons::LEFT],
            microphone: true,
            index1: 3,
            index2: 5,
            strobe: true,
        };

        let decoded = decode_standard_controller_state(
            &encode_standard_controller_state(snapshot).expect("encode succeeds"),
        )
        .expect("decode succeeds");

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn encoded_input_state_round_trips_from_snapshot() {
        let snapshot = StandardControllerState {
            buttons: [Buttons::A | Buttons::START, Buttons::LEFT],
            microphone: true,
            index1: 3,
            index2: 5,
            strobe: true,
        };

        let decoded =
            decode_input_state(&encode_standard_input_state(snapshot).expect("encode succeeds"))
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
        let base = StandardControllerState {
            buttons: [Buttons::empty(), Buttons::LEFT],
            microphone: true,
            index1: 3,
            index2: 5,
            strobe: true,
        };

        let snapshot = apply_standard_input_state(
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
            StandardControllerState {
                buttons: [Buttons::A, Buttons::RIGHT],
                microphone: false,
                index1: 3,
                index2: 5,
                strobe: true,
            }
        );
    }

    #[test]
    fn controller_ports_return_one_after_eight_bits() {
        let mut state = StandardControllerState {
            buttons: [Buttons::A, Buttons::empty()],
            microphone: false,
            index1: 0,
            index2: 0,
            strobe: false,
        };

        write_standard_controller_port(&mut state, 1);
        write_standard_controller_port(&mut state, 0);

        for _ in 0..8 {
            let _ = read_standard_controller_port(&mut state, 0);
        }

        assert_eq!(read_standard_controller_port(&mut state, 0).data & 1, 1);
        assert_eq!(read_standard_controller_port(&mut state, 0).data & 1, 1);
    }
}
