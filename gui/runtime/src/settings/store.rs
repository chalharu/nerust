use std::{
    fs,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use serde_value::Value;

use super::{SettingsDocument, SettingsError, SettingsPaths, SettingsSnapshot, SettingsStore};

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

pub(super) fn load_settings(
    path: &Path,
    defaults: &SettingsSnapshot,
) -> (SettingsSnapshot, SettingsDocument) {
    match fs::read_to_string(path) {
        Ok(contents) => match serde_saphyr::from_str::<Value>(&contents) {
            Ok(raw) => {
                let document = SettingsDocument::from_value_with_known_systems(raw, defaults);
                match document.clone().into_snapshot() {
                    Ok(snapshot) => (with_required_system_defaults(snapshot, defaults), document),
                    Err(err) => {
                        log::warn!(
                            "settings file {} has corrupt or unknown fields, recovering: {err}",
                            path.display(),
                        );
                        (
                            with_required_system_defaults(
                                recover_snapshot(defaults, document.value()),
                                defaults,
                            ),
                            document,
                        )
                    }
                }
            }
            Err(err) => {
                log::warn!(
                    "settings file {} is not valid YAML, using defaults: {err}",
                    path.display(),
                );
                settings_from_defaults(defaults)
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            settings_from_defaults(defaults)
        }
        Err(error) => {
            log::warn!(
                "settings file {} unreadable, using defaults: {error}",
                path.display(),
            );
            settings_from_defaults(defaults)
        }
    }
}

fn with_required_system_defaults(
    mut snapshot: SettingsSnapshot,
    defaults: &SettingsSnapshot,
) -> SettingsSnapshot {
    for (system_id, settings) in &defaults.shared.systems {
        snapshot
            .shared
            .systems
            .entry(system_id.clone())
            .or_insert_with(|| settings.clone());
    }
    for (system_id, settings) in &defaults.shared.input.systems {
        snapshot
            .shared
            .input
            .systems
            .entry(system_id.clone())
            .or_insert_with(|| settings.clone());
    }
    snapshot
}

fn settings_from_defaults(defaults: &SettingsSnapshot) -> (SettingsSnapshot, SettingsDocument) {
    (
        defaults.clone(),
        SettingsDocument::from_snapshot(defaults)
            .expect("serializing validated default settings should succeed"),
    )
}

/// Recover a `SettingsSnapshot` from raw YAML by trying each top-level field
/// independently.  A corrupt field (e.g. an enum variant from a future version)
/// is reset to its default while the remaining fields are preserved.
fn recover_snapshot(defaults: &SettingsSnapshot, raw: &Value) -> SettingsSnapshot {
    let Value::Map(map) = raw else {
        return defaults.clone();
    };
    let mut result = defaults.clone();
    for key_str in ["shared", "local", "app_state"] {
        let key = Value::String(key_str.to_string());
        let Some(field_val) = map.get(&key) else {
            continue;
        };
        match key_str {
            "shared" => {
                let filtered = filter_unknown_systems(field_val, &defaults.shared);
                if let Ok(v) = filtered.deserialize_into() {
                    result.shared = v;
                }
            }
            "local" => {
                if let Ok(v) = field_val.clone().deserialize_into() {
                    result.local = v;
                }
            }
            "app_state" => {
                if let Ok(v) = field_val.clone().deserialize_into() {
                    result.app_state = v;
                }
            }
            _ => {}
        }
    }
    result
}

fn filter_unknown_systems(
    shared: &Value,
    defaults: &nerust_gui_settings::shared::DesktopSharedSettings,
) -> Value {
    let mut filtered = shared.clone();
    let Ok(defaults) = serde_value::to_value(defaults) else {
        return filtered;
    };
    retain_known_map_entries(&mut filtered, &defaults, &["systems"]);
    retain_known_map_entries(&mut filtered, &defaults, &["input", "systems"]);
    filtered
}

fn retain_known_map_entries(value: &mut Value, defaults: &Value, path: &[&str]) {
    let Some(Value::Map(default_map)) = value_at_path(defaults, path) else {
        return;
    };
    let Some(Value::Map(map)) = value_at_path_mut(value, path) else {
        return;
    };
    map.retain(|key, _| default_map.contains_key(key));
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, segment| {
        let Value::Map(map) = current else {
            return None;
        };
        map.get(&Value::String((*segment).to_string()))
    })
}

fn value_at_path_mut<'a>(value: &'a mut Value, path: &[&str]) -> Option<&'a mut Value> {
    let Some((segment, remaining)) = path.split_first() else {
        return Some(value);
    };
    let Value::Map(map) = value else {
        return None;
    };
    value_at_path_mut(
        map.get_mut(&Value::String((*segment).to_string()))?,
        remaining,
    )
}

pub(super) fn save_snapshot_store(
    store: &SettingsStore,
    document: &SettingsDocument,
) -> Result<(), SettingsError> {
    match store {
        SettingsStore::FileBacked(paths) => {
            if let Some(parent) = paths.settings_file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(
                &paths.settings_file,
                serde_saphyr::to_string(document.value())?,
            )?;
            Ok(())
        }
        SettingsStore::Ephemeral => Ok(()),
    }
}
