use std::path::PathBuf;

use nerust_gui_settings::{
    app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
};

pub mod apply;
pub mod manager;
pub mod persistence;
mod store;

#[derive(Debug)]
pub(super) enum SettingsStore {
    FileBacked(SettingsPaths),
    Ephemeral,
}

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

/// Frontend/backend capabilities, constructed directly by each frontend.
///
/// Replaces the closed `HostBackendProfile` enum. Each frontend specifies
/// its own capabilities rather than being matched against a fixed set of
/// (host_kind, render_backend_kind) pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostBackendCapabilities {
    pub window: HostWindowCapabilities,
    pub presentation: Option<BackendPresentationCapabilities>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPaths {
    pub settings_file: PathBuf,
    pub central_storage_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    pub audio_volume_changed: bool,
    pub renderer_rebuild_required: bool,
    pub window_settings_changed: bool,
    pub backend_presentation_changed: bool,
    pub scaling_changed: bool,
    pub vsync_changed: bool,
    pub fullscreen_default_changed: bool,
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, env, fs, path::PathBuf};

    use super::{
        BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities,
        SettingsApplyPlan, SettingsSnapshot,
        apply::{derive_apply_plan, validate_shared_settings},
        manager::SettingsManager,
        persistence::{
            resolve_central_storage_paths, resolve_persistence_paths_with_import,
            system_storage_key,
        },
    };
    use nerust_core_traits::identity::SystemIdentity;
    use nerust_gui_settings::{
        app_state::{DesktopAppState, RememberedWindowSize},
        input::{
            IMPLICIT_PROFILE_ID, InputSettings, KeyboardBinding, KeyboardKey, PersistedControlId,
            ShortcutAction, ShortcutBinding, SystemInputSettings,
        },
        language::AppLanguage,
        local::{HostBackendLocalSettings, ScalingMode},
        nes::{Mmc3IrqVariant, NesSettings, NesVideoFilter},
        shared::{DesktopSharedSettings, StoragePolicy, SystemSettings},
    };
    use nerust_core_traits::SystemId;
    use nerust_persistence::sidecar::resolve_sidecars;

    fn tao_caps() -> HostBackendCapabilities {
        HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: true,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: Some(BackendPresentationCapabilities {
                supports_vsync: true,
            }),
        }
    }

    fn gtk_caps() -> HostBackendCapabilities {
        HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        }
    }

    fn test_system_identity() -> SystemIdentity {
        SystemIdentity::new(SystemId::new("nes"), vec![4, 1, 0x11, 0x22, 0x33])
    }

    fn test_shared_defaults() -> DesktopSharedSettings {
        DesktopSharedSettings {
            systems: BTreeMap::from([(
                SystemId::new("nes"),
                SystemSettings::Nes(NesSettings::default()),
            )]),
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
    fn central_storage_paths_use_system_and_identity_not_rom_path() {
        let root = PathBuf::from("/base");
        let identity = test_system_identity();
        let first = resolve_central_storage_paths(&root, SystemId::new("nes"), &identity);
        let second = resolve_central_storage_paths(&root, SystemId::new("nes"), &identity);

        assert_eq!(first, second);
        assert!(first.mapper_save_path.ends_with("mapper.sav"));
        assert!(first.states_dir.ends_with("states"));
        assert!(!system_storage_key(SystemId::new("nes"), &identity).is_empty());
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

        let identity = test_system_identity();
        let resolved = resolve_persistence_paths_with_import(
            &shared,
            None,
            SystemId::new("nes"),
            Some(&rom_path),
            &identity,
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
        let identity = test_system_identity();
        let central = resolve_central_storage_paths(&central_root, SystemId::new("nes"), &identity);
        fs::create_dir_all(central.mapper_save_path.parent().unwrap()).unwrap();
        fs::write(&central.mapper_save_path, b"central").unwrap();

        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::CustomDirectory;
        shared.persistence.storage_directory = Some(central_root);

        let identity = test_system_identity();
        let resolved = resolve_persistence_paths_with_import(
            &shared,
            None,
            SystemId::new("nes"),
            Some(&rom_path),
            &identity,
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
        let identity = test_system_identity();
        let custom = resolve_central_storage_paths(&custom_root, SystemId::new("nes"), &identity);
        fs::create_dir_all(custom.mapper_save_path.parent().unwrap()).unwrap();
        fs::write(&custom.mapper_save_path, b"custom").unwrap();

        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::AppSharedData;
        shared.persistence.storage_directory = Some(custom_root);
        let paths = super::SettingsPaths {
            settings_file: root.join("config/settings.yaml"),
            central_storage_root: root.join("data/persistence"),
        };

        let resolved = resolve_persistence_paths_with_import(
            &shared,
            Some(&paths),
            SystemId::new("nes"),
            Some(&rom_path),
            &test_system_identity(),
        )
        .unwrap();

        assert_eq!(fs::read(&resolved.mapper_save_path).unwrap(), b"custom");
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

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

        assert_eq!(
            plan,
            SettingsApplyPlan {
                language_changed: true,
                bindings_changed: false,
                persistence_changed: false,
                session_rebuild_required: true,
                audio_volume_changed: false,
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
        let SystemSettings::Nes(nes) = after.shared.systems.get_mut(&SystemId::new("nes")).unwrap();
        nes.video.filter = NesVideoFilter::NtscRgb;

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

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
        let SystemSettings::Nes(nes) = after.shared.systems.get_mut(&SystemId::new("nes")).unwrap();
        nes.core.mmc3_irq_variant = Some(Mmc3IrqVariant::Sharp);

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

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

        let plan = derive_apply_plan(&gtk_caps(), &before, &after);

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

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

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

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

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
            systems: BTreeMap::from([(SystemId::new("nes"), {
                let mut system = SystemInputSettings::default();
                system.keyboard_profiles.insert(
                    IMPLICIT_PROFILE_ID.to_string(),
                    nerust_gui_settings::input::KeyboardProfile {
                        bindings: vec![KeyboardBinding::new(
                            "nes.attachment.player1",
                            PersistedControlId::digital("nes.control.a"),
                            KeyboardKey::KeyZ,
                        )],
                    },
                );
                system
            })]),
            shortcuts: nerust_gui_settings::input::ShortcutSettings {
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

        let paths = super::SettingsPaths::from_root(root.clone());
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
        let SystemSettings::Nes(nes) = snapshot
            .shared
            .systems
            .get_mut(&SystemId::new("nes"))
            .unwrap();
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
    fn update_window_size_uses_fixed_key() {
        let manager = SettingsManager::ephemeral(
            test_shared_defaults(),
            test_local_defaults(),
            DesktopAppState::default(),
        );

        manager.update_window_size(960, 720).unwrap();
        manager.update_window_size(800, 600).unwrap();

        let app_state = manager.app_state().unwrap();

        // All callers use the same fixed key, so the second value replaces the first.
        assert!(app_state.window_size("tao+wgpu").is_none());
        assert!(app_state.window_size("gtk+opengl").is_none());
        assert_eq!(
            app_state.window_size("main"),
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
    fn host_backend_capabilities_carry_individual_backend_values() {
        let caps = super::HostBackendCapabilities {
            window: super::HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: false,
                supports_scaling: false,
            },
            presentation: Some(super::BackendPresentationCapabilities {
                supports_vsync: true,
            }),
        };
        assert!(!caps.window.remembers_window_size);
        assert!(!caps.window.supports_fullscreen_default);
        assert!(!caps.window.supports_scaling);
        assert!(caps.presentation.is_some_and(|p| p.supports_vsync));
    }

    #[test]
    fn settings_paths_can_be_built_from_an_explicit_root() {
        let root = PathBuf::from("/tmp/nerust-test");
        let paths = super::SettingsPaths::from_root(root.clone());

        assert_eq!(
            paths.settings_file,
            root.join("config").join("settings.yaml")
        );
        assert_eq!(
            paths.central_storage_root,
            root.join("data").join("persistence")
        );
    }

    #[test]
    fn unknown_enum_variant_resets_only_that_field() {
        use nerust_gui_settings::language::AppLanguage;

        let dir = std::env::temp_dir().join(format!("nerust-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let path = dir.join("config").join("settings.yaml");
        std::fs::write(
            &path,
            b"shared:\n  general:\n    language: future_variant\n",
        )
        .unwrap();

        let paths = super::SettingsPaths::from_root(dir.clone());
        let manager = SettingsManager::load_with_paths(
            paths,
            test_shared_defaults(),
            test_local_defaults(),
            DesktopAppState::default(),
        )
        .unwrap();
        let snap = manager.snapshot().unwrap();

        assert_eq!(
            snap.shared.general.language,
            AppLanguage::SystemDefault,
            "unknown variant should fall back to default",
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ntsc_filter_survives_save_reload_cycle() {
        use nerust_gui_settings::nes::NesVideoFilter;
        use nerust_core_traits::SystemId;

        let dir = std::env::temp_dir().join(format!("nerust-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let paths = super::SettingsPaths::from_root(dir.clone());
        let manager = SettingsManager::load_with_paths(
            paths.clone(),
            test_shared_defaults(),
            test_local_defaults(),
            DesktopAppState::default(),
        )
        .unwrap();

        // Change NTSC filter
        let mut snap = manager.snapshot().unwrap();
        let nes = snap.shared.systems.get_mut(&SystemId::new("nes")).unwrap();
        let SystemSettings::Nes(nes_settings) = nes;
        nes_settings.video.filter = NesVideoFilter::NtscRgb;
        manager.save_snapshot(snap.clone()).unwrap();
        drop(manager);

        // Reload
        let manager2 = SettingsManager::load_with_paths(
            paths,
            test_shared_defaults(),
            test_local_defaults(),
            DesktopAppState::default(),
        )
        .unwrap();
        let snap2 = manager2.snapshot().unwrap();
        let nes2 = snap2.shared.systems.get(&SystemId::new("nes")).unwrap();
        let SystemSettings::Nes(nes_settings) = nes2;
        assert_eq!(
            nes_settings.video.filter,
            NesVideoFilter::NtscRgb,
            "NTSC filter should persist across save/reload",
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
