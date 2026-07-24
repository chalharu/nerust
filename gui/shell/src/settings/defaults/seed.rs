use std::sync::Arc;

use nerust_core_traits::factory::CoreFactory;
use nerust_gui_settings::{
    app_state::DesktopAppState,
    input::{IMPLICIT_PROFILE_ID, ShortcutAction, ShortcutBinding},
    local::HostBackendLocalSettings,
    shared::DesktopSharedSettings,
};
use nerust_keyboard::Key;

pub fn default_shared_settings(factories: &[Arc<dyn CoreFactory>]) -> DesktopSharedSettings {
    let mut settings = DesktopSharedSettings::default();
    #[cfg(test)]
    if factories.is_empty() {
        seed_nes_defaults(&mut settings);
    }
    for factory in factories {
        let sid = factory.system_id();
        if let Some(sys_settings) = factory.default_system_settings() {
            settings.systems.insert(sid, sys_settings);
        }
        if let Some(attachment) = factory.default_input_attachment_id()
            && let Some(control_prefix) = factory.default_input_control_prefix()
        {
            let mut input = nerust_gui_settings::input::SystemInputSettings::default();
            input.implicit_keyboard_profile_mut().bindings =
                crate::keyboard_defaults::default_system_bindings(attachment, control_prefix);
            let _ = input
                .keyboard_profiles
                .entry(IMPLICIT_PROFILE_ID.to_string())
                .or_default();
            settings.input.systems.insert(sid, input);
        }
    }
    seed_global_shortcuts(&mut settings);
    settings
}

#[cfg(test)]
fn seed_nes_defaults(settings: &mut DesktopSharedSettings) {
    use nerust_core_traits::identity::SystemId;
    settings.systems.insert(
        SystemId::new("nes"),
        Box::new(nerust_nes_settings::NesSettings::default())
            as Box<dyn nerust_settings_traits::SystemSettings>,
    );
    let mut nes_input = nerust_gui_settings::input::SystemInputSettings::default();
    nes_input.implicit_keyboard_profile_mut().bindings =
        crate::keyboard_defaults::default_system_bindings("nes.attachment.player1", "nes.control");
    let _ = nes_input
        .keyboard_profiles
        .entry(IMPLICIT_PROFILE_ID.to_string())
        .or_default();
    settings
        .input
        .systems
        .insert(SystemId::new("nes"), nes_input);
}

fn seed_global_shortcuts(settings: &mut DesktopSharedSettings) {
    settings.input.shortcuts.keyboard = vec![
        ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: Some(Key::Space),
        },
        ShortcutBinding {
            action: ShortcutAction::SaveActiveSlot,
            key: Some(Key::F5),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectNextSlot,
            key: Some(Key::F6),
        },
        ShortcutBinding {
            action: ShortcutAction::SelectPreviousSlot,
            key: Some(Key::F7),
        },
        ShortcutBinding {
            action: ShortcutAction::LoadActiveSlot,
            key: Some(Key::F8),
        },
        ShortcutBinding {
            action: ShortcutAction::ToggleFullscreen,
            key: Some(Key::F11),
        },
        ShortcutBinding {
            action: ShortcutAction::Reset,
            key: None,
        },
    ];
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
        let settings = default_shared_settings(&[]);

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
