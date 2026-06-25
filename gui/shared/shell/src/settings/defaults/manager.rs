use nerust_gui_runtime::settings::{
    HostBackendIdentity, SettingsError, SettingsSnapshot, manager::SettingsManager,
};

use super::seed::{default_app_state, default_local_settings, default_shared_settings};

pub fn load_settings_manager(identity: HostBackendIdentity) -> SettingsManager {
    SettingsManager::load_or_ephemeral(
        identity,
        default_shared_settings(),
        default_local_settings(),
        default_app_state(),
    )
}

pub fn current_or_default(manager: &SettingsManager) -> SettingsSnapshot {
    manager.snapshot().unwrap_or_else(|error| {
        log::warn!("settings read failed; using defaults: {error}");
        SettingsSnapshot {
            shared: default_shared_settings(),
            local: default_local_settings(),
            app_state: default_app_state(),
        }
    })
}

pub fn save_settings(
    manager: &SettingsManager,
    settings: SettingsSnapshot,
) -> Result<(), SettingsError> {
    manager.save_snapshot(settings)
}

#[cfg(test)]
mod tests {
    use nerust_gui_runtime::settings::manager::SettingsManager;

    use super::{
        current_or_default, default_app_state, default_local_settings, default_shared_settings,
    };

    #[test]
    fn current_or_default_falls_back_for_ephemeral_manager_reads() {
        let manager = SettingsManager::ephemeral(
            default_shared_settings(),
            default_local_settings(),
            default_app_state(),
        );
        assert!(
            current_or_default(&manager)
                .shared
                .systems
                .contains_key(&nerust_contract_input::SystemId::new("nes"))
        );
    }
}
