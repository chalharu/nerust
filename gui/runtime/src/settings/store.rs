use std::{
    fs,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;

use super::{SettingsError, SettingsPaths, SettingsSnapshot, SettingsStore};

const SETTINGS_FILE_NAME: &str = "settings.yaml";
const CENTRAL_STORAGE_DIR_NAME: &str = "persistence";

impl SettingsPaths {
    pub fn new(config_dir: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            settings_file: config_dir.into().join(SETTINGS_FILE_NAME),
            central_storage_root: data_dir.into().join(CENTRAL_STORAGE_DIR_NAME),
        }
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self::new(root.join("config"), root.join("data"))
    }
}

pub(super) fn settings_paths() -> Result<SettingsPaths, SettingsError> {
    let Some(project_dirs) = ProjectDirs::from("io", "github.chalharu", "nerust") else {
        return Err(SettingsError::DirectoriesUnavailable);
    };
    Ok(SettingsPaths::new(
        project_dirs.config_dir(),
        project_dirs.data_local_dir(),
    ))
}

pub(super) fn load_snapshot(path: &Path, defaults: &SettingsSnapshot) -> SettingsSnapshot {
    match fs::read_to_string(path) {
        Ok(contents) => match serde_saphyr::from_str::<SettingsSnapshot>(&contents) {
            Ok(snapshot) => snapshot,
            Err(err) => {
                log::warn!(
                    "settings file {} is corrupt, using defaults: {err}",
                    path.display(),
                );
                defaults.clone()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => defaults.clone(),
        Err(error) => {
            log::warn!(
                "settings file {} unreadable, using defaults: {error}",
                path.display(),
            );
            defaults.clone()
        }
    }
}

pub(super) fn save_snapshot_store(
    store: &SettingsStore,
    snapshot: &SettingsSnapshot,
) -> Result<(), SettingsError> {
    match store {
        SettingsStore::FileBacked(paths) => {
            if let Some(parent) = paths.settings_file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&paths.settings_file, serde_saphyr::to_string(snapshot)?)?;
            Ok(())
        }
        SettingsStore::Ephemeral => Ok(()),
    }
}
