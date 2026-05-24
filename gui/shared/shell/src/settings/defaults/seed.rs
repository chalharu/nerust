use nerust_contract_settings::{
    desktop::{DesktopSettings, SystemSettings},
    input::{
        BindingProfile, ControlBinding, HostInputSource, KeyboardKey, PersistedAttachmentId,
        PersistedControlId,
    },
    nes::NesSettings,
    shortcut::{ShortcutAction, ShortcutBinding},
};
use nerust_input_nes::topology::ids::{
    NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT,
    NES_CONTROL_RIGHT, NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::{DigitalControlId, SystemId};
use std::collections::BTreeMap;

pub fn default_desktop_settings() -> DesktopSettings {
    let mut settings = DesktopSettings {
        systems: BTreeMap::from([(SystemId::Nes, SystemSettings::Nes(NesSettings::default()))]),
        ..Default::default()
    };
    settings.input.keyboard_profiles.insert(
        SystemId::Nes,
        BindingProfile {
            bindings: vec![
                default_control_binding(NES_CONTROL_A, KeyboardKey::KeyZ),
                default_control_binding(NES_CONTROL_B, KeyboardKey::KeyX),
                default_control_binding(NES_CONTROL_SELECT, KeyboardKey::KeyC),
                default_control_binding(NES_CONTROL_START, KeyboardKey::KeyV),
                default_control_binding(NES_CONTROL_UP, KeyboardKey::ArrowUp),
                default_control_binding(NES_CONTROL_DOWN, KeyboardKey::ArrowDown),
                default_control_binding(NES_CONTROL_LEFT, KeyboardKey::ArrowLeft),
                default_control_binding(NES_CONTROL_RIGHT, KeyboardKey::ArrowRight),
            ],
        },
    );
    settings.shortcuts.keyboard = vec![
        ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: KeyboardKey::Space,
        },
        ShortcutBinding {
            action: ShortcutAction::Reset,
            key: KeyboardKey::Escape,
        },
        ShortcutBinding {
            action: ShortcutAction::SaveActiveSlotOrNew,
            key: KeyboardKey::F5,
        },
        ShortcutBinding {
            action: ShortcutAction::SelectNextSlot,
            key: KeyboardKey::F6,
        },
        ShortcutBinding {
            action: ShortcutAction::SelectPreviousSlot,
            key: KeyboardKey::F7,
        },
        ShortcutBinding {
            action: ShortcutAction::LoadActiveSlot,
            key: KeyboardKey::F8,
        },
        ShortcutBinding {
            action: ShortcutAction::ToggleFullscreen,
            key: KeyboardKey::F11,
        },
    ];
    settings
}

fn default_control_binding(control: DigitalControlId, key: KeyboardKey) -> ControlBinding {
    ControlBinding {
        attachment: PersistedAttachmentId::new(NES_ATTACHMENT_PLAYER_ONE.as_str()),
        control: PersistedControlId::digital(control.as_str()),
        source: HostInputSource::Keyboard(key),
    }
}

#[cfg(test)]
mod tests {
    use super::default_desktop_settings;

    #[test]
    fn default_settings_seed_nes_bindings_and_system_settings() {
        let settings = default_desktop_settings();

        assert!(
            settings
                .systems
                .contains_key(&nerust_input_schema::SystemId::Nes)
        );
        assert!(
            settings
                .input
                .keyboard_profiles
                .contains_key(&nerust_input_schema::SystemId::Nes)
        );
    }
}
