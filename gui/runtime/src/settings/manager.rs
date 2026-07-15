use std::{
    path::Path,
    sync::{Arc, RwLock},
};

use nerust_core_traits::identity::{SystemId, SystemIdentity};
use nerust_gui_settings::{
    app_state::{DesktopAppState, RememberedWindowSize},
    local::HostBackendLocalSettings,
    shared::DesktopSharedSettings,
};
use nerust_persistence::sidecar::SidecarPaths;

use super::{
    SettingsError, SettingsPaths, SettingsSnapshot, SettingsStore,
    apply::{validate_local_settings, validate_shared_settings},
    persistence::{resolve_persistence_paths, resolve_persistence_paths_with_import},
    store::{load_snapshot, save_snapshot_store, settings_paths},
};

#[derive(Clone, Debug)]
pub struct SettingsManager {
    inner: Arc<RwLock<SettingsState>>,
}

#[derive(Debug)]
struct SettingsState {
    defaults: SettingsSnapshot,
    current: SettingsSnapshot,
    store: SettingsStore,
}

impl SettingsManager {
    pub fn load(
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Result<Self, SettingsError> {
        let paths = settings_paths()?;
        Self::load_with_paths(paths, shared_defaults, local_defaults, app_state_defaults)
    }

    pub fn load_with_paths(
        paths: SettingsPaths,
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Result<Self, SettingsError> {
        let defaults = SettingsSnapshot {
            shared: shared_defaults,
            local: local_defaults,
            app_state: app_state_defaults,
        };
        let current = load_snapshot(&paths.settings_file, &defaults);
        Ok(Self {
            inner: Arc::new(RwLock::new(SettingsState {
                defaults: defaults.clone(),
                current,
                store: SettingsStore::FileBacked(paths),
            })),
        })
    }

    pub fn load_or_ephemeral(
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Self {
        match Self::load(
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

    pub fn load_or_ephemeral_with_paths(
        paths: SettingsPaths,
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Self {
        let defaults = SettingsSnapshot {
            shared: shared_defaults,
            local: local_defaults,
            app_state: app_state_defaults,
        };
        match Self::load_with_paths_inner(paths, &defaults) {
            Ok(manager) => manager,
            Err(error) => {
                log::warn!("settings persistence unavailable; using ephemeral settings: {error}");
                Self::ephemeral(defaults.shared, defaults.local, defaults.app_state)
            }
        }
    }

    fn load_with_paths_inner(
        paths: SettingsPaths,
        defaults: &SettingsSnapshot,
    ) -> Result<Self, SettingsError> {
        let current = load_snapshot(&paths.settings_file, defaults);
        Ok(Self {
            inner: Arc::new(RwLock::new(SettingsState {
                defaults: defaults.clone(),
                current,
                store: SettingsStore::FileBacked(paths),
            })),
        })
    }

    pub fn ephemeral(
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Self {
        let defaults = SettingsSnapshot {
            shared: shared_defaults,
            local: local_defaults,
            app_state: app_state_defaults,
        };
        Self {
            inner: Arc::new(RwLock::new(SettingsState {
                current: defaults.clone(),
                defaults,
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

    pub fn save_snapshot(&self, snapshot: SettingsSnapshot) -> Result<(), SettingsError> {
        validate_shared_settings(&snapshot.shared)?;
        validate_local_settings(&snapshot.local)?;

        let mut guard = self
            .inner
            .write()
            .map_err(|_| SettingsError::LockPoisoned)?;
        save_snapshot_store(&guard.store, &snapshot)?;
        guard.current = snapshot;
        Ok(())
    }

    pub fn reload(&self) -> Result<SettingsSnapshot, SettingsError> {
        let mut guard = self
            .inner
            .write()
            .map_err(|_| SettingsError::LockPoisoned)?;
        let loaded = match &guard.store {
            SettingsStore::FileBacked(paths) => {
                load_snapshot(&paths.settings_file, &guard.defaults)
            }
            SettingsStore::Ephemeral => guard.current.clone(),
        };
        guard.current = loaded.clone();
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

    const WINDOW_SIZE_KEY: &'static str = "main";

    pub fn update_window_size(&self, width: u32, height: u32) -> Result<(), SettingsError> {
        let mut snapshot = self.snapshot()?;
        snapshot.app_state.set_window_size(
            Self::WINDOW_SIZE_KEY,
            RememberedWindowSize { width, height },
        );
        self.save_snapshot(snapshot)
    }

    pub fn resolve_persistence_paths(
        &self,
        system: SystemId,
        rom_path: Option<&Path>,
        identity: &SystemIdentity,
    ) -> Result<SidecarPaths, SettingsError> {
        let snapshot = self.snapshot()?;
        resolve_persistence_paths(
            &snapshot.shared,
            self.paths()?.as_ref(),
            system,
            rom_path,
            identity,
        )
    }

    pub fn resolve_persistence_paths_with_import(
        &self,
        system: SystemId,
        rom_path: Option<&Path>,
        identity: &SystemIdentity,
    ) -> Result<SidecarPaths, SettingsError> {
        let snapshot = self.snapshot()?;
        let paths = self.paths()?;
        resolve_persistence_paths_with_import(
            &snapshot.shared,
            paths.as_ref(),
            system,
            rom_path,
            identity,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use nerust_core_traits::identity::SystemId;
    use nerust_gui_settings::{
        app_state::{DesktopAppState, RememberedWindowSize},
        input::{
            IMPLICIT_PROFILE_ID, InputSettings, KeyboardBinding, KeyboardKey, PersistedControlId,
            ShortcutAction, ShortcutBinding, SystemInputSettings,
        },
        language::AppLanguage,
        nes::NesVideoFilter,
        shared::SystemSettings,
    };

    use super::super::{SettingsPaths, test_local_defaults, test_root, test_shared_defaults};
    use super::SettingsManager;

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

        let paths = SettingsPaths::from_root(root.clone());
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
    fn unknown_enum_variant_resets_only_that_field() {
        let dir = std::env::temp_dir().join(format!("nerust-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let path = dir.join("config").join("settings.yaml");
        std::fs::write(
            &path,
            b"shared:\n  general:\n    language: future_variant\n",
        )
        .unwrap();

        let paths = SettingsPaths::from_root(dir.clone());
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
        let dir = std::env::temp_dir().join(format!("nerust-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir should be creatable");

        let paths = SettingsPaths::from_root(dir.clone());
        let manager = SettingsManager::load_with_paths(
            paths.clone(),
            test_shared_defaults(),
            test_local_defaults(),
            DesktopAppState::default(),
        )
        .unwrap();

        let mut snap = manager.snapshot().unwrap();
        let nes = snap.shared.systems.get_mut(&SystemId::new("nes")).unwrap();
        let SystemSettings::Nes(nes_settings) = nes;
        nes_settings.video.filter = NesVideoFilter::NtscRgb;
        manager.save_snapshot(snap.clone()).unwrap();
        drop(manager);

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
