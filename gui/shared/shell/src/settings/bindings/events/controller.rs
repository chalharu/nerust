use nerust_contract_settings::{
    desktop::DesktopSettings,
    input::{HostInputSource, KeyboardKey},
};
use nerust_input_nes::input::persisted::digital_event_from_persisted_ids;
use nerust_input_schema::{DigitalInputEvent, SystemId};

pub fn controller_event_for_key(
    settings: &DesktopSettings,
    key: KeyboardKey,
    pressed: bool,
) -> Option<DigitalInputEvent> {
    let profile = settings.input.keyboard_profiles.get(&SystemId::Nes)?;
    profile
        .bindings
        .iter()
        .find_map(|binding| match &binding.source {
            HostInputSource::Keyboard(binding_key) if *binding_key == key => {
                digital_event_from_persisted_ids(
                    binding.attachment.as_str(),
                    binding.control.as_str(),
                    pressed,
                )
            }
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::controller_event_for_key;
    use crate::settings::defaults::seed::default_desktop_settings;
    use nerust_contract_settings::input::{
        ControlBinding, HostInputSource, KeyboardKey, PersistedAttachmentId, PersistedControlId,
    };
    use nerust_input_nes::topology::ids::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A,
    };

    #[test]
    fn keyboard_bindings_resolve_to_nes_input_events() {
        let settings = default_desktop_settings();
        let event = controller_event_for_key(&settings, KeyboardKey::KeyZ, true).unwrap();

        assert_eq!(event.attachment, NES_ATTACHMENT_PLAYER_ONE);
        assert_eq!(event.control, NES_CONTROL_A);
    }

    #[test]
    fn keyboard_bindings_support_player_two_controls() {
        let mut settings = default_desktop_settings();
        settings
            .input
            .keyboard_profiles
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap()
            .bindings
            .push(ControlBinding {
                attachment: PersistedAttachmentId::new(NES_ATTACHMENT_PLAYER_TWO.as_str()),
                control: PersistedControlId::digital(FAMICOM_P2_CONTROL_MICROPHONE.as_str()),
                source: HostInputSource::Keyboard(KeyboardKey::KeyM),
            });
        let event = controller_event_for_key(&settings, KeyboardKey::KeyM, true).unwrap();

        assert_eq!(event.attachment, NES_ATTACHMENT_PLAYER_TWO);
        assert_eq!(event.control, FAMICOM_P2_CONTROL_MICROPHONE);
    }
}
