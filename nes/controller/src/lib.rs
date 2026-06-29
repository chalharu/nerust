pub mod codec;
pub mod nes_input_cell;
pub mod persisted;
pub mod topology;
pub mod touch;

use nerust_nes_core::{
    controller::Controller,
    input_types::{Buttons, NesInputFrame},
};

use crate::codec::{decode_input_state, encode_input_state as encode_frame_input_state};

/// NES パッドのセーブステート関連のトレイト。
///
/// `Controller` トレイトがランタイムの入力インタフェースを提供するのに対し、
/// こちらはシフトレジスタ状態の保存/復元を担当する。
/// Phase 7 で旧 Console が削除されるときに同時に削除される。
pub trait ControllerState: Controller + Send {
    fn reset_runtime(&mut self);
    fn validate_controller_state(&self, bytes: &[u8]) -> Result<(), String>;
    fn apply_controller_state(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn current_controller_state(&self) -> Result<Vec<u8>, String>;
}

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

#[derive(serde::Serialize, serde::Deserialize)]
struct ControllerStatePayload {
    schema_version: u32,
    buttons: [Buttons; 2],
    microphone: bool,
    index1: u64,
    index2: u64,
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

#[cfg(test)]
mod tests {
    use nerust_nes_core::input_types::{Buttons, NesInputFrame};

    use super::{
        StandardControllerSnapshot, apply_input_state, decode_controller_state,
        encode_controller_state, encode_input_state,
    };
    use crate::codec::decode_input_state;

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
            &crate::codec::encode_input_state(NesInputFrame {
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
}
