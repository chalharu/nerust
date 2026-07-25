#[cfg(test)]
use nerust_core_traits::identity::SystemId;
#[cfg(test)]
use nerust_gui_settings::shared::DesktopSharedSettings;
#[cfg(test)]
use nerust_input_traits::DigitalInputEvent;
#[cfg(test)]
use nerust_keyboard::Key;

#[cfg(test)]
pub fn controller_event_for_key<F>(
    settings: &DesktopSharedSettings,
    system: &dyn SystemId,
    key: Key,
    pressed: bool,
    resolve: F,
) -> Option<DigitalInputEvent>
where
    F: Fn(&str, &str, bool) -> Option<DigitalInputEvent>,
{
    let profile = settings
        .input
        .systems
        .get(system)?
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
    use nerust_gui_settings::input::{KeyboardBinding, PersistedControlId};
    use nerust_keyboard::Key;

    use super::controller_event_for_key;
    use crate::test_support::{
        DummySystemId, TEST_ATT_P1, TEST_ATT_P2, TEST_CTRL_A, TEST_CTRL_MIC, test_nes_defaults,
        test_resolve,
    };

    #[test]
    fn keyboard_bindings_resolve_to_nes_input_events() {
        let settings = test_nes_defaults();
        let event =
            controller_event_for_key(&settings, &DummySystemId, Key::KeyZ, true, test_resolve)
                .unwrap();

        assert_eq!(event.attachment, TEST_ATT_P1);
        assert_eq!(event.control, TEST_CTRL_A);
    }

    #[test]
    fn keyboard_bindings_support_player_two_controls() {
        let mut settings = test_nes_defaults();
        settings
            .input
            .systems
            .get_mut(&(Box::new(DummySystemId) as Box<_>))
            .unwrap()
            .implicit_keyboard_profile_mut()
            .bindings
            .push(KeyboardBinding {
                attachment: nerust_gui_settings::input::PersistedAttachmentId::new(
                    TEST_ATT_P2.as_str(),
                ),
                control: PersistedControlId::digital(TEST_CTRL_MIC.as_str()),
                key: Key::KeyM,
            });
        let event =
            controller_event_for_key(&settings, &DummySystemId, Key::KeyM, true, test_resolve)
                .unwrap();

        assert_eq!(event.attachment, TEST_ATT_P2);
        assert_eq!(event.control, TEST_CTRL_MIC);
    }
}
