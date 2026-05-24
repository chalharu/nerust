use crc::{CRC_32_ISO_HDLC, Crc};
use directories::ProjectDirs;
use nerust_contract_settings::{
    DESKTOP_SETTINGS_SCHEMA_VERSION, DesktopSettings, HostSettings, StoragePolicy,
};
use nerust_persistence::sidecar::{SidecarPaths, resolve_sidecars};
use serde_yaml::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

const SETTINGS_FILE_NAME: &str = "desktop-settings.yaml";
const APP_DATA_STATES_DIR: &str = "states";
const APP_DATA_MAPPER_SAVES_DIR: &str = "mapper_saves";
const RECENT_ROM_LIMIT: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("desktop settings config directory is unavailable")]
    ConfigDirectoryUnavailable,
    #[error(
        "desktop settings use schema version {found}, but only version {expected} is supported"
    )]
    UnsupportedSchemaVersion { found: u32, expected: u32 },
    #[error("custom persistence roots require both mapper save and state roots")]
    IncompleteCustomRoots,
    #[error("desktop settings serialization failed: {0}")]
    Serialize(#[from] serde_yaml::Error),
    #[error("desktop settings I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("desktop settings lock is poisoned")]
    LockPoisoned,
    #[error("desktop settings file persistence is unavailable in this host context")]
    PersistenceUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopSettingsPaths {
    pub config_dir: PathBuf,
    pub settings_file: PathBuf,
    pub data_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct DesktopSettingsManager {
    inner: Arc<RwLock<DesktopSettingsState>>,
}

#[derive(Debug)]
struct DesktopSettingsState {
    defaults: DesktopSettings,
    current: DesktopSettings,
    store: SettingsStore,
}

#[derive(Debug)]
enum SettingsStore {
    FileBacked(DesktopSettingsPaths),
    Ephemeral,
}

impl DesktopSettingsManager {
    pub fn load(defaults: DesktopSettings) -> Result<Self, SettingsError> {
        let paths = settings_paths()?;
        let current = load_settings_file(&paths.settings_file, &defaults)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(DesktopSettingsState {
                defaults,
                current,
                store: SettingsStore::FileBacked(paths),
            })),
        })
    }

    pub fn ephemeral(defaults: DesktopSettings) -> Self {
        Self {
            inner: Arc::new(RwLock::new(DesktopSettingsState {
                current: defaults.clone(),
                defaults,
                store: SettingsStore::Ephemeral,
            })),
        }
    }

    pub fn current(&self) -> Result<DesktopSettings, SettingsError> {
        Ok(self
            .inner
            .read()
            .map_err(|_| SettingsError::LockPoisoned)?
            .current
            .clone())
    }

    pub fn defaults(&self) -> Result<DesktopSettings, SettingsError> {
        Ok(self
            .inner
            .read()
            .map_err(|_| SettingsError::LockPoisoned)?
            .defaults
            .clone())
    }

    pub fn paths(&self) -> Result<Option<DesktopSettingsPaths>, SettingsError> {
        let guard = self.inner.read().map_err(|_| SettingsError::LockPoisoned)?;
        Ok(match &guard.store {
            SettingsStore::FileBacked(paths) => Some(paths.clone()),
            SettingsStore::Ephemeral => None,
        })
    }

    pub fn save(&self, settings: DesktopSettings) -> Result<(), SettingsError> {
        let mut guard = self
            .inner
            .write()
            .map_err(|_| SettingsError::LockPoisoned)?;
        let normalized = normalized_settings(&guard.defaults, settings)?;
        save_store(&guard.store, &normalized)?;
        guard.current = normalized;
        Ok(())
    }

    pub fn reload(&self) -> Result<DesktopSettings, SettingsError> {
        let mut guard = self
            .inner
            .write()
            .map_err(|_| SettingsError::LockPoisoned)?;
        let loaded = match &guard.store {
            SettingsStore::FileBacked(paths) => {
                load_settings_file(&paths.settings_file, &guard.defaults)?
            }
            SettingsStore::Ephemeral => guard.defaults.clone(),
        };
        guard.current = loaded.clone();
        Ok(loaded)
    }

    pub fn record_opened_rom(&self, rom_path: &Path) -> Result<(), SettingsError> {
        let mut settings = self.current()?;
        settings.general.last_open_directory = rom_path.parent().map(Path::to_path_buf);
        settings.general.recent_roms.retain(|path| path != rom_path);
        settings
            .general
            .recent_roms
            .insert(0, rom_path.to_path_buf());
        settings.general.recent_roms.truncate(RECENT_ROM_LIMIT);
        self.save(settings)
    }

    pub fn remember_window_size(&self, width: u32, height: u32) -> Result<(), SettingsError> {
        let mut settings = self.current()?;
        if !settings.host.remember_window_bounds {
            return Ok(());
        }
        settings.host.window_width = Some(width);
        settings.host.window_height = Some(height);
        self.save(settings)
    }

    pub fn effective_window_size(
        &self,
        fallback_width: u32,
        fallback_height: u32,
    ) -> Result<(u32, u32), SettingsError> {
        let settings = self.current()?;
        Ok(
            match (
                settings.host.remember_window_bounds,
                settings.host.window_width,
                settings.host.window_height,
            ) {
                (true, Some(width), Some(height)) => (width, height),
                _ => (fallback_width, fallback_height),
            },
        )
    }

    pub fn resolve_persistence_paths(
        &self,
        rom_path: &Path,
    ) -> Result<SidecarPaths, SettingsError> {
        let settings = self.current()?;
        resolve_persistence_paths(&settings, self.paths()?.as_ref(), rom_path)
    }
}

