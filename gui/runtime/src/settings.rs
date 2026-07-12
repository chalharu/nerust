use std::path::PathBuf;

use nerust_gui_settings::{
    app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
};

pub mod apply;
pub mod manager;
pub mod persistence;
mod store;
mod tests;

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
