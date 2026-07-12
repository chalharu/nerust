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

use crate::settings::{
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
