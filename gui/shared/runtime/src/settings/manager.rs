use super::apply::{validate_local_settings, validate_shared_settings};
use super::persistence::{resolve_persistence_paths, resolve_persistence_paths_with_import};
use super::store::{
    empty_mapping, load_settings_document, merge_serialized_value, normalize_app_state,
    normalize_loaded_settings, normalize_local_settings, normalize_shared_settings,
    save_snapshot_store, settings_paths, strip_legacy_local_video_fields,
};
use super::{HostBackendIdentity, SettingsError, SettingsPaths, SettingsSnapshot};
use nerust_contract_rom::RomIdentity;
use nerust_gui_settings::app_state::{
    DESKTOP_APP_STATE_SCHEMA_VERSION, DesktopAppState, RememberedWindowSize,
};
use nerust_gui_settings::local::{
    HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION, HostBackendLocalSettings,
};
use nerust_gui_settings::shared::{DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION, DesktopSharedSettings};
use nerust_input_schema::SystemId;
use nerust_persistence::sidecar::SidecarPaths;
use serde_yaml::Value;
use std::path::Path;
use std::sync::{Arc, RwLock};

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
pub(super) enum SettingsStore {
    FileBacked(SettingsPaths),
    Ephemeral,
}

#[derive(Debug)]
pub(super) struct LoadedSettingsDocument<T> {
    pub(super) settings: T,
    pub(super) raw: Value,
}

impl SettingsManager {
    pub fn load(
        identity: HostBackendIdentity,
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Result<Self, SettingsError> {
        let paths = settings_paths(&identity)?;
        Self::load_with_paths(paths, shared_defaults, local_defaults, app_state_defaults)
    }

    pub fn load_with_paths(
        paths: SettingsPaths,
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Result<Self, SettingsError> {
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

    pub fn load_or_ephemeral_with_paths(
        paths: SettingsPaths,
        shared_defaults: DesktopSharedSettings,
        local_defaults: HostBackendLocalSettings,
        app_state_defaults: DesktopAppState,
    ) -> Self {
        match Self::load_with_paths(
            paths,
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
        snapshot
            .app_state
            .set_window_size(identity.to_string(), RememberedWindowSize { width, height });
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
