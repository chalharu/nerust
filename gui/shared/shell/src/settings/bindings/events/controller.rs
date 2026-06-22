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
    use nerust_input_schema::{
        AttachmentId, DigitalControlId, DigitalInputEvent, DigitalInputState, SystemId,
    };

    fn test_resolve(attachment: &str, control: &str, pressed: bool) -> Option<DigitalInputEvent> {
        Some(DigitalInputEvent::new(
            AttachmentId::new(Box::leak(attachment.to_string().into_boxed_str())),
            DigitalControlId::new(Box::leak(control.to_string().into_boxed_str())),
            if pressed {
                DigitalInputState::Pressed
            } else {
                DigitalInputState::Released
            },
        ))
    }

    #[test]
    fn keyboard_bindings_resolve_to_nes_input_events() {
        let settings = default_shared_settings();
        let event = controller_event_for_key(
            &settings,
            SystemId::Nes,
            KeyboardKey::KeyZ,
            true,
            test_resolve,
        )
        .unwrap();

        assert_eq!(
            event.attachment,
            AttachmentId::new("nes.attachment.player1")
        );
        assert_eq!(event.control, DigitalControlId::new("nes.control.a"));
    }

    #[test]
    fn keyboard_bindings_support_player_two_controls() {
        let mut settings = default_shared_settings();
        settings
            .input
            .systems
            .get_mut(&SystemId::Nes)
            .unwrap()
            .implicit_keyboard_profile_mut()
            .bindings
            .push(KeyboardBinding {
                attachment: nerust_gui_settings::input::PersistedAttachmentId::new(
                    "nes.attachment.player2",
                ),
                control: PersistedControlId::digital("nes.control.microphone"),
                key: KeyboardKey::KeyM,
            });
        let event = controller_event_for_key(
            &settings,
            SystemId::Nes,
            KeyboardKey::KeyM,
            true,
            test_resolve,
        )
        .unwrap();

        assert_eq!(
            event.attachment,
            AttachmentId::new("nes.attachment.player2")
        );
        assert_eq!(
            event.control,
            DigitalControlId::new("nes.control.microphone")
        );
    }
}
