use std::{
    fs,
    path::{Path, PathBuf},
};

use directories::ProjectDirs;
use nerust_gui_settings::{
    app_state::{DESKTOP_APP_STATE_SCHEMA_VERSION, DesktopAppState},
    local::{HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION, HostBackendLocalSettings},
    shared::{DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION, DesktopSharedSettings},
};
use serde_yaml::Value;

use super::{LoadedSettingsDocument, SettingsError, SettingsPaths, SettingsStore};

const SHARED_SETTINGS_FILE_NAME: &str = "shared-settings.yaml";
const APP_STATE_FILE_NAME: &str = "app-state.yaml";
const LOCAL_SETTINGS_DIR_NAME: &str = "local-settings";
const CENTRAL_STORAGE_DIR_NAME: &str = "persistence";

const LOCAL_SETTINGS_FILE_NAME: &str = "local-settings.yaml";

impl SettingsPaths {
    pub fn new(config_dir: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Self {
        let config_dir = config_dir.into();
        let data_dir = data_dir.into();
        Self {
            shared_settings_file: config_dir.join(SHARED_SETTINGS_FILE_NAME),
            local_settings_file: config_dir
                .join(LOCAL_SETTINGS_DIR_NAME)
                .join(LOCAL_SETTINGS_FILE_NAME),
            app_state_file: data_dir.join(APP_STATE_FILE_NAME),
            central_storage_root: data_dir.join(CENTRAL_STORAGE_DIR_NAME),
            config_dir,
            data_dir,
        }
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self::new(root.join("config"), root.join("data"))
    }
}

pub(super) fn settings_paths() -> Result<SettingsPaths, SettingsError> {
    let Some(project_dirs) = ProjectDirs::from("com", "github.chalharu", "nerust") else {
        return Err(SettingsError::DirectoriesUnavailable);
    };
    let config_dir = project_dirs.config_dir().to_path_buf();
    let data_dir = project_dirs.data_local_dir().to_path_buf();
    Ok(SettingsPaths::new(config_dir, data_dir))
}

pub(super) fn load_settings_document<T: Clone + serde::de::DeserializeOwned + serde::Serialize>(
    path: &Path,
    defaults: &T,
    schema_version: u32,
) -> Result<LoadedSettingsDocument<T>, SettingsError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let raw: Value = serde_yaml::from_str(&contents)?;
            ensure_supported_schema_version(&raw, schema_version)?;
            match decode_settings_document(defaults, raw.clone()) {
                Ok(settings) => Ok(LoadedSettingsDocument { settings, raw }),
                Err(error) => {
                    log::warn!(
                        "settings file {} is corrupt, resetting to defaults: {error}",
                        path.display(),
                    );
                    Ok(LoadedSettingsDocument {
                        settings: defaults.clone(),
                        raw: serde_yaml::to_value(defaults).unwrap_or_else(|_| empty_mapping()),
                    })
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(LoadedSettingsDocument {
            settings: defaults.clone(),
            raw: empty_mapping(),
        }),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn save_snapshot_store(
    store: &SettingsStore,
    shared: &Value,
    local: &Value,
    app_state: &Value,
) -> Result<(), SettingsError> {
    match store {
        SettingsStore::FileBacked(paths) => {
            fs::create_dir_all(&paths.config_dir)?;
            fs::create_dir_all(&paths.data_dir)?;
            if let Some(parent) = paths.local_settings_file.parent() {
                fs::create_dir_all(parent)?;
            }
            write_yaml(&paths.shared_settings_file, shared)?;
            write_yaml(&paths.local_settings_file, local)?;
            write_yaml(&paths.app_state_file, app_state)?;
            Ok(())
        }
        SettingsStore::Ephemeral => Ok(()),
    }
}

pub(super) fn normalize_loaded_settings<T: Clone + serde::Serialize + serde::de::DeserializeOwned>(
    defaults: &T,
    loaded: T,
) -> Result<T, SettingsError> {
    decode_settings_document(defaults, serde_yaml::to_value(loaded)?)
}

pub(super) fn normalize_shared_settings(
    mut settings: DesktopSharedSettings,
) -> DesktopSharedSettings {
    settings.schema_version = DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION;
    settings
}

pub(super) fn normalize_local_settings(
    mut settings: HostBackendLocalSettings,
) -> HostBackendLocalSettings {
    settings.schema_version = HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION;
    settings
}

pub(super) fn normalize_app_state(mut settings: DesktopAppState) -> DesktopAppState {
    settings.schema_version = DESKTOP_APP_STATE_SCHEMA_VERSION;
    settings
}

pub(super) fn merge_serialized_value<T: serde::Serialize>(
    existing: Value,
    value: &T,
) -> Result<Value, SettingsError> {
    Ok(merge_with_defaults(existing, serde_yaml::to_value(value)?))
}

pub(super) fn merge_with_defaults(mut defaults: Value, overlay: Value) -> Value {
    merge_yaml(&mut defaults, overlay);
    defaults
}

pub(super) fn strip_legacy_local_video_fields(mut document: Value) -> Value {
    let Some(video) = document
        .as_mapping_mut()
        .and_then(|mapping| mapping.get_mut(Value::String("video".into())))
        .and_then(Value::as_mapping_mut)
    else {
        return document;
    };
    for key in ["fullscreen_default", "scaling", "vsync"] {
        video.remove(Value::String(key.into()));
    }
    document
}

pub(super) fn empty_mapping() -> Value {
    Value::Mapping(Default::default())
}

fn write_yaml<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_yaml::to_string(value)?)?;
    Ok(())
}

/// Deserialize `loaded` (overlaid on `defaults`), recovering field-by-field
/// when individual enum variants fail to decode.
///
/// A single unknown enum variant (e.g. a field written by a newer version)
/// no longer resets the entire document — only the field that contains it
/// falls back to its default.
fn decode_settings_document<T: Clone + serde::Serialize + serde::de::DeserializeOwned>(
    defaults: &T,
    loaded: Value,
) -> Result<T, SettingsError> {
    let merged = merge_with_defaults(serde_yaml::to_value(defaults)?, loaded.clone());
    match serde_yaml::from_value(merged.clone()) {
        Ok(v) => Ok(v),
        Err(_) => {
            // Recover field-by-field: walk top-level keys, keep the ones that
            // deserialize, fall back to default for the ones that don't.
            let Some(loaded_map) = loaded.as_mapping() else {
                return Ok(serde_yaml::from_value(merged)?);
            };
            let mut best = defaults.clone();
            for key in loaded_map.keys() {
                let mut candidate = serde_yaml::to_value(&best)?;
                if let Some(cm) = candidate.as_mapping_mut() {
                    cm.insert(key.clone(), loaded_map[key].clone());
                }
                if let Ok(patched) = serde_yaml::from_value::<T>(candidate) {
                    best = patched;
                }
                // On failure, keep best as-is (field stays at default)
            }
            Ok(best)
        }
    }
}

fn ensure_supported_schema_version(value: &Value, expected: u32) -> Result<(), SettingsError> {
    let Some(found) = value
        .as_mapping()
        .and_then(|mapping| mapping.get(Value::String("schema_version".into())))
        .and_then(Value::as_u64)
    else {
        return Ok(());
    };
    let found = found as u32;
    if found > expected {
        return Err(SettingsError::UnsupportedSchemaVersion { found, expected });
    }
    Ok(())
}

fn merge_yaml(into: &mut Value, overlay: Value) {
    match (into, overlay) {
        (Value::Mapping(into_map), Value::Mapping(overlay_map)) => {
            for (key, value) in overlay_map {
                match into_map.get_mut(&key) {
                    Some(existing) => merge_yaml(existing, value),
                    None => {
                        into_map.insert(key, value);
                    }
                }
            }
        }
        (Value::Sequence(into_items), Value::Sequence(overlay_items)) => {
            let existing_items = into_items.clone();
            let mut used = vec![false; existing_items.len()];
            let mut merged = Vec::with_capacity(overlay_items.len());
            for (overlay_index, overlay_item) in overlay_items.into_iter().enumerate() {
                let match_index =
                    sequence_match_index(&existing_items, &used, overlay_index, &overlay_item);
                let mut item = match match_index {
                    Some(index) => {
                        used[index] = true;
                        existing_items[index].clone()
                    }
                    None => Value::Null,
                };
                merge_yaml(&mut item, overlay_item);
                merged.push(item);
            }
            *into_items = merged;
        }
        (target, value) => {
            *target = value;
        }
    }
}

fn sequence_match_index(
    existing_items: &[Value],
    used: &[bool],
    overlay_index: usize,
    overlay_item: &Value,
) -> Option<usize> {
    if let Some(identity) = sequence_item_identity(overlay_item)
        && let Some(index) = existing_items.iter().enumerate().find_map(|(index, item)| {
            (!used[index] && sequence_item_identity(item).as_ref() == Some(&identity))
                .then_some(index)
        })
    {
        return Some(index);
    }
    (overlay_index < existing_items.len() && !used[overlay_index]).then_some(overlay_index)
}

fn sequence_item_identity(value: &Value) -> Option<SequenceItemIdentity> {
    let mapping = value.as_mapping()?;
    if let (Some(attachment), Some(control)) = (
        mapping.get(Value::String("attachment".into())),
        mapping.get(Value::String("control".into())),
    ) {
        return Some(SequenceItemIdentity::KeyboardBinding {
            attachment: attachment.clone(),
            control: control.clone(),
        });
    }
    mapping.get(Value::String("action".into())).map(|action| {
        SequenceItemIdentity::ShortcutBinding {
            action: action.clone(),
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SequenceItemIdentity {
    KeyboardBinding { attachment: Value, control: Value },
    ShortcutBinding { action: Value },
}
