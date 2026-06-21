use nerust_gui_settings::input::KeyboardKey;
use nerust_gui_settings::shared::DesktopSharedSettings;
use nerust_input_schema::{DigitalInputEvent, SystemId};

pub fn controller_event_for_key<F>(
    settings: &DesktopSharedSettings,
    system: SystemId,
    key: KeyboardKey,
    pressed: bool,
    resolve: F,
) -> Option<DigitalInputEvent>
where
    F: Fn(&str, &str, bool) -> Option<DigitalInputEvent>,
{
    let profile = settings
        .input
        .systems
        .get(&system)?
        .implicit_keyboard_profile()?;
    profile
        .bindings
        .iter()
        .find(|binding| binding.key == key)
        .and_then(|binding| {
            resolve(
                binding.attachment.as_str(),
                binding.control.as_str(),
                pressed,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::controller_event_for_key;
    use crate::settings::defaults::seed::default_shared_settings;
    use nerust_gui_settings::input::{KeyboardBinding, KeyboardKey, PersistedControlId};
    use nerust_input_nes_runtime::topology::{
        FAMICOM_P2_CONTROL_MICROPHONE, NES_ATTACHMENT_PLAYER_ONE, NES_ATTACHMENT_PLAYER_TWO,
        NES_CONTROL_A,
    };

    #[test]
    fn keyboard_bindings_resolve_to_nes_input_events() {
        let settings = default_shared_settings();
        let event = controller_event_for_key(
            &settings,
            nerust_input_schema::SystemId::Nes,
            KeyboardKey::KeyZ,
            true,
            nerust_input_nes_runtime::persisted::digital_event_from_persisted_ids,
        )
        .unwrap();

        assert_eq!(event.attachment, NES_ATTACHMENT_PLAYER_ONE);
        assert_eq!(event.control, NES_CONTROL_A);
    }

    #[test]
    fn keyboard_bindings_support_player_two_controls() {
        let mut settings = default_shared_settings();
        settings
            .input
            .systems
            .get_mut(&nerust_input_schema::SystemId::Nes)
            .unwrap()
            .implicit_keyboard_profile_mut()
            .bindings
            .push(KeyboardBinding {
                attachment: nerust_gui_settings::input::PersistedAttachmentId::new(
                    NES_ATTACHMENT_PLAYER_TWO.as_str(),
                ),
                control: PersistedControlId::digital(FAMICOM_P2_CONTROL_MICROPHONE.as_str()),
                key: KeyboardKey::KeyM,
            });
        let event = controller_event_for_key(
            &settings,
            nerust_input_schema::SystemId::Nes,
            KeyboardKey::KeyM,
            true,
            nerust_input_nes_runtime::persisted::digital_event_from_persisted_ids,
        )
        .unwrap();

        assert_eq!(event.attachment, NES_ATTACHMENT_PLAYER_TWO);
        assert_eq!(event.control, FAMICOM_P2_CONTROL_MICROPHONE);
    }
}
