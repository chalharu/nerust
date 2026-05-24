use super::seed::default_desktop_settings;
use nerust_contract_settings::desktop::DesktopSettings;
use nerust_gui_runtime::settings::{DesktopSettingsManager, SettingsError};

pub fn load_settings_manager() -> DesktopSettingsManager {
    let defaults = default_desktop_settings();
    match DesktopSettingsManager::load(defaults.clone()) {
        Ok(manager) => manager,
        Err(error) => {
            log::warn!("desktop settings file is unavailable; using in-memory defaults: {error}");
            DesktopSettingsManager::ephemeral(defaults)
        }
    }
}

pub fn current_or_default(manager: &DesktopSettingsManager) -> DesktopSettings {
    manager.current().unwrap_or_else(|error| {
        log::warn!("desktop settings read failed; using defaults: {error}");
        default_desktop_settings()
    })
}

pub fn save_settings(
    manager: &DesktopSettingsManager,
    settings: DesktopSettings,
) -> Result<(), SettingsError> {
    manager.save(settings)
}

#[cfg(test)]
mod tests {
    use super::{current_or_default, default_desktop_settings};
    use nerust_gui_runtime::settings::DesktopSettingsManager;

    #[test]
    fn current_or_default_falls_back_for_ephemeral_manager_reads() {
        let manager = DesktopSettingsManager::ephemeral(default_desktop_settings());
        assert!(
            current_or_default(&manager)
                .systems
                .contains_key(&nerust_input_schema::SystemId::Nes)
        );
    }
}
