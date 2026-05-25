use crc::{CRC_32_ISO_HDLC, Crc};
use directories::ProjectDirs;
use nerust_contract_rom::RomIdentity;
use nerust_contract_settings::app_state::{DESKTOP_APP_STATE_SCHEMA_VERSION, DesktopAppState};
use nerust_contract_settings::local::{
    HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION, HostBackendLocalSettings,
};
use nerust_contract_settings::nes::NesVideoFilter;
use nerust_contract_settings::shared::{
    DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION, DesktopSharedSettings, StoragePolicy, SystemSettings,
};
use nerust_input_schema::SystemId;
use nerust_persistence::sidecar::{SidecarPaths, resolve_sidecars};
use serde_yaml::Value;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

const SHARED_SETTINGS_FILE_NAME: &str = "shared-settings.yaml";
const APP_STATE_FILE_NAME: &str = "app-state.yaml";
const LOCAL_SETTINGS_DIR_NAME: &str = "local-settings";
const CENTRAL_STORAGE_DIR_NAME: &str = "persistence";
const MAPPER_SAVE_FILE_NAME: &str = "mapper.sav";
const STATES_DIR_NAME: &str = "states";

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("desktop settings directories are unavailable")]
    DirectoriesUnavailable,
    #[error("settings schema version {found} is newer than supported version {expected}")]
    UnsupportedSchemaVersion { found: u32, expected: u32 },
    #[error("custom storage directory is required when policy=custom_directory")]
    MissingCustomStorageDirectory,
    #[error("settings persistence is unavailable in this host context")]
    PersistenceUnavailable,
    #[error("settings serialization failed: {0}")]
    Serialize(#[from] serde_yaml::Error),
    #[error("settings I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings lock is poisoned")]
    LockPoisoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBackendIdentity {
    host: String,
    backend: String,
}

impl HostBackendIdentity {
    pub fn new(host: impl Into<String>, backend: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            backend: backend.into(),
        }
    }

    pub fn host(&self) -> &str {
        self.host.as_str()
    }

    pub fn backend(&self) -> &str {
        self.backend.as_str()
    }

    pub fn gtk_opengl() -> Self {
        Self::new("gtk", "opengl")
    }

    pub fn glutin_opengl() -> Self {
        Self::new("glutin", "opengl")
    }

    pub fn tao_wgpu() -> Self {
        Self::new("tao", "wgpu")
    }

    fn file_stem(&self) -> String {
        format!(
            "{}+{}",
            sanitize_path_component(&self.host),
            sanitize_path_component(&self.backend)
        )
    }
}

