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
/// The factory serializes/deserializes system-specific config bytes.
pub struct FactorySettingsView {
    pub language: Language,
    /// Opaque serialized system-specific configuration.
    /// For NES: serialized `NesSettings` from gui/settings.
    pub system_config: Option<Box<dyn SystemSettings>>,
}