pub fn resolve_persistence_paths(
    settings: &DesktopSettings,
    paths: Option<&DesktopSettingsPaths>,
    rom_path: &Path,
) -> Result<SidecarPaths, SettingsError> {
    match settings.persistence.storage_policy {
        StoragePolicy::RomSidecar => Ok(resolve_sidecars(rom_path)),
        StoragePolicy::AppData => {
            let Some(paths) = paths else {
                return Err(SettingsError::PersistenceUnavailable);
            };
            Ok(app_data_sidecars(paths, rom_path))
        }
        StoragePolicy::CustomRoots => {
            let state_root = settings
                .persistence
                .state_root
                .as_ref()
                .ok_or(SettingsError::IncompleteCustomRoots)?;
            let mapper_save_root = settings
                .persistence
                .mapper_save_root
                .as_ref()
                .ok_or(SettingsError::IncompleteCustomRoots)?;
            Ok(custom_root_sidecars(state_root, mapper_save_root, rom_path))
        }
    }
}

pub fn host_settings(settings: &DesktopSettings) -> &HostSettings {
    &settings.host
}

fn settings_paths() -> Result<DesktopSettingsPaths, SettingsError> {
    let Some(project_dirs) = ProjectDirs::from("com", "github.chalharu", "nerust") else {
        return Err(SettingsError::ConfigDirectoryUnavailable);
    };
    Ok(DesktopSettingsPaths {
        config_dir: project_dirs.config_dir().to_path_buf(),
        settings_file: project_dirs.config_dir().join(SETTINGS_FILE_NAME),
        data_dir: project_dirs.data_local_dir().to_path_buf(),
    })
}

fn load_settings_file(
    path: &Path,
    defaults: &DesktopSettings,
) -> Result<DesktopSettings, SettingsError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let loaded_value: Value = serde_yaml::from_str(&contents)?;
            ensure_supported_schema_version(&loaded_value)?;
            normalized_settings_value(defaults, loaded_value)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(defaults.clone()),
        Err(error) => Err(error.into()),
    }
}

fn save_store(store: &SettingsStore, settings: &DesktopSettings) -> Result<(), SettingsError> {
    match store {
        SettingsStore::FileBacked(paths) => {
            fs::create_dir_all(&paths.config_dir)?;
            let encoded = serde_yaml::to_string(settings)?;
            fs::write(&paths.settings_file, encoded)?;
            Ok(())
        }
        SettingsStore::Ephemeral => Ok(()),
    }
}

fn normalized_settings(
    defaults: &DesktopSettings,
    settings: DesktopSettings,
) -> Result<DesktopSettings, SettingsError> {
    normalized_settings_value(defaults, serde_yaml::to_value(settings)?)
}

fn normalized_settings_value(
    defaults: &DesktopSettings,
    loaded_value: Value,
) -> Result<DesktopSettings, SettingsError> {
    let mut merged = serde_yaml::to_value(defaults)?;
    merge_yaml(&mut merged, loaded_value);
    let mut settings: DesktopSettings = serde_yaml::from_value(merged)?;
    settings.schema_version = DESKTOP_SETTINGS_SCHEMA_VERSION;
    settings.systems = settings.systems.into_iter().collect::<BTreeMap<_, _>>();
    Ok(settings)
}

fn ensure_supported_schema_version(value: &Value) -> Result<(), SettingsError> {
    let Some(found) = value
        .as_mapping()
        .and_then(|mapping| mapping.get(Value::String("schema_version".into())))
        .and_then(Value::as_u64)
    else {
        return Ok(());
    };
    let found = found as u32;
    if found > DESKTOP_SETTINGS_SCHEMA_VERSION {
        return Err(SettingsError::UnsupportedSchemaVersion {
            found,
            expected: DESKTOP_SETTINGS_SCHEMA_VERSION,
        });
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
        (target, value) => {
            *target = value;
        }
    }
}

fn app_data_sidecars(paths: &DesktopSettingsPaths, rom_path: &Path) -> SidecarPaths {
    let (stem, key) = rom_storage_key(rom_path);
    SidecarPaths {
        mapper_save_path: paths
            .data_dir
            .join(APP_DATA_MAPPER_SAVES_DIR)
            .join(format!("{stem}-{key}.sav")),
        states_dir: paths
            .data_dir
            .join(APP_DATA_STATES_DIR)
            .join(format!("{stem}-{key}.states")),
    }
}

