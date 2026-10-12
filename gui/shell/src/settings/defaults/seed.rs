use std::sync::Arc;

use nerust_core_traits::factory::CoreFactory;
use nerust_gui_settings::{
    app_state::DesktopAppState, input::IMPLICIT_PROFILE_ID, local::HostBackendLocalSettings,
    shared::DesktopSharedSettings,
};

pub fn default_shared_settings(factories: &[Arc<dyn CoreFactory>]) -> DesktopSharedSettings {
    let mut settings = DesktopSharedSettings::default();
    for factory in factories {
        let Some(sd) = factory.as_system_defaults() else {
            continue;
        };
        let sid = factory.system_id();
        if let Some(sys_settings) = sd.default_system_settings() {
            settings.systems.insert(sid.clone(), sys_settings);
        }
        if let Some(attachment) = sd.default_input_attachment_id()
            && let Some(control_prefix) = sd.default_input_control_prefix()
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

fn seed_global_shortcuts(settings: &mut DesktopSharedSettings) {
    settings.input.shortcuts.keyboard = nerust_gui_settings::input::default_shortcut_bindings();
}

pub fn default_local_settings() -> HostBackendLocalSettings {
    HostBackendLocalSettings::default()
}

pub fn default_app_state() -> DesktopAppState {
    DesktopAppState::default()
}

#[cfg(test)]
mod tests {
    use nerust_gui_settings::input::ShortcutAction;
    use nerust_keyboard::Key;

    use super::default_shared_settings;

    #[test]
    fn default_settings_include_global_shortcuts() {
        let settings = default_shared_settings(&[]);
        assert!(
            settings
                .input
                .shortcuts
                .keyboard
                .iter()
                .any(|binding| binding.action == ShortcutAction::Reset && binding.key.is_none())
        );
    }

    #[test]
    fn debugger_shortcut_defaults_to_f4() {
        let settings = default_shared_settings(&[]);
        assert!(
            settings
                .input
                .shortcuts
                .keyboard
                .iter()
                .any(|binding| binding.action == ShortcutAction::OpenDebugger
                    && binding.key == Some(Key::F4))
        );
    }
}
