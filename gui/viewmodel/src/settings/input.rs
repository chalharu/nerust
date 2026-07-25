use nerust_core_traits::identity::SystemId;

/// Stub: per-system input settings view model (to be implemented).
pub struct InputSettingsViewModel;

impl InputSettingsViewModel {
    pub fn system_id(&self) -> &dyn SystemId {
        panic!("InputSettingsViewModel not yet implemented");
    }
}
