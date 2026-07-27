use nerust_gui_settings::snapshot::SettingsSnapshot;

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
        use std::fs;
        if let Some(parent) = self.0.settings_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &self.0.settings_file,
            serde_saphyr::to_string(document.value())?,
        )?;
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

    fn load(&self, _defaults: &SettingsSnapshot) -> (SettingsSnapshot, SettingsDocument) {
        unreachable!("EphemeralStore::load is not called; reload falls back for ephemeral")
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