fn custom_root_sidecars(
    state_root: &Path,
    mapper_save_root: &Path,
    rom_path: &Path,
) -> SidecarPaths {
    let (stem, key) = rom_storage_key(rom_path);
    SidecarPaths {
        mapper_save_path: mapper_save_root.join(format!("{stem}-{key}.sav")),
        states_dir: state_root.join(format!("{stem}-{key}.states")),
    }
}

fn rom_storage_key(rom_path: &Path) -> (String, String) {
    let name = rom_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("game");
    let normalized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let crc = Crc::<u32>::new(&CRC_32_ISO_HDLC);
    let checksum = crc.checksum(rom_path.to_string_lossy().as_bytes());
    (normalized, format!("{checksum:08x}"))
}

#[cfg(test)]
mod tests {
    use super::{
        DesktopSettingsManager, SettingsError, app_data_sidecars, merge_yaml,
        normalized_settings_value, resolve_persistence_paths, rom_storage_key,
    };
    use nerust_contract_settings::{
        DesktopSettings, PersistenceSettings, StoragePolicy, SystemSettings,
    };
    use nerust_input_schema::SystemId;
    use serde_yaml::{Mapping, Value};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn test_defaults() -> DesktopSettings {
        DesktopSettings {
            persistence: PersistenceSettings::default(),
            systems: BTreeMap::from([(SystemId::Nes, SystemSettings::Nes(Default::default()))]),
            ..Default::default()
        }
    }

    #[test]
    fn merge_yaml_preserves_missing_default_keys() {
        let mut defaults = Value::Mapping(Mapping::from_iter([(
            Value::String("outer".into()),
            Value::Mapping(Mapping::from_iter([
                (Value::String("keep".into()), Value::Bool(true)),
                (
                    Value::String("replace".into()),
                    Value::String("default".into()),
                ),
            ])),
        )]));

        merge_yaml(
            &mut defaults,
            Value::Mapping(Mapping::from_iter([(
                Value::String("outer".into()),
                Value::Mapping(Mapping::from_iter([(
                    Value::String("replace".into()),
                    Value::String("loaded".into()),
                )])),
            )])),
        );

        let outer = defaults
            .as_mapping()
            .unwrap()
            .get(Value::String("outer".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        assert_eq!(
            outer.get(Value::String("keep".into())).unwrap(),
            &Value::Bool(true)
        );
        assert_eq!(
            outer.get(Value::String("replace".into())).unwrap(),
            &Value::String("loaded".into())
        );
    }

    #[test]
    fn normalized_settings_fill_missing_defaults_from_seed_settings() {
        let defaults = test_defaults();
        let loaded = Value::Mapping(Mapping::from_iter([(
            Value::String("general".into()),
            Value::Mapping(Mapping::new()),
        )]));

        let normalized = normalized_settings_value(&defaults, loaded).unwrap();

        assert!(normalized.systems.contains_key(&SystemId::Nes));
    }

    #[test]
    fn resolve_persistence_paths_uses_app_data_keyed_by_full_rom_path() {
        let mut settings = test_defaults();
        settings.persistence.storage_policy = StoragePolicy::AppData;
        let paths = super::DesktopSettingsPaths {
            config_dir: PathBuf::from("/config"),
            settings_file: PathBuf::from("/config/settings.yaml"),
            data_dir: PathBuf::from("/data"),
        };

        let resolved = resolve_persistence_paths(
            &settings,
            Some(&paths),
            PathBuf::from("/roms/Super Mario Bros.nes").as_path(),
        )
        .unwrap();

        assert!(resolved.mapper_save_path.starts_with("/data/mapper_saves"));
        assert!(resolved.states_dir.starts_with("/data/states"));
        assert_ne!(
            app_data_sidecars(&paths, PathBuf::from("/roms/a.nes").as_path()),
            app_data_sidecars(&paths, PathBuf::from("/elsewhere/a.nes").as_path())
        );
    }

    #[test]
    fn custom_roots_require_both_paths() {
        let mut settings = test_defaults();
        settings.persistence.storage_policy = StoragePolicy::CustomRoots;
        settings.persistence.state_root = Some(PathBuf::from("/states"));

        let error =
            resolve_persistence_paths(&settings, None, PathBuf::from("/roms/game.nes").as_path())
                .unwrap_err();

        assert!(matches!(error, SettingsError::IncompleteCustomRoots));
    }

    #[test]
    fn ephemeral_manager_round_trips_current_state() {
        let manager = DesktopSettingsManager::ephemeral(test_defaults());
        let mut current = manager.current().unwrap();
        current.general.restore_last_session = true;

        manager.save(current.clone()).unwrap();

        assert_eq!(manager.current().unwrap(), current);
    }

    #[test]
    fn rom_storage_key_uses_name_and_checksum() {
        let (name, checksum) = rom_storage_key(PathBuf::from("/roms/game.nes").as_path());

        assert_eq!(name, "game.nes");
        assert_eq!(checksum.len(), 8);
    }
}
