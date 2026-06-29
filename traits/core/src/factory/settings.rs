/// Lightweight settings view passed to CoreFactory methods.
///
/// Avoids coupling CoreFactory to gui/runtime's SettingsSnapshot.
/// The factory serializes/deserializes system-specific config bytes.
pub struct FactorySettingsView {
    /// Language preference: 0=SystemDefault, 1=Japanese, 2=English.
    pub language: u8,
    /// Opaque serialized system-specific configuration.
    /// For NES: serialized `NesSettings` from gui/settings.
    pub system_config_bytes: Vec<u8>,
}
