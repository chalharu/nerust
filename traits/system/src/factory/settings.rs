use nerust_settings_traits::SystemSettings;

/// Language preference for UI labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    SystemDefault,
    Japanese,
    English,
}

/// Factory settings without a dependency on the GUI's settings snapshot.
pub struct FactorySettingsView {
    pub language: Language,
    pub system_config: Option<Box<dyn SystemSettings>>,
}
