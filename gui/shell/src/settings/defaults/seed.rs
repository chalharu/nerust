use std::collections::BTreeMap;

use nerust_core_traits::SystemId;
use nerust_gui_settings::{
    app_state::DesktopAppState,
    input::{
        IMPLICIT_PROFILE_ID, KeyboardBinding, KeyboardKey, PersistedControlId, ShortcutAction,
        ShortcutBinding,
    },
    local::HostBackendLocalSettings,
    nes::NesSettings,
    shared::{DesktopSharedSettings, SystemSettings},
    snes::SnesSettings,
};
use nerust_input_traits::DigitalControlId;

const NES_ATTACHMENT_CONTROLLER_ONE: &str = "nes.attachment.player1";
const NES_CONTROL_A: DigitalControlId = DigitalControlId::new("nes.control.a");
const NES_CONTROL_B: DigitalControlId = DigitalControlId::new("nes.control.b");
const NES_CONTROL_SELECT: DigitalControlId = DigitalControlId::new("nes.control.select");
const NES_CONTROL_START: DigitalControlId = DigitalControlId::new("nes.control.start");
const NES_CONTROL_UP: DigitalControlId = DigitalControlId::new("nes.control.up");
const NES_CONTROL_DOWN: DigitalControlId = DigitalControlId::new("nes.control.down");
const NES_CONTROL_LEFT: DigitalControlId = DigitalControlId::new("nes.control.left");
const NES_CONTROL_RIGHT: DigitalControlId = DigitalControlId::new("nes.control.right");

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
        systems: BTreeMap::from([
            (
                SystemId::new("nes"),
                SystemSettings::Nes(NesSettings::default()),
            ),
            (
                SystemId::new("snes"),
                SystemSettings::Snes(SnesSettings::default()),
            ),
        ]),
        ..Default::default()
    };

    let mut nes_input = nerust_gui_settings::input::SystemInputSettings::default();
    nes_input.implicit_keyboard_profile_mut().bindings = vec![
        default_control_binding(
            NES_ATTACHMENT_CONTROLLER_ONE,
            NES_CONTROL_A,
            KeyboardKey::KeyZ,
        ),
        default_control_binding(
            NES_ATTACHMENT_CONTROLLER_ONE,
            NES_CONTROL_B,
            KeyboardKey::KeyX,
        ),
        default_control_binding(
            NES_ATTACHMENT_CONTROLLER_ONE,
            NES_CONTROL_SELECT,
            KeyboardKey::KeyC,
        ),
        default_control_binding(
            NES_ATTACHMENT_CONTROLLER_ONE,
            NES_CONTROL_START,
            KeyboardKey::KeyV,
        ),
        default_control_binding(
            NES_ATTACHMENT_CONTROLLER_ONE,
            NES_CONTROL_UP,
            KeyboardKey::ArrowUp,
        ),
        default_control_binding(
            NES_ATTACHMENT_CONTROLLER_ONE,
            NES_CONTROL_DOWN,
            KeyboardKey::ArrowDown,
        ),
        default_control_binding(
            NES_ATTACHMENT_CONTROLLER_ONE,
            NES_CONTROL_LEFT,
            KeyboardKey::ArrowLeft,
        ),
        default_control_binding(
            NES_ATTACHMENT_CONTROLLER_ONE,
            NES_CONTROL_RIGHT,
            KeyboardKey::ArrowRight,
        ),
    ];
    let _ = nes_input
        .keyboard_profiles
        .entry(IMPLICIT_PROFILE_ID.to_string())
        .or_default();
    settings
        .input
        .systems
        .insert(SystemId::new("nes"), nes_input);

    let mut snes_input = nerust_gui_settings::input::SystemInputSettings::default();
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
    settings
        .input
        .systems
        .insert(SystemId::new("snes"), snes_input);

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
    use nerust_core_traits::SystemId;
    use nerust_gui_settings::{input::ShortcutAction, shared::SystemSettings};

    use crate::test_support::TEST_CTRL_MIC;

    use super::default_shared_settings;

    #[test]
    fn default_settings_seed_nes_bindings_and_system_settings() {
        let settings = default_shared_settings();

        assert!(settings.systems.contains_key(&SystemId::new("nes")));
        assert!(settings.input.systems.contains_key(&SystemId::new("nes")));
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
                .get(&SystemId::new("nes"))
                .unwrap()
                .implicit_keyboard_profile()
                .unwrap()
                .bindings
                .iter()
                .any(|binding| binding.control.as_str() == TEST_CTRL_MIC.as_str())
        );
    }

    #[test]
    fn defaults_keep_nes_and_snes_system_settings_separate() {
        let settings = default_shared_settings();

        assert!(matches!(
            settings.systems.get(&SystemId::new("nes")),
            Some(SystemSettings::Nes(_))
        ));
        assert!(matches!(
            settings.systems.get(&SystemId::new("snes")),
            Some(SystemSettings::Snes(_))
        ));
        assert!(settings.input.systems.contains_key(&SystemId::new("nes")));
        assert!(settings.input.systems.contains_key(&SystemId::new("snes")));
    }
}
