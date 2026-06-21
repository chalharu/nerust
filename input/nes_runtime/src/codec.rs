use nerust_nes_core::input_types::{Buttons, NesInputFrame};

const INPUT_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct InputStatePayload {
    schema_version: u32,
    player_one: Buttons,
    player_two: Buttons,
    microphone: bool,
}

pub fn encode_input_state(frame: NesInputFrame) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec_named(&InputStatePayload {
        schema_version: INPUT_STATE_SCHEMA_VERSION,
        player_one: frame.player_one,
        player_two: frame.player_two,
        microphone: frame.microphone,
    })
    .map_err(|error| error.to_string())
}

pub fn decode_input_state(bytes: &[u8]) -> Result<NesInputFrame, String> {
    let payload =
        rmp_serde::from_slice::<InputStatePayload>(bytes).map_err(|error| error.to_string())?;
    if payload.schema_version != INPUT_STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported input state schema version: {}",
            payload.schema_version
        ));
    }
    Ok(NesInputFrame {
        player_one: payload.player_one,
        player_two: payload.player_two,
        microphone: payload.microphone,
    })
}

#[cfg(test)]
mod tests {
    use super::{decode_input_state, encode_input_state};
    use nerust_nes_core::input_types::{Buttons, NesInputFrame};

    #[test]
    fn input_state_round_trips() {
        let frame = NesInputFrame {
            player_one: Buttons::A | Buttons::START,
            player_two: Buttons::LEFT,
            microphone: true,
        };

        let decoded = decode_input_state(&encode_input_state(frame).expect("encode succeeds"))
            .expect("decode succeeds");

        assert_eq!(decoded, frame);
    }
}
