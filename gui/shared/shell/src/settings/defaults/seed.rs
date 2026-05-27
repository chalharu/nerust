use nerust_contract_settings::app_state::DesktopAppState;
use nerust_contract_settings::input::{
    IMPLICIT_PROFILE_ID, KeyboardBinding, KeyboardKey, PersistedControlId, ShortcutAction,
    ShortcutBinding,
};
use nerust_contract_settings::local::HostBackendLocalSettings;
use nerust_contract_settings::nes::NesSettings;
use nerust_contract_settings::shared::{DesktopSharedSettings, SystemSettings};
use nerust_input_nes::topology::{
    NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT,
    NES_CONTROL_RIGHT, NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::{DigitalControlId, SystemId};
use std::collections::BTreeMap;

const SNES_ATTACHMENT_CONTROLLER_ONE: &str = "snes.attachment.controller1";
const SNES_CONTROL_B: DigitalControlId = DigitalControlId::new("snes.control.b");
const SNES_CONTROL_Y: DigitalControlId = DigitalControlId::new("snes.control.y");
const SNES_CONTROL_SELECT: DigitalControlId = DigitalControlId::new("snes.control.select");
const SNES_CONTROL_START: DigitalControlId = DigitalControlId::new("snes.control.start");
const SNES_CONTROL_UP: DigitalControlId = DigitalControlId::new("snes.control.up");
const SNES_CONTROL_DOWN: DigitalControlId = DigitalControlId::new("snes.control.down");
const SNES_CONTROL_LEFT: DigitalControlId = DigitalControlId::new("snes.control.left");
const SNES_CONTROL_RIGHT: DigitalControlId = DigitalControlId::new("snes.control.right");
const SNES_CONTROL_A: DigitalControlId = DigitalControlId::new("snes.control.a");
const SNES_CONTROL_X: DigitalControlId = DigitalControlId::new("snes.control.x");
const SNES_CONTROL_L: DigitalControlId = DigitalControlId::new("snes.control.l");
const SNES_CONTROL_R: DigitalControlId = DigitalControlId::new("snes.control.r");

pub fn default_shared_settings() -> DesktopSharedSettings {
    let mut settings = DesktopSharedSettings {
        systems: BTreeMap::from([(SystemId::Nes, SystemSettings::Nes(NesSettings::default()))]),
        ..Default::default()
    };
    let mut nes_input = nerust_contract_settings::input::SystemInputSettings::default();
    nes_input.implicit_keyboard_profile_mut().bindings = vec![
        default_control_binding(
            NES_ATTACHMENT_PLAYER_ONE.as_str(),
            NES_CONTROL_A,
            KeyboardKey::KeyZ,
        ),
        default_control_binding(
            NES_ATTACHMENT_PLAYER_ONE.as_str(),
            NES_CONTROL_B,
            KeyboardKey::KeyX,
        ),
        default_control_binding(
            NES_ATTACHMENT_PLAYER_ONE.as_str(),
            NES_CONTROL_SELECT,
            KeyboardKey::KeyC,
        ),
        default_control_binding(
            NES_ATTACHMENT_PLAYER_ONE.as_str(),
            NES_CONTROL_START,
            KeyboardKey::KeyV,
        ),
        default_control_binding(
            NES_ATTACHMENT_PLAYER_ONE.as_str(),
            NES_CONTROL_UP,
            KeyboardKey::ArrowUp,
        ),
        default_control_binding(
            NES_ATTACHMENT_PLAYER_ONE.as_str(),
            NES_CONTROL_DOWN,
            KeyboardKey::ArrowDown,
        ),
        default_control_binding(
            NES_ATTACHMENT_PLAYER_ONE.as_str(),
            NES_CONTROL_LEFT,
            KeyboardKey::ArrowLeft,
        ),
        default_control_binding(
            NES_ATTACHMENT_PLAYER_ONE.as_str(),
            NES_CONTROL_RIGHT,
            KeyboardKey::ArrowRight,
        ),
    ];
    let _ = nes_input
        .keyboard_profiles
        .entry(IMPLICIT_PROFILE_ID.to_string())
        .or_default();
    settings.input.systems.insert(SystemId::Nes, nes_input);
    let mut snes_input = nerust_contract_settings::input::SystemInputSettings::default();
    snes_input.implicit_keyboard_profile_mut().bindings = vec![
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_B,
            KeyboardKey::KeyZ,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_Y,
            KeyboardKey::KeyX,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_SELECT,
            KeyboardKey::KeyC,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_START,
            KeyboardKey::KeyV,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_UP,
            KeyboardKey::ArrowUp,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_DOWN,
            KeyboardKey::ArrowDown,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_LEFT,
            KeyboardKey::ArrowLeft,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_RIGHT,
            KeyboardKey::ArrowRight,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_A,
            KeyboardKey::KeyA,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_X,
            KeyboardKey::KeyS,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_L,
            KeyboardKey::KeyQ,
        ),
        default_control_binding(
            SNES_ATTACHMENT_CONTROLLER_ONE,
            SNES_CONTROL_R,
            KeyboardKey::KeyW,
        ),
    ];
    let _ = snes_input
        .keyboard_profiles
        .entry(IMPLICIT_PROFILE_ID.to_string())
        .or_default();
    settings.input.systems.insert(SystemId::Snes, snes_input);
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
    use nerust_contract_settings::input::ShortcutAction;
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
