use std::collections::BTreeMap;

use keyboard_types::Code;
use nerust_core_traits::identity::SystemId;
use nerust_gui_settings::{
    app_state::DesktopAppState,
    input::{IMPLICIT_PROFILE_ID, ShortcutAction, ShortcutBinding},
    local::HostBackendLocalSettings,
    nes::NesSettings,
    shared::{DesktopSharedSettings, SystemSettings},
};
pub fn default_shared_settings() -> DesktopSharedSettings {
    let mut settings = DesktopSharedSettings {
        systems: BTreeMap::from([(
            SystemId::new("nes"),
            SystemSettings::Nes(NesSettings::default()),
        )]),
        ..Default::default()
    };
    let mut nes_input = nerust_gui_settings::input::SystemInputSettings::default();
    nes_input.implicit_keyboard_profile_mut().bindings =
        crate::keyboard_defaults::default_nes_bindings();
    let _ = nes_input
        .keyboard_profiles
        .entry(IMPLICIT_PROFILE_ID.to_string())
        .or_default();
    settings
        .input
        .systems
        .insert(SystemId::new("nes"), nes_input);
    settings.input.shortcuts.keyboard = vec![
        ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: Some(Code::Space),
        },
        ShortcutBinding {
            action: ShortcutAction::SaveActiveSlot,
            key: Some(Code::F5),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectNextSlot,
            key: Some(Code::F6),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectPreviousSlot,
            key: Some(Code::F7),
        },
        ShortcutBinding {
            action: ShortcutAction::LoadActiveSlot,
            key: Some(Code::F8),
        },
        ShortcutBinding {
            action: ShortcutAction::ToggleFullscreen,
            key: Some(Code::F11),
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

#[cfg(test)]
mod tests {
    use nerust_core_traits::identity::SystemId;
    use nerust_gui_settings::input::ShortcutAction;

    use super::default_shared_settings;
    use crate::test_support::TEST_CTRL_MIC;

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
}
