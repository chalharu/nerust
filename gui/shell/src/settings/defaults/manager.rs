use nerust_gui_runtime::settings::{SettingsSnapshot, manager::SettingsManager};

use super::seed::{default_app_state, default_local_settings, default_shared_settings};

pub fn current_or_default(manager: &SettingsManager) -> SettingsSnapshot {
    manager.snapshot().unwrap_or_else(|error| {
        log::warn!("settings read failed; using defaults: {error}");
        SettingsSnapshot {
            shared: default_shared_settings(&[]),
            local: default_local_settings(),
            app_state: default_app_state(),
        }
    })
}

#[cfg(test)]
mod tests {
    use nerust_core_traits::identity::SystemId;
    use nerust_gui_runtime::settings::manager::SettingsManager;

    use super::{current_or_default, default_app_state, default_local_settings};
    use crate::settings::defaults::seed::test_nes_defaults;

    #[test]
    fn current_or_default_falls_back_for_ephemeral_manager_reads() {
        let settings = test_nes_defaults();
        let manager =
            SettingsManager::ephemeral(settings, default_local_settings(), default_app_state());
        assert!(
            current_or_default(&manager)
                .shared
                .systems
                .contains_key(&SystemId::new("nes"))
        );
    }
}