impl fmt::Display for HostBackendIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}+{}", self.host, self.backend)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub shared_settings_file: PathBuf,
    pub local_settings_file: PathBuf,
    pub app_state_file: PathBuf,
    pub central_storage_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsSnapshot {
    pub shared: DesktopSharedSettings,
    pub local: HostBackendLocalSettings,
    pub app_state: DesktopAppState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsApplyPlan {
    pub language_changed: bool,
    pub bindings_changed: bool,
    pub persistence_changed: bool,
    pub session_rebuild_required: bool,
    pub scaling_changed: bool,
    pub vsync_changed: bool,
    pub fullscreen_default_changed: bool,
}

#[derive(Clone, Debug)]
pub struct SettingsManager {
    inner: Arc<RwLock<SettingsState>>,
}

#[derive(Debug)]
struct SettingsState {
    shared_defaults: DesktopSharedSettings,
    local_defaults: HostBackendLocalSettings,
    app_state_defaults: DesktopAppState,
    current: SettingsSnapshot,
    shared_document: Value,
    local_document: Value,
    app_state_document: Value,
    store: SettingsStore,
}

#[derive(Debug)]
enum SettingsStore {
    FileBacked(SettingsPaths),
    Ephemeral,
}

#[derive(Debug)]
struct LoadedSettingsDocument<T> {
    settings: T,
    raw: Value,
}

impl SettingsManager {
    pub fn load(
        identity: HostBackendIdentity,
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Result<Self, SettingsError> {
        let paths = settings_paths(&identity)?;
        let shared_document = load_settings_document(
            &paths.shared_settings_file,
            &shared_defaults,
            DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION,
        )?;
        let local_document = load_settings_document(
            &paths.local_settings_file,
            &local_defaults,
            HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION,
        )?;
        let app_state_document = load_settings_document(
            &paths.app_state_file,
            &app_state_defaults,
            DESKTOP_APP_STATE_SCHEMA_VERSION,
        )?;
        let current = SettingsSnapshot {
            shared: shared_document.settings,
            local: local_document.settings,
            app_state: app_state_document.settings,
        };
        Ok(Self {
            inner: Arc::new(RwLock::new(SettingsState {
                shared_defaults,
                local_defaults,
                app_state_defaults,
                current,
                shared_document: shared_document.raw,
                local_document: local_document.raw,
                app_state_document: app_state_document.raw,
                store: SettingsStore::FileBacked(paths),
            })),
        })
    }

    pub fn load_or_ephemeral(
        identity: HostBackendIdentity,
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Self {
        match Self::load(
            identity,
            shared_defaults.clone(),
            local_defaults.clone(),
            app_state_defaults.clone(),
        ) {
            Ok(manager) => manager,
            Err(error) => {
                log::warn!("settings persistence unavailable; using ephemeral settings: {error}");
                Self::ephemeral(shared_defaults, local_defaults, app_state_defaults)
            }
        }
    }

    pub fn ephemeral(
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(SettingsState {
                current: SettingsSnapshot {
                    shared: shared_defaults.clone(),
                    local: local_defaults.clone(),
                    app_state: app_state_defaults.clone(),
                },
                shared_defaults,
                local_defaults,
                app_state_defaults,
                shared_document: empty_mapping(),
                local_document: empty_mapping(),
                app_state_document: empty_mapping(),
                store: SettingsStore::Ephemeral,
            })),
        }
    }

    pub fn snapshot(&self) -> Result<SettingsSnapshot, SettingsError> {
        Ok(self
            .inner
            .read()
            .map_err(|_| SettingsError::LockPoisoned)?
            .current
            .clone())
    }

    pub fn shared(&self) -> Result<DesktopSharedSettings, SettingsError> {
        Ok(self.snapshot()?.shared)
    }

    pub fn local(&self) -> Result<HostBackendLocalSettings, SettingsError> {
        Ok(self.snapshot()?.local)
    }

    pub fn app_state(&self) -> Result<DesktopAppState, SettingsError> {
        Ok(self.snapshot()?.app_state)
    }

    pub fn paths(&self) -> Result<Option<SettingsPaths>, SettingsError> {
        let guard = self.inner.read().map_err(|_| SettingsError::LockPoisoned)?;
        Ok(match &guard.store {
            SettingsStore::FileBacked(paths) => Some(paths.clone()),
            SettingsStore::Ephemeral => None,
        })
    }

    pub fn save_shared(&self, shared: DesktopSharedSettings) -> Result<(), SettingsError> {
        let mut snapshot = self.snapshot()?;
        snapshot.shared = normalize_shared_settings(shared);
        self.save_snapshot(snapshot)
    }

    pub fn save_local(&self, local: HostBackendLocalSettings) -> Result<(), SettingsError> {
        let mut snapshot = self.snapshot()?;
        snapshot.local = normalize_local_settings(local);
        self.save_snapshot(snapshot)
    }

    pub fn save_app_state(&self, app_state: DesktopAppState) -> Result<(), SettingsError> {
        let mut snapshot = self.snapshot()?;
        snapshot.app_state = normalize_app_state(app_state);
        self.save_snapshot(snapshot)
    }

    pub fn save_snapshot(&self, snapshot: SettingsSnapshot) -> Result<(), SettingsError> {
        validate_shared_settings(&snapshot.shared)?;
        validate_local_settings(&snapshot.local)?;

        let mut guard = self
            .inner
            .write()
            .map_err(|_| SettingsError::LockPoisoned)?;
        let normalized = SettingsSnapshot {
            shared: normalize_loaded_settings(&guard.shared_defaults, snapshot.shared)?,
            local: normalize_loaded_settings(&guard.local_defaults, snapshot.local)?,
            app_state: normalize_loaded_settings(&guard.app_state_defaults, snapshot.app_state)?,
        };
        let shared_document =
            merge_serialized_value(guard.shared_document.clone(), &normalized.shared)?;
        let local_document =
            merge_serialized_value(guard.local_document.clone(), &normalized.local)?;
        let app_state_document =
            merge_serialized_value(guard.app_state_document.clone(), &normalized.app_state)?;
        save_snapshot_store(
            &guard.store,
            &shared_document,
            &local_document,
            &app_state_document,
        )?;
        guard.current = normalized;
        guard.shared_document = shared_document;
        guard.local_document = local_document;
        guard.app_state_document = app_state_document;
        Ok(())
    }

    pub fn reload(&self) -> Result<SettingsSnapshot, SettingsError> {
        let mut guard = self
            .inner
            .write()
            .map_err(|_| SettingsError::LockPoisoned)?;
        let (loaded, shared_document, local_document, app_state_document) = match &guard.store {
            SettingsStore::FileBacked(paths) => {
                let shared_document = load_settings_document(
                    &paths.shared_settings_file,
                    &guard.shared_defaults,
                    DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION,
                )?;
                let local_document = load_settings_document(
                    &paths.local_settings_file,
                    &guard.local_defaults,
                    HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION,
                )?;
                let app_state_document = load_settings_document(
                    &paths.app_state_file,
                    &guard.app_state_defaults,
                    DESKTOP_APP_STATE_SCHEMA_VERSION,
                )?;
                (
                    SettingsSnapshot {
                        shared: shared_document.settings,
                        local: local_document.settings,
                        app_state: app_state_document.settings,
                    },
                    shared_document.raw,
                    local_document.raw,
                    app_state_document.raw,
                )
            }
            SettingsStore::Ephemeral => (
                SettingsSnapshot {
                    shared: guard.current.shared.clone(),
                    local: guard.current.local.clone(),
                    app_state: guard.current.app_state.clone(),
                },
                guard.shared_document.clone(),
                guard.local_document.clone(),
                guard.app_state_document.clone(),
            ),
        };
        guard.current = loaded.clone();
        guard.shared_document = shared_document;
        guard.local_document = local_document;
        guard.app_state_document = app_state_document;
        Ok(loaded)
    }

    pub fn update_last_successful_rom_directory(
        &self,
        rom_path: &Path,
    ) -> Result<(), SettingsError> {
        let mut snapshot = self.snapshot()?;
        snapshot.app_state.last_successful_rom_directory = rom_path.parent().map(Path::to_path_buf);
        self.save_snapshot(snapshot)
    }

    pub fn resolve_persistence_paths(
        &self,
        system: SystemId,
        rom_path: Option<&Path>,
        rom_identity: RomIdentity,
    ) -> Result<SidecarPaths, SettingsError> {
        let snapshot = self.snapshot()?;
        resolve_persistence_paths(
            &snapshot.shared,
            self.paths()?.as_ref(),
            system,
            rom_path,
            rom_identity,
        )
    }

    pub fn resolve_persistence_paths_with_import(
        &self,
        system: SystemId,
        rom_path: Option<&Path>,
        rom_identity: RomIdentity,
    ) -> Result<SidecarPaths, SettingsError> {
        let snapshot = self.snapshot()?;
        let paths = self.paths()?;
        resolve_persistence_paths_with_import(
            &snapshot.shared,
            paths.as_ref(),
            system,
            rom_path,
            rom_identity,
        )
    }
}

pub fn derive_apply_plan(before: &SettingsSnapshot, after: &SettingsSnapshot) -> SettingsApplyPlan {
    let audio_changed = before.local.audio != after.local.audio;
    let visual_changed = live_system_settings_changed(&before.shared, &after.shared);
    SettingsApplyPlan {
        language_changed: before.shared.general != after.shared.general,
        bindings_changed: before.shared.input != after.shared.input,
        persistence_changed: before.shared.persistence != after.shared.persistence,
        session_rebuild_required: audio_changed || visual_changed,
        scaling_changed: before.local.video.scaling != after.local.video.scaling,
        vsync_changed: before.local.video.vsync != after.local.video.vsync,
        fullscreen_default_changed: before.local.video.fullscreen_default
            != after.local.video.fullscreen_default,
    }
}

pub fn validate_shared_settings(settings: &DesktopSharedSettings) -> Result<(), SettingsError> {
    if matches!(
        settings.persistence.storage_policy,
        StoragePolicy::CustomDirectory
    ) {
        let Some(path) = settings.persistence.storage_directory.as_ref() else {
            return Err(SettingsError::MissingCustomStorageDirectory);
        };
        validate_directory_path(path)?;
    }
    Ok(())
}

pub fn validate_local_settings(settings: &HostBackendLocalSettings) -> Result<(), SettingsError> {
    let volume = settings.local_audio_volume_percent();
    let sample_rate = settings.audio.sample_rate;
    let latency = settings.audio.latency_ms;
    if !(0..=100).contains(&volume) {
        return Err(SettingsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "master volume must be between 0 and 100",
        )));
    }
    if !matches!(sample_rate, 22_050 | 44_100 | 48_000) {
        return Err(SettingsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sample rate must be 22050, 44100, or 48000",
        )));
    }
    if !(10..=200).contains(&latency) {
        return Err(SettingsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "audio latency must be between 10 and 200 ms",
        )));
    }
    Ok(())
}

