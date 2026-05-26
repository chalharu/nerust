use nerust_contract_rom::RomIdentity;
use nerust_contract_settings::app_state::{DESKTOP_APP_STATE_SCHEMA_VERSION, DesktopAppState};
use nerust_contract_settings::local::{
    HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION, HostBackendLocalSettings,
};
use nerust_contract_settings::shared::{
    DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION, DesktopSharedSettings,
};
use nerust_input_schema::SystemId;
use nerust_persistence::sidecar::SidecarPaths;
use serde_yaml::Value;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

mod apply;
mod persistence;
mod store;

pub use self::apply::{derive_apply_plan, validate_local_settings, validate_shared_settings};
pub use self::persistence::{
    resolve_central_storage_paths, resolve_persistence_paths,
    resolve_persistence_paths_with_import, system_storage_key,
};
use self::store::{
    empty_mapping, load_settings_document, merge_serialized_value, normalize_app_state,
    normalize_loaded_settings, normalize_local_settings, normalize_shared_settings,
    save_snapshot_store, settings_paths, strip_legacy_local_video_fields,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKind {
    Gtk,
    Glutin,
    Tao,
}

impl HostKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gtk => "gtk",
            Self::Glutin => "glutin",
            Self::Tao => "tao",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderBackendKind {
    OpenGl,
    Wgpu,
}

impl RenderBackendKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenGl => "opengl",
            Self::Wgpu => "wgpu",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostWindowCapabilities {
    pub remembers_window_size: bool,
    pub supports_fullscreen_default: bool,
    pub supports_scaling: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendPresentationCapabilities {
    pub supports_vsync: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostBackendCapabilities {
    pub window: HostWindowCapabilities,
    pub presentation: Option<BackendPresentationCapabilities>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostBackendProfile {
    host: HostKind,
    backend: RenderBackendKind,
}

pub type HostBackendIdentity = HostBackendProfile;

impl HostBackendProfile {
    pub fn new(host: HostKind, backend: RenderBackendKind) -> Self {
        Self { host, backend }
    }

    pub fn host(&self) -> HostKind {
        self.host
    }

    pub fn backend(&self) -> RenderBackendKind {
        self.backend
    }

    pub fn capabilities(&self) -> HostBackendCapabilities {
        match (self.host, self.backend) {
            (HostKind::Gtk, RenderBackendKind::OpenGl) => HostBackendCapabilities {
                window: HostWindowCapabilities {
                    remembers_window_size: false,
                    supports_fullscreen_default: true,
                    supports_scaling: true,
                },
                presentation: None,
            },
            (HostKind::Glutin, RenderBackendKind::OpenGl) => HostBackendCapabilities {
                window: HostWindowCapabilities {
                    remembers_window_size: true,
                    supports_fullscreen_default: true,
                    supports_scaling: true,
                },
                presentation: None,
            },
            (HostKind::Tao, RenderBackendKind::Wgpu) => HostBackendCapabilities {
                window: HostWindowCapabilities {
                    remembers_window_size: true,
                    supports_fullscreen_default: true,
                    supports_scaling: true,
                },
                presentation: Some(BackendPresentationCapabilities {
                    supports_vsync: true,
                }),
            },
            _ => HostBackendCapabilities {
                window: HostWindowCapabilities {
                    remembers_window_size: true,
                    supports_fullscreen_default: true,
                    supports_scaling: true,
                },
                presentation: None,
            },
        }
    }

    pub fn gtk_opengl() -> Self {
        Self::new(HostKind::Gtk, RenderBackendKind::OpenGl)
    }

    pub fn glutin_opengl() -> Self {
        Self::new(HostKind::Glutin, RenderBackendKind::OpenGl)
    }

    pub fn tao_wgpu() -> Self {
        Self::new(HostKind::Tao, RenderBackendKind::Wgpu)
    }

    fn file_stem(&self) -> String {
        format!(
            "{}+{}",
            sanitize_path_component(self.host.label()),
            sanitize_path_component(self.backend.label())
        )
    }
}

impl fmt::Display for HostBackendProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}+{}", self.host.label(), self.backend.label())
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

#[derive(Debug, Clone, PartialEq, serde_derive::Serialize, serde_derive::Deserialize)]
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
    pub renderer_rebuild_required: bool,
    pub window_settings_changed: bool,
    pub backend_presentation_changed: bool,
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
        let local_document = merge_serialized_value(
            strip_legacy_local_video_fields(guard.local_document.clone()),
            &normalized.local,
        )?;
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

    pub fn update_window_size(
        &self,
        identity: &HostBackendIdentity,
        width: u32,
        height: u32,
    ) -> Result<(), SettingsError> {
        let mut snapshot = self.snapshot()?;
        snapshot.app_state.set_window_size(
            identity.to_string(),
            nerust_contract_settings::app_state::RememberedWindowSize { width, height },
        );
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

#[cfg(test)]
mod tests {
    use super::store::merge_with_defaults;
    use super::{
        HostBackendIdentity, SettingsApplyPlan, SettingsManager, SettingsSnapshot,
        resolve_central_storage_paths, resolve_persistence_paths_with_import, system_storage_key,
    };
    use nerust_contract_mirror::MirrorMode;
    use nerust_contract_options::Mmc3IrqVariant;
    use nerust_contract_rom::{RomFormat, RomIdentity};
    use nerust_contract_settings::app_state::{DesktopAppState, RememberedWindowSize};
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
        let merged = super::store::merge_serialized_value(
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

        let merged = super::store::merge_serialized_value(existing, &shared).unwrap();
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
    fn local_settings_save_prunes_legacy_flat_video_fields() {
        let existing: Value = serde_yaml::from_str(
            r#"
schema_version: 1
video:
  fullscreen_default: true
  scaling: x3
  vsync: false
  future: keep-video
"#,
        )
        .unwrap();
        let merged = super::store::merge_serialized_value(
            super::store::strip_legacy_local_video_fields(existing),
            &test_local_defaults(),
        )
        .unwrap();

        let decoded: HostBackendLocalSettings = serde_yaml::from_value(merged.clone()).unwrap();
        assert!(!decoded.video.window.fullscreen_default);
        assert_eq!(decoded.video.window.scaling, ScalingMode::FitToWindow);
        assert!(decoded.video.presentation.vsync);

        let video = merged
            .as_mapping()
            .unwrap()
            .get(Value::String("video".into()))
            .unwrap()
            .as_mapping()
            .unwrap();
        assert!(!video.contains_key(Value::String("fullscreen_default".into())));
        assert!(!video.contains_key(Value::String("scaling".into())));
        assert!(!video.contains_key(Value::String("vsync".into())));
        assert_eq!(
            video.get(Value::String("future".into())).unwrap(),
            &Value::String("keep-video".into())
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
        after.local.video.window.scaling = ScalingMode::X3;
        after.local.audio.latency_ms = 90;

        let plan = super::derive_apply_plan(HostBackendIdentity::tao_wgpu(), &before, &after);

        assert_eq!(
            plan,
            SettingsApplyPlan {
                language_changed: true,
                bindings_changed: false,
                persistence_changed: false,
                session_rebuild_required: true,
                renderer_rebuild_required: true,
                window_settings_changed: true,
                backend_presentation_changed: false,
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

        let plan = super::derive_apply_plan(HostBackendIdentity::tao_wgpu(), &before, &after);

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

        let plan = super::derive_apply_plan(HostBackendIdentity::tao_wgpu(), &before, &after);

        assert!(!plan.session_rebuild_required);
    }

    #[test]
    fn gtk_opengl_ignores_backend_presentation_changes() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        after.local.video.presentation.vsync = !after.local.video.presentation.vsync;

        let plan = super::derive_apply_plan(HostBackendIdentity::gtk_opengl(), &before, &after);

        assert!(plan.vsync_changed);
        assert!(!plan.backend_presentation_changed);
        assert!(!plan.renderer_rebuild_required);
    }

    #[test]
    fn tao_wgpu_rebuilds_renderer_for_vsync_changes() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        after.local.video.presentation.vsync = !after.local.video.presentation.vsync;

        let plan = super::derive_apply_plan(HostBackendIdentity::tao_wgpu(), &before, &after);

        assert!(plan.vsync_changed);
        assert!(plan.backend_presentation_changed);
        assert!(plan.renderer_rebuild_required);
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
    fn update_window_size_records_host_specific_app_state() {
        let manager = SettingsManager::ephemeral(
            test_shared_defaults(),
            test_local_defaults(),
            DesktopAppState::default(),
        );

        manager
            .update_window_size(&HostBackendIdentity::tao_wgpu(), 960, 720)
            .unwrap();
        manager
            .update_window_size(&HostBackendIdentity::gtk_opengl(), 800, 600)
            .unwrap();

        let app_state = manager.app_state().unwrap();

        assert_eq!(
            app_state.window_size("tao+wgpu"),
            Some(RememberedWindowSize {
                width: 960,
                height: 720,
            })
        );
        assert_eq!(
            app_state.window_size("gtk+opengl"),
            Some(RememberedWindowSize {
                width: 800,
                height: 600,
            })
        );
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
