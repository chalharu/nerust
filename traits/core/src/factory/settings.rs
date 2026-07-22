use nerust_settings_traits::SystemSettings;

/// Language preference for UI labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    SystemDefault,
    Japanese,
    English,
}

/// Lightweight settings view passed to CoreFactory methods.
///
/// Avoids coupling CoreFactory to gui/runtime's SettingsSnapshot.
/// The factory downcasts the trait object to its concrete system type.
pub struct FactorySettingsView {
    pub language: Language,
    /// System-specific configuration as a trait object.
    /// For NES: downcast to `NesSettings`.
    pub system_config: Option<Box<dyn SystemSettings>>,
}
