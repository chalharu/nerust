use crate::topology::ids::{
    FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
    NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT, NES_CONTROL_RIGHT,
    NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::{DigitalInputEvent, DigitalInputState};

pub fn digital_event_from_persisted_ids(
    attachment_id: &str,
    control_id: &str,
    pressed: bool,
) -> Option<DigitalInputEvent> {
    let attachment = match attachment_id {
        value if value == NES_ATTACHMENT_PLAYER_ONE.as_str() => NES_ATTACHMENT_PLAYER_ONE,
        value if value == NES_ATTACHMENT_PLAYER_TWO.as_str() => NES_ATTACHMENT_PLAYER_TWO,
        _ => return None,
    };
    let control = match (attachment, control_id) {
        (_, value) if value == NES_CONTROL_A.as_str() => NES_CONTROL_A,
        (_, value) if value == NES_CONTROL_B.as_str() => NES_CONTROL_B,
        (NES_ATTACHMENT_PLAYER_ONE, value) if value == NES_CONTROL_SELECT.as_str() => {
            NES_CONTROL_SELECT
        }
        (NES_ATTACHMENT_PLAYER_ONE, value) if value == NES_CONTROL_START.as_str() => {
            NES_CONTROL_START
        }
        (_, value) if value == NES_CONTROL_UP.as_str() => NES_CONTROL_UP,
        (_, value) if value == NES_CONTROL_DOWN.as_str() => NES_CONTROL_DOWN,
        (_, value) if value == NES_CONTROL_LEFT.as_str() => NES_CONTROL_LEFT,
        (_, value) if value == NES_CONTROL_RIGHT.as_str() => NES_CONTROL_RIGHT,
        (NES_ATTACHMENT_PLAYER_TWO, value) if value == FAMICOM_P2_CONTROL_MICROPHONE.as_str() => {
            FAMICOM_P2_CONTROL_MICROPHONE
        }
        _ => return None,
    };
    Some(DigitalInputEvent::new(
        attachment,
        control,
        if pressed {
            DigitalInputState::Pressed
        } else {
            DigitalInputState::Released
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::digital_event_from_persisted_ids;
    use crate::topology::ids::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_TWO, NES_CONTROL_SELECT,
    };

    #[test]
    fn persisted_ids_reject_player_two_select_and_accept_microphone() {
        assert!(
            digital_event_from_persisted_ids(
                NES_ATTACHMENT_PLAYER_TWO.as_str(),
                FAMICOM_P2_CONTROL_MICROPHONE.as_str(),
                true
            )
            .is_some()
        );
        assert!(
            digital_event_from_persisted_ids(
                NES_ATTACHMENT_PLAYER_TWO.as_str(),
                NES_CONTROL_SELECT.as_str(),
                true
            )
            .is_none()
        );
    }
}
