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

const P1: &str = "nes.attachment.player1";
const A: DigitalControlId = DigitalControlId::new("nes.control.a");
const B: DigitalControlId = DigitalControlId::new("nes.control.b");
const SELECT: DigitalControlId = DigitalControlId::new("nes.control.select");
const START: DigitalControlId = DigitalControlId::new("nes.control.start");
const UP: DigitalControlId = DigitalControlId::new("nes.control.up");
const DOWN: DigitalControlId = DigitalControlId::new("nes.control.down");
const LEFT: DigitalControlId = DigitalControlId::new("nes.control.left");
const RIGHT: DigitalControlId = DigitalControlId::new("nes.control.right");

pub fn default_shared_settings() -> DesktopSharedSettings {
    let mut settings = DesktopSharedSettings {
        systems: BTreeMap::from([(SystemId::Nes, SystemSettings::Nes(NesSettings::default()))]),
        ..Default::default()
    };
    let mut nes_input = nerust_gui_settings::input::SystemInputSettings::default();
    nes_input.implicit_keyboard_profile_mut().bindings = vec![
        default_control_binding(P1, A, KeyboardKey::KeyZ),
        default_control_binding(P1, B, KeyboardKey::KeyX),
        default_control_binding(P1, SELECT, KeyboardKey::KeyC),
        default_control_binding(P1, START, KeyboardKey::KeyV),
        default_control_binding(P1, UP, KeyboardKey::ArrowUp),
        default_control_binding(P1, DOWN, KeyboardKey::ArrowDown),
        default_control_binding(P1, LEFT, KeyboardKey::ArrowLeft),
        default_control_binding(P1, RIGHT, KeyboardKey::ArrowRight),
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
                .any(|binding| binding.control.as_str() == "nes.control.microphone")
        );
    }
}
