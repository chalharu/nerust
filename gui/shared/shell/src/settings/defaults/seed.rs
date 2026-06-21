use nerust_gui_settings::app_state::DesktopAppState;
use nerust_gui_settings::input::{
    IMPLICIT_PROFILE_ID, KeyboardBinding, KeyboardKey, PersistedControlId, ShortcutAction,
    ShortcutBinding,
};
use nerust_gui_settings::local::HostBackendLocalSettings;
use nerust_gui_settings::nes::NesSettings;
use nerust_gui_settings::shared::{DesktopSharedSettings, SystemSettings};
use nerust_input_schema::{DigitalControlId, SystemId};
use std::collections::BTreeMap;

pub fn default_shared_settings() -> DesktopSharedSettings {
    let mut settings = DesktopSharedSettings {
        systems: BTreeMap::from([(SystemId::Nes, SystemSettings::Nes(NesSettings::default()))]),
        ..Default::default()
    };
    let mut nes_input = nerust_gui_settings::input::SystemInputSettings::default();
    const P1: &str = "nes.attachment.player1";
    nes_input.implicit_keyboard_profile_mut().bindings = vec![
        default_control_binding(
            P1,
            DigitalControlId::new("nes.control.a"),
            KeyboardKey::KeyZ,
        ),
        default_control_binding(
            P1,
            DigitalControlId::new("nes.control.b"),
            KeyboardKey::KeyX,
        ),
        default_control_binding(
            P1,
            DigitalControlId::new("nes.control.select"),
            KeyboardKey::KeyC,
        ),
        default_control_binding(
            P1,
            DigitalControlId::new("nes.control.start"),
            KeyboardKey::KeyV,
        ),
        default_control_binding(
            P1,
            DigitalControlId::new("nes.control.up"),
            KeyboardKey::ArrowUp,
        ),
        default_control_binding(
            P1,
            DigitalControlId::new("nes.control.down"),
            KeyboardKey::ArrowDown,
        ),
        default_control_binding(
            P1,
            DigitalControlId::new("nes.control.left"),
            KeyboardKey::ArrowLeft,
        ),
        default_control_binding(
            P1,
            DigitalControlId::new("nes.control.right"),
            KeyboardKey::ArrowRight,
        ),
    ];
    let _ = nes_input
        .keyboard_profiles
        .entry(IMPLICIT_PROFILE_ID.to_string())
        .or_default();
    settings.input.systems.insert(SystemId::Nes, nes_input);
    settings.input.shortcuts.keyboard = vec![
        ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: Some(KeyboardKey::Space),
        },
        ShortcutBinding {
            action: ShortcutAction::SaveActiveSlot,
            key: Some(KeyboardKey::F5),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectNextSlot,
            key: Some(KeyboardKey::F6),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectPreviousSlot,
            key: Some(KeyboardKey::F7),
        },
        ShortcutBinding {
            action: ShortcutAction::LoadActiveSlot,
            key: Some(KeyboardKey::F8),
        },
        ShortcutBinding {
            action: ShortcutAction::ToggleFullscreen,
            key: Some(KeyboardKey::F11),
        },
        ShortcutBinding {
            action: ShortcutAction::Reset,
            key: None,
        },
    ];
    settings
}

pub fn default_local_settings() -> HostBackendLocalSettings {
    HostBackendLocalSettings::default()
}

pub fn default_app_state() -> DesktopAppState {
    DesktopAppState::default()
}

fn default_control_binding(
    attachment: &str,
    control: DigitalControlId,
    key: KeyboardKey,
) -> KeyboardBinding {
    KeyboardBinding::new(
        attachment,
        PersistedControlId::digital(control.as_str()),
        key,
    )
}

#[cfg(test)]
mod tests {
    use super::default_shared_settings;
    use nerust_gui_settings::input::ShortcutAction;
    use nerust_input_nes::topology::FAMICOM_P2_CONTROL_MICROPHONE;

    #[test]
    fn default_settings_seed_nes_bindings_and_system_settings() {
        let settings = default_shared_settings();

        assert!(
            settings
                .systems
                .contains_key(&nerust_input_schema::SystemId::Nes)
        );
        assert!(
            settings
                .input
                .systems
                .contains_key(&nerust_input_schema::SystemId::Nes)
        );
        assert!(
            settings
                .input
                .shortcuts
                .keyboard
                .iter()
                .any(|binding| binding.action == ShortcutAction::Reset && binding.key.is_none())
        );
        assert!(
            !settings
                .input
                .systems
                .get(&nerust_input_schema::SystemId::Nes)
                .unwrap()
                .implicit_keyboard_profile()
                .unwrap()
                .bindings
                .iter()
                .any(|binding| binding.control.as_str() == FAMICOM_P2_CONTROL_MICROPHONE.as_str())
        );
    }
}