pub fn resolve_persistence_paths(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: SystemId,
    rom_path: Option<&Path>,
    rom_identity: RomIdentity,
) -> Result<SidecarPaths, SettingsError> {
    resolve_current_persistence_paths(shared, paths, system, rom_path, rom_identity)
}

pub fn resolve_persistence_paths_with_import(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: SystemId,
    rom_path: Option<&Path>,
    rom_identity: RomIdentity,
) -> Result<SidecarPaths, SettingsError> {
    let resolved =
        resolve_current_persistence_paths(shared, paths, system, rom_path, rom_identity)?;
    maybe_auto_import_storage(shared, paths, system, rom_path, rom_identity, &resolved)?;
    Ok(resolved)
}

pub fn system_storage_key(system: SystemId, rom_identity: RomIdentity) -> String {
    let canonical = canonical_rom_identity(system, rom_identity);
    let checksum = Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(canonical.as_bytes());
    format!(
        "m{:04x}-s{:02x}-p{:016x}-c{:016x}-t{:016x}-{:08x}",
        rom_identity.mapper_type,
        rom_identity.sub_mapper_type,
        rom_identity.prg_rom_crc64,
        rom_identity.chr_rom_crc64,
        rom_identity.trainer_crc64,
        checksum
    )
}

fn resolve_current_persistence_paths(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: SystemId,
    rom_path: Option<&Path>,
    rom_identity: RomIdentity,
) -> Result<SidecarPaths, SettingsError> {
    match shared.persistence.storage_policy {
        StoragePolicy::Sidecar => {
            let rom_path = rom_path.ok_or(SettingsError::PersistenceUnavailable)?;
            Ok(resolve_sidecars(rom_path))
        }
        StoragePolicy::AppSharedData => {
            let Some(paths) = paths else {
                return Err(SettingsError::PersistenceUnavailable);
            };
            Ok(resolve_central_storage_paths(
                &paths.central_storage_root,
                system,
                rom_identity,
            ))
        }
        StoragePolicy::CustomDirectory => {
            let root = shared
                .persistence
                .storage_directory
                .as_deref()
                .ok_or(SettingsError::MissingCustomStorageDirectory)?;
            Ok(resolve_central_storage_paths(root, system, rom_identity))
        }
    }
}

