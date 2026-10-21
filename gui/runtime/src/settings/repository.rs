use nerust_settings_core::model::snapshot::SettingsSnapshot;

use super::{SettingsDocument, SettingsError, SettingsPaths};

/// Low-level persistence backend for settings.
///
/// Implementations handle the actual I/O (file system, in-memory, or test double).
pub trait SettingsStore: std::fmt::Debug + Send + Sync {
    fn save(&self, document: &SettingsDocument) -> Result<(), SettingsError>;
    fn load(&self, defaults: &SettingsSnapshot) -> (SettingsSnapshot, SettingsDocument);
    fn paths(&self) -> Option<SettingsPaths>;
}

/// File-based settings store.
#[derive(Debug)]
pub(super) struct FileBackedStore(pub(super) SettingsPaths);

impl SettingsStore for FileBackedStore {
    fn save(&self, document: &SettingsDocument) -> Result<(), SettingsError> {
        let serialized = serde_saphyr::to_string(document.value())?;
        nerust_persistence::write_atomic(&self.0.settings_file, serialized.as_bytes())
            .map_err(|error| SettingsError::Io(std::io::Error::other(error)))?;
        Ok(())
    }

    fn load(&self, defaults: &SettingsSnapshot) -> (SettingsSnapshot, SettingsDocument) {
        super::store::load_settings(&self.0.settings_file, defaults)
    }

    fn paths(&self) -> Option<SettingsPaths> {
        Some(self.0.clone())
    }
}

/// In-memory settings store (no persistence across restarts).
#[derive(Debug)]
pub(super) struct EphemeralStore;

impl SettingsStore for EphemeralStore {
    fn save(&self, _document: &SettingsDocument) -> Result<(), SettingsError> {
        Ok(())
    }

    fn load(&self, defaults: &SettingsSnapshot) -> (SettingsSnapshot, SettingsDocument) {
        (
            defaults.clone(),
            SettingsDocument::from_snapshot(defaults).expect("validated defaults must serialize"),
        )
    }

    fn paths(&self) -> Option<SettingsPaths> {
        None
    }
}

/// Test-only store that fails on save.
#[derive(Debug)]
pub struct FailingStore;

impl SettingsStore for FailingStore {
    fn save(&self, _document: &SettingsDocument) -> Result<(), SettingsError> {
        Err(SettingsError::Io(std::io::Error::other(
            "simulated save failure",
        )))
    }

    fn load(&self, _defaults: &SettingsSnapshot) -> (SettingsSnapshot, SettingsDocument) {
        unreachable!("FailingStore is used for save-failure tests only")
    }

    fn paths(&self) -> Option<SettingsPaths> {
        None
    }
}
