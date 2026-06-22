use nerust_contract_input::{DigitalInputEvent, SystemId};
use nerust_gui_settings::input::KeyboardKey;
use nerust_gui_settings::shared::DesktopSharedSettings;

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
    use crate::test_support::{TEST_ATT_P1, TEST_ATT_P2, TEST_CTRL_A, TEST_CTRL_MIC, test_resolve};
    use nerust_contract_input::SystemId;
    use nerust_gui_settings::input::{KeyboardBinding, KeyboardKey, PersistedControlId};

    #[test]
    fn keyboard_bindings_resolve_to_nes_input_events() {
        let settings = default_shared_settings();
        let event = controller_event_for_key(
            &settings,
            SystemId::new("nes"),
            KeyboardKey::KeyZ,
            true,
            test_resolve,
        )
        .unwrap();

        assert_eq!(event.attachment, TEST_ATT_P1);
        assert_eq!(event.control, TEST_CTRL_A);
    }

    #[test]
    fn keyboard_bindings_support_player_two_controls() {
        let mut settings = default_shared_settings();
        settings
            .input
            .systems
            .get_mut(&SystemId::new("nes"))
            .unwrap()
            .implicit_keyboard_profile_mut()
            .bindings
            .push(KeyboardBinding {
                attachment: nerust_gui_settings::input::PersistedAttachmentId::new(
                    TEST_ATT_P2.as_str(),
                ),
                control: PersistedControlId::digital(TEST_CTRL_MIC.as_str()),
                key: KeyboardKey::KeyM,
            });
        let event = controller_event_for_key(
            &settings,
            SystemId::new("nes"),
            KeyboardKey::KeyM,
            true,
            test_resolve,
        )
        .unwrap();

        assert_eq!(event.attachment, TEST_ATT_P2);
        assert_eq!(event.control, TEST_CTRL_MIC);
    }
}