fn validate_directory_path(path: &Path) -> Result<(), SettingsError> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        match fs::metadata(candidate) {
            Ok(metadata) => {
                if metadata.is_dir() {
                    return Ok(());
                }
                return Err(SettingsError::Io(std::io::Error::other(
                    "custom storage path is not a directory",
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                current = candidate.parent();
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn maybe_auto_import_storage(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: SystemId,
    rom_path: Option<&Path>,
    rom_identity: RomIdentity,
    destination: &SidecarPaths,
) -> Result<(), SettingsError> {
    let Some(rom_path) = rom_path else {
        return Ok(());
    };
    if !storage_is_empty(destination)? {
        return Ok(());
    }

    match shared.persistence.storage_policy {
        StoragePolicy::Sidecar => {
            if let Some(paths) = paths {
                let app_shared = resolve_central_storage_paths(
                    &paths.central_storage_root,
                    system,
                    rom_identity,
                );
                if !storage_is_empty(&app_shared)? {
                    copy_storage_contents(&app_shared, destination)?;
                    return Ok(());
                }
            }
            if let Some(root) = shared.persistence.storage_directory.as_deref() {
                let custom = resolve_central_storage_paths(root, system, rom_identity);
                if !storage_is_empty(&custom)? {
                    copy_storage_contents(&custom, destination)?;
                }
            }
        }
        StoragePolicy::AppSharedData | StoragePolicy::CustomDirectory => {
            let sidecar = resolve_sidecars(rom_path);
            if !storage_is_empty(&sidecar)? {
                copy_storage_contents(&sidecar, destination)?;
                return Ok(());
            }
            match shared.persistence.storage_policy {
                StoragePolicy::AppSharedData => {
                    if let Some(root) = shared.persistence.storage_directory.as_deref() {
                        let custom = resolve_central_storage_paths(root, system, rom_identity);
                        if !storage_is_empty(&custom)? {
                            copy_storage_contents(&custom, destination)?;
                        }
                    }
                }
                StoragePolicy::CustomDirectory => {
                    if let Some(paths) = paths {
                        let app_shared = resolve_central_storage_paths(
                            &paths.central_storage_root,
                            system,
                            rom_identity,
                        );
                        if !storage_is_empty(&app_shared)? {
                            copy_storage_contents(&app_shared, destination)?;
                        }
                    }
                }
                StoragePolicy::Sidecar => {}
            }
        }
    }
    Ok(())
}

pub fn resolve_central_storage_paths(
    root: &Path,
    system: SystemId,
    rom_identity: RomIdentity,
) -> SidecarPaths {
    let base = root
        .join(system_id_slug(system))
        .join(system_storage_key(system, rom_identity));
    SidecarPaths {
        mapper_save_path: base.join(MAPPER_SAVE_FILE_NAME),
        states_dir: base.join(STATES_DIR_NAME),
    }
}

fn settings_paths(identity: &HostBackendIdentity) -> Result<SettingsPaths, SettingsError> {
    let Some(project_dirs) = ProjectDirs::from("com", "github.chalharu", "nerust") else {
        return Err(SettingsError::DirectoriesUnavailable);
    };
    let config_dir = project_dirs.config_dir().to_path_buf();
    let data_dir = project_dirs.data_local_dir().to_path_buf();
    Ok(SettingsPaths {
        shared_settings_file: config_dir.join(SHARED_SETTINGS_FILE_NAME),
        local_settings_file: config_dir
            .join(LOCAL_SETTINGS_DIR_NAME)
            .join(format!("{}.yaml", identity.file_stem())),
        app_state_file: data_dir.join(APP_STATE_FILE_NAME),
        central_storage_root: data_dir.join(CENTRAL_STORAGE_DIR_NAME),
        config_dir,
        data_dir,
    })
}

fn load_settings_document<T: Clone + serde::de::DeserializeOwned + serde::Serialize>(
    path: &Path,
    defaults: &T,
    schema_version: u32,
) -> Result<LoadedSettingsDocument<T>, SettingsError> {
    match fs::read_to_string(path) {
        Ok(contents) => {
            let raw: Value = serde_yaml::from_str(&contents)?;
            ensure_supported_schema_version(&raw, schema_version)?;
            Ok(LoadedSettingsDocument {
                settings: decode_settings_document(defaults, raw.clone())?,
                raw,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(LoadedSettingsDocument {
            settings: defaults.clone(),
            raw: empty_mapping(),
        }),
        Err(error) => Err(error.into()),
    }
}

fn save_snapshot_store(
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

fn write_yaml<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_yaml::to_string(value)?)?;
    Ok(())
}

fn normalize_loaded_settings<T: serde::Serialize + serde::de::DeserializeOwned>(
    defaults: &T,
    loaded: T,
) -> Result<T, SettingsError> {
    decode_settings_document(defaults, serde_yaml::to_value(loaded)?)
}

fn decode_settings_document<T: serde::Serialize + serde::de::DeserializeOwned>(
    defaults: &T,
    loaded: Value,
) -> Result<T, SettingsError> {
    Ok(serde_yaml::from_value(merge_with_defaults(
        serde_yaml::to_value(defaults)?,
        loaded,
    ))?)
}

fn merge_serialized_value<T: serde::Serialize>(
    existing: Value,
    value: &T,
) -> Result<Value, SettingsError> {
    Ok(merge_with_defaults(existing, serde_yaml::to_value(value)?))
}

fn normalize_shared_settings(mut settings: DesktopSharedSettings) -> DesktopSharedSettings {
    settings.schema_version = DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION;
    settings
}

fn normalize_local_settings(mut settings: HostBackendLocalSettings) -> HostBackendLocalSettings {
    settings.schema_version = HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION;
    settings
}

fn normalize_app_state(mut settings: DesktopAppState) -> DesktopAppState {
    settings.schema_version = DESKTOP_APP_STATE_SCHEMA_VERSION;
    settings
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

fn merge_with_defaults(mut defaults: Value, overlay: Value) -> Value {
    merge_yaml(&mut defaults, overlay);
    defaults
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

fn live_system_settings_changed(
    before: &DesktopSharedSettings,
    after: &DesktopSharedSettings,
) -> bool {
    nes_live_filter(before) != nes_live_filter(after)
}

fn nes_live_filter(settings: &DesktopSharedSettings) -> NesVideoFilter {
    settings
        .systems
        .get(&SystemId::Nes)
        .map(|settings| match settings {
            SystemSettings::Nes(nes) => nes.video.filter,
        })
        .unwrap_or_default()
}

fn empty_mapping() -> Value {
    Value::Mapping(Default::default())
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

fn sanitize_path_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn canonical_rom_identity(system: SystemId, rom_identity: RomIdentity) -> String {
    format!(
        "v1|{}|{:04x}|{:02x}|{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{:016x}|{:016x}|{:016x}",
        system_id_slug(system),
        rom_identity.mapper_type,
        rom_identity.sub_mapper_type,
        rom_identity.format,
        rom_identity.has_battery,
        rom_identity.trainer_len,
        rom_identity.prg_rom_len,
        rom_identity.chr_rom_len,
        rom_identity.prg_ram_len,
        rom_identity.save_prg_ram_len,
        rom_identity.chr_ram_len,
        rom_identity.save_chr_ram_len,
        mirror_mode_slug(rom_identity.mirror_mode),
        rom_identity.prg_rom_crc64,
        rom_identity.chr_rom_crc64,
        rom_identity.trainer_crc64,
    )
}

fn mirror_mode_slug(mode: nerust_contract_mirror::MirrorMode) -> String {
    match mode {
        nerust_contract_mirror::MirrorMode::Horizontal => "horizontal".to_string(),
        nerust_contract_mirror::MirrorMode::Vertical => "vertical".to_string(),
        nerust_contract_mirror::MirrorMode::Single0 => "single0".to_string(),
        nerust_contract_mirror::MirrorMode::Single1 => "single1".to_string(),
        nerust_contract_mirror::MirrorMode::Four => "four".to_string(),
        nerust_contract_mirror::MirrorMode::Custom(lut) => {
            let mut text = "custom".to_string();
            for value in lut {
                text.push_str(format!("-{value}").as_str());
            }
            text
        }
    }
}

fn system_id_slug(system: SystemId) -> &'static str {
    match system {
        SystemId::Nes => "nes",
        SystemId::Snes => "snes",
        SystemId::Ps1 => "ps1",
        SystemId::MegaDrive => "megadrive",
    }
}

fn storage_is_empty(paths: &SidecarPaths) -> Result<bool, SettingsError> {
    let mapper_exists = paths.mapper_save_path.is_file();
    let states_exist = match fs::read_dir(&paths.states_dir) {
        Ok(mut entries) => entries.next().is_some(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    Ok(!mapper_exists && !states_exist)
}

fn copy_storage_contents(
    source: &SidecarPaths,
    destination: &SidecarPaths,
) -> Result<(), SettingsError> {
    if source.mapper_save_path.is_file() {
        if let Some(parent) = destination.mapper_save_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _ = fs::copy(&source.mapper_save_path, &destination.mapper_save_path)?;
    }
    match fs::read_dir(&source.states_dir) {
        Ok(entries) => {
            fs::create_dir_all(&destination.states_dir)?;
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    let _ = fs::copy(&path, destination.states_dir.join(entry.file_name()))?;
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

trait LocalSettingsExt {
    fn local_audio_volume_percent(&self) -> u16;
}

impl LocalSettingsExt for HostBackendLocalSettings {
    fn local_audio_volume_percent(&self) -> u16 {
        u16::from(self.audio.master_volume_percent)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HostBackendIdentity, SettingsApplyPlan, SettingsManager, SettingsSnapshot,
        merge_with_defaults, resolve_central_storage_paths, resolve_persistence_paths_with_import,
        system_storage_key,
    };
    use nerust_contract_mirror::MirrorMode;
    use nerust_contract_options::Mmc3IrqVariant;
    use nerust_contract_rom::{RomFormat, RomIdentity};
    use nerust_contract_settings::app_state::DesktopAppState;
    use nerust_contract_settings::input::{
        IMPLICIT_PROFILE_ID, InputSettings, KeyboardBinding, KeyboardKey, PersistedControlId,
        ShortcutAction, ShortcutBinding, SystemInputSettings,
    };
    use nerust_contract_settings::local::{HostBackendLocalSettings, ScalingMode};
    use nerust_contract_settings::shared::{DesktopSharedSettings, StoragePolicy, SystemSettings};
    use nerust_contract_settings::{
        language::AppLanguage,
        nes::{NesSettings, NesVideoFilter},
    };
    use nerust_input_schema::SystemId;
    use nerust_persistence::sidecar::resolve_sidecars;
    use serde_yaml::{Mapping, Value};
    use std::collections::BTreeMap;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

    fn test_rom_identity() -> RomIdentity {
        RomIdentity {
            format: RomFormat::INes,
            mapper_type: 4,
            sub_mapper_type: 1,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: true,
            trainer_len: 0,
            prg_rom_len: 0x8000,
            chr_rom_len: 0x2000,
            prg_ram_len: 0,
            save_prg_ram_len: 0x2000,
            chr_ram_len: 0,
            save_chr_ram_len: 0,
            prg_rom_crc64: 0x11,
            chr_rom_crc64: 0x22,
            trainer_crc64: 0x33,
        }
    }

    fn test_shared_defaults() -> DesktopSharedSettings {
        DesktopSharedSettings {
            systems: BTreeMap::from([(SystemId::Nes, SystemSettings::Nes(NesSettings::default()))]),
            ..Default::default()
        }
    }

    fn test_local_defaults() -> HostBackendLocalSettings {
        HostBackendLocalSettings::default()
    }

    fn test_root(name: &str) -> PathBuf {
        env::current_dir()
            .unwrap()
            .join("target")
            .join("gui-runtime-settings")
            .join(name)
    }

    #[test]
    fn merge_with_defaults_backfills_missing_fields() {
        let merged = merge_with_defaults(
            serde_yaml::to_value(test_shared_defaults()).unwrap(),
            Value::Mapping(Mapping::from_iter([(
                Value::String("general".into()),
                Value::Mapping(Mapping::from_iter([(
                    Value::String("language".into()),
                    Value::String("english".into()),
                )])),
            )])),
        );

        let decoded: DesktopSharedSettings = serde_yaml::from_value(merged).unwrap();
        assert_eq!(decoded.general.language, AppLanguage::English);
        assert!(decoded.systems.contains_key(&SystemId::Nes));
    }

    #[test]
    fn central_storage_paths_use_system_and_identity_not_rom_path() {
        let root = PathBuf::from("/base");
        let identity = test_rom_identity();
        let first = resolve_central_storage_paths(&root, SystemId::Nes, identity);
        let second = resolve_central_storage_paths(&root, SystemId::Nes, identity);

        assert_eq!(first, second);
        assert!(first.mapper_save_path.ends_with("mapper.sav"));
        assert!(first.states_dir.ends_with("states"));
        assert!(system_storage_key(SystemId::Nes, identity).contains("m0004"));
    }

    #[test]
    fn sidecar_imports_into_empty_central_storage() {
        let root = test_root("import-sidecar-to-central");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let rom_path = root.join("game.nes");
        fs::write(&rom_path, [0_u8; 4]).unwrap();
        let sidecar = resolve_sidecars(&rom_path);
        fs::write(&sidecar.mapper_save_path, b"mapper").unwrap();
        fs::create_dir_all(&sidecar.states_dir).unwrap();
        fs::write(sidecar.states_dir.join("slot-1.zip"), b"state").unwrap();

        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::CustomDirectory;
        shared.persistence.storage_directory = Some(root.join("central"));

        let resolved = resolve_persistence_paths_with_import(
            &shared,
            None,
            SystemId::Nes,
            Some(&rom_path),
            test_rom_identity(),
        )
        .unwrap();

        assert_eq!(fs::read(&resolved.mapper_save_path).unwrap(), b"mapper");
        assert_eq!(
            fs::read(resolved.states_dir.join("slot-1.zip")).unwrap(),
            b"state"
        );
    }

    #[test]
    fn central_storage_wins_when_destination_is_not_empty() {
        let root = test_root("central-destination-wins");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let rom_path = root.join("game.nes");
        fs::write(&rom_path, [0_u8; 4]).unwrap();
        let sidecar = resolve_sidecars(&rom_path);
        fs::write(&sidecar.mapper_save_path, b"sidecar").unwrap();

        let central_root = root.join("central");
        let central =
            resolve_central_storage_paths(&central_root, SystemId::Nes, test_rom_identity());
        fs::create_dir_all(central.mapper_save_path.parent().unwrap()).unwrap();
        fs::write(&central.mapper_save_path, b"central").unwrap();

        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::CustomDirectory;
        shared.persistence.storage_directory = Some(central_root);

        let resolved = resolve_persistence_paths_with_import(
            &shared,
            None,
            SystemId::Nes,
            Some(&rom_path),
            test_rom_identity(),
        )
        .unwrap();

        assert_eq!(fs::read(&resolved.mapper_save_path).unwrap(), b"central");
    }

    #[test]
    fn app_shared_data_imports_from_custom_storage_when_sidecar_is_empty() {
        let root = test_root("app-shared-imports-custom");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let rom_path = root.join("game.nes");
        fs::write(&rom_path, [0_u8; 4]).unwrap();
        let custom_root = root.join("custom");
        let custom =
            resolve_central_storage_paths(&custom_root, SystemId::Nes, test_rom_identity());
        fs::create_dir_all(custom.mapper_save_path.parent().unwrap()).unwrap();
        fs::write(&custom.mapper_save_path, b"custom").unwrap();

        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::AppSharedData;
        shared.persistence.storage_directory = Some(custom_root);
        let paths = super::SettingsPaths {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            shared_settings_file: root.join("config/shared.yaml"),
            local_settings_file: root.join("config/local.yaml"),
            app_state_file: root.join("data/app-state.yaml"),
            central_storage_root: root.join("data/persistence"),
        };

        let resolved = resolve_persistence_paths_with_import(
            &shared,
            Some(&paths),
            SystemId::Nes,
            Some(&rom_path),
            test_rom_identity(),
        )
        .unwrap();

        assert_eq!(fs::read(&resolved.mapper_save_path).unwrap(), b"custom");
    }

    #[test]
    fn merge_serialized_value_preserves_unknown_fields() {
        let merged = super::merge_serialized_value(
            Value::Mapping(Mapping::from_iter([(
                Value::String("future".into()),
                Value::String("keep-me".into()),
            )])),
            &test_shared_defaults(),
        )
        .unwrap();

        assert_eq!(
            merged
                .as_mapping()
                .unwrap()
                .get(Value::String("future".into()))
                .unwrap(),
            &Value::String("keep-me".into())
        );
    }

    #[test]
    fn merge_serialized_value_preserves_unknown_fields_inside_sequence_items() {
        let mut shared = test_shared_defaults();
        shared.input = InputSettings {
            systems: BTreeMap::from([(SystemId::Nes, {
                let mut system = SystemInputSettings::default();
                system.keyboard_profiles.insert(
                    IMPLICIT_PROFILE_ID.to_string(),
                    nerust_contract_settings::input::KeyboardProfile {
                        bindings: vec![KeyboardBinding::new(
                            "nes.attachment.player1",
                            PersistedControlId::digital("nes.control.a"),
                            KeyboardKey::KeyZ,
                        )],
                    },
                );
                system
            })]),
            shortcuts: nerust_contract_settings::input::ShortcutSettings {
                keyboard: vec![ShortcutBinding {
                    action: ShortcutAction::TogglePause,
                    key: Some(KeyboardKey::Space),
                }],
            },
        };
        let existing: Value = serde_yaml::from_str(
            r#"
input:
  systems:
    nes:
      keyboard_profiles:
        default:
          bindings:
            - attachment: nes.attachment.player1
              control:
                kind: digital
                id: nes.control.a
              key: key_z
              future: keep-binding
  shortcuts:
    keyboard:
      - action: toggle_pause
        key: space
        future: keep-shortcut
"#,
        )
        .unwrap();

        let merged = super::merge_serialized_value(existing, &shared).unwrap();
        let input = merged
            .as_mapping()
            .unwrap()
            .get(Value::String("input".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        let bindings = input
            .get(Value::String("systems".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(Value::String("nes".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(Value::String("keyboard_profiles".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(Value::String("default".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(Value::String("bindings".into()))
            .unwrap()
            .as_sequence()
            .unwrap();
        let shortcuts = input
            .get(Value::String("shortcuts".into()))
            .unwrap()
            .as_mapping()
            .unwrap()
            .get(Value::String("keyboard".into()))
            .unwrap()
            .as_sequence()
            .unwrap();

        assert_eq!(
            bindings[0]
                .as_mapping()
                .unwrap()
                .get(Value::String("future".into()))
                .unwrap(),
            &Value::String("keep-binding".into())
        );
        assert_eq!(
            shortcuts[0]
                .as_mapping()
                .unwrap()
                .get(Value::String("future".into()))
                .unwrap(),
            &Value::String("keep-shortcut".into())
        );
    }

    #[test]
    fn apply_plan_flags_changed_categories() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        after.shared.general.language = AppLanguage::Japanese;
        after.local.video.scaling = ScalingMode::X3;
        after.local.audio.latency_ms = 90;

        let plan = super::derive_apply_plan(&before, &after);

        assert_eq!(
            plan,
            SettingsApplyPlan {
                language_changed: true,
                bindings_changed: false,
                persistence_changed: false,
                session_rebuild_required: true,
                scaling_changed: true,
                vsync_changed: false,
                fullscreen_default_changed: false,
            }
        );
    }

    #[test]
    fn filter_change_requires_immediate_session_rebuild() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        let SystemSettings::Nes(nes) = after.shared.systems.get_mut(&SystemId::Nes).unwrap();
        nes.video.filter = NesVideoFilter::NtscRgb;

        let plan = super::derive_apply_plan(&before, &after);

        assert!(plan.session_rebuild_required);
    }

    #[test]
    fn mmc3_variant_change_waits_for_next_rom_load() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        let SystemSettings::Nes(nes) = after.shared.systems.get_mut(&SystemId::Nes).unwrap();
        nes.core.mmc3_irq_variant = Some(Mmc3IrqVariant::Sharp);

        let plan = super::derive_apply_plan(&before, &after);

        assert!(!plan.session_rebuild_required);
    }

    #[test]
    fn ephemeral_manager_round_trips_snapshot() {
        let manager = SettingsManager::ephemeral(
            test_shared_defaults(),
            test_local_defaults(),
            DesktopAppState::default(),
        );
        let mut snapshot = manager.snapshot().unwrap();
        snapshot.shared.input = InputSettings {
            systems: BTreeMap::from([(SystemId::Nes, {
                let mut system = SystemInputSettings::default();
                system.keyboard_profiles.insert(
                    IMPLICIT_PROFILE_ID.to_string(),
                    nerust_contract_settings::input::KeyboardProfile {
                        bindings: vec![KeyboardBinding::new(
                            "nes.attachment.player1",
                            PersistedControlId::digital("nes.control.a"),
                            KeyboardKey::KeyZ,
                        )],
                    },
                );
                system
            })]),
            shortcuts: nerust_contract_settings::input::ShortcutSettings {
                keyboard: vec![ShortcutBinding {
                    action: ShortcutAction::TogglePause,
                    key: Some(KeyboardKey::Space),
                }],
            },
        };

        manager.save_snapshot(snapshot.clone()).unwrap();

        assert_eq!(manager.snapshot().unwrap(), snapshot);
    }

    #[test]
    fn validate_shared_settings_does_not_create_custom_directory_during_validation() {
        let root = test_root("validate-custom-directory");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let custom_directory = root.join("custom").join("nested");
        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::CustomDirectory;
        shared.persistence.storage_directory = Some(custom_directory.clone());

        super::validate_shared_settings(&shared).unwrap();

        assert!(!custom_directory.exists());
    }

    #[test]
    fn host_backend_identity_formats_stably() {
        assert_eq!(HostBackendIdentity::gtk_opengl().to_string(), "gtk+opengl");
        assert_eq!(
            HostBackendIdentity::glutin_opengl().to_string(),
            "glutin+opengl"
        );
        assert_eq!(HostBackendIdentity::tao_wgpu().to_string(), "tao+wgpu");
    }
}
