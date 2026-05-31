use nerust_contract_settings::app_state::DesktopAppState;
use nerust_contract_settings::local::HostBackendLocalSettings;
use nerust_contract_settings::shared::DesktopSharedSettings;
use std::fmt;
use std::path::PathBuf;

pub mod apply;
pub mod manager;
pub mod persistence;
mod store;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("default settings directories are unavailable for this host")]
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
    Android,
    Gtk,
    Glutin,
    Tao,
}

impl HostKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Gtk => "gtk",
            Self::Glutin => "glutin",
            Self::Tao => "tao",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioBackendKind {
    OpenAl,
    Android,
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
            (HostKind::Android, RenderBackendKind::Wgpu) => HostBackendCapabilities {
                window: HostWindowCapabilities {
                    remembers_window_size: false,
                    supports_fullscreen_default: false,
                    supports_scaling: false,
                },
                presentation: Some(BackendPresentationCapabilities {
                    supports_vsync: true,
                }),
            },
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

    pub fn audio_backend(self) -> AudioBackendKind {
        match (self.host, self.backend) {
            (HostKind::Android, RenderBackendKind::Wgpu) => AudioBackendKind::Android,
            _ => AudioBackendKind::OpenAl,
        }
    }

    pub fn android_wgpu() -> Self {
        Self::new(HostKind::Android, RenderBackendKind::Wgpu)
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
    use super::apply::{derive_apply_plan, validate_shared_settings};
    use super::manager::SettingsManager;
    use super::persistence::{
        resolve_central_storage_paths, resolve_persistence_paths_with_import, system_storage_key,
    };
    use super::store::merge_with_defaults;
    use super::{HostBackendIdentity, SettingsApplyPlan, SettingsSnapshot};
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
        snes::SnesSettings,
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
            systems: BTreeMap::from([
                (SystemId::Nes, SystemSettings::Nes(NesSettings::default())),
                (
                    SystemId::Snes,
                    SystemSettings::Snes(SnesSettings::default()),
                ),
            ]),
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

        let plan = derive_apply_plan(HostBackendIdentity::tao_wgpu(), &before, &after);

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
        let SystemSettings::Nes(nes) = after.shared.systems.get_mut(&SystemId::Nes).unwrap() else {
            panic!("expected NES settings");
        };
        nes.video.filter = NesVideoFilter::NtscRgb;

        let plan = derive_apply_plan(HostBackendIdentity::tao_wgpu(), &before, &after);

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
        let SystemSettings::Nes(nes) = after.shared.systems.get_mut(&SystemId::Nes).unwrap() else {
            panic!("expected NES settings");
        };
        nes.core.mmc3_irq_variant = Some(Mmc3IrqVariant::Sharp);

        let plan = derive_apply_plan(HostBackendIdentity::tao_wgpu(), &before, &after);

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

        let plan = derive_apply_plan(HostBackendIdentity::gtk_opengl(), &before, &after);

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

        let plan = derive_apply_plan(HostBackendIdentity::tao_wgpu(), &before, &after);

        assert!(plan.vsync_changed);
        assert!(plan.backend_presentation_changed);
        assert!(plan.renderer_rebuild_required);
    }

    #[test]
    fn fullscreen_default_change_only_marks_window_settings() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        after.local.video.window.fullscreen_default = !after.local.video.window.fullscreen_default;

        let plan = derive_apply_plan(HostBackendIdentity::tao_wgpu(), &before, &after);

        assert!(plan.fullscreen_default_changed);
        assert!(plan.window_settings_changed);
        assert!(!plan.session_rebuild_required);
        assert!(!plan.renderer_rebuild_required);
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
    fn file_backed_manager_round_trips_snapshot_across_reloads() {
        let root = test_root("file-backed-roundtrip");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let paths =
            super::SettingsPaths::from_root(root.clone(), &HostBackendIdentity::gtk_opengl());
        let manager = SettingsManager::load_with_paths(
            paths.clone(),
            test_shared_defaults(),
            test_local_defaults(),
            DesktopAppState::default(),
        )
        .unwrap();

        let mut snapshot = manager.snapshot().unwrap();
        snapshot.shared.general.language = AppLanguage::Japanese;
        snapshot.local.audio.muted = true;
        let SystemSettings::Nes(nes) = snapshot.shared.systems.get_mut(&SystemId::Nes).unwrap();
        nes.video.filter = NesVideoFilter::NtscRgb;
        manager.save_snapshot(snapshot.clone()).unwrap();

        let reloaded = SettingsManager::load_with_paths(
            paths,
            test_shared_defaults(),
            test_local_defaults(),
            DesktopAppState::default(),
        )
        .unwrap()
        .snapshot()
        .unwrap();

        assert_eq!(reloaded, snapshot);

        let _ = fs::remove_dir_all(&root);
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

        validate_shared_settings(&shared).unwrap();

        assert!(!custom_directory.exists());
    }

    #[test]
    fn host_backend_identity_formats_stably() {
        assert_eq!(
            HostBackendIdentity::android_wgpu().to_string(),
            "android+wgpu"
        );
        assert_eq!(HostBackendIdentity::gtk_opengl().to_string(), "gtk+opengl");
        assert_eq!(
            HostBackendIdentity::glutin_opengl().to_string(),
            "glutin+opengl"
        );
        assert_eq!(HostBackendIdentity::tao_wgpu().to_string(), "tao+wgpu");
    }

    #[test]
    fn android_wgpu_profile_exposes_mobile_capabilities() {
        let profile = HostBackendIdentity::android_wgpu();

        assert_eq!(profile.audio_backend(), super::AudioBackendKind::Android);
        assert_eq!(
            profile.capabilities(),
            super::HostBackendCapabilities {
                window: super::HostWindowCapabilities {
                    remembers_window_size: false,
                    supports_fullscreen_default: false,
                    supports_scaling: false,
                },
                presentation: Some(super::BackendPresentationCapabilities {
                    supports_vsync: true,
                }),
            }
        );
    }

    #[test]
    fn settings_paths_can_be_built_from_an_explicit_root() {
        let root = PathBuf::from("/tmp/nerust-android");
        let paths =
            super::SettingsPaths::from_root(root.clone(), &HostBackendIdentity::android_wgpu());

        assert_eq!(paths.config_dir, root.join("config"));
        assert_eq!(paths.data_dir, root.join("data"));
        assert_eq!(
            paths.shared_settings_file,
            root.join("config").join("shared-settings.yaml")
        );
        assert_eq!(
            paths.local_settings_file,
            root.join("config")
                .join("local-settings")
                .join("android+wgpu.yaml")
        );
        assert_eq!(
            paths.app_state_file,
            root.join("data").join("app-state.yaml")
        );
        assert_eq!(
            paths.central_storage_root,
            root.join("data").join("persistence")
        );
    }
}
