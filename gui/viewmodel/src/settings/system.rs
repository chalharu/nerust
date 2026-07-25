use nerust_core_traits::identity::SystemId;

/// Stub: per-system settings view model (to be implemented).
pub struct SystemSettingsViewModel;

impl SystemSettingsViewModel {
    pub fn system_id(&self) -> &dyn SystemId {
        panic!("SystemSettingsViewModel not yet implemented");
    }
}
