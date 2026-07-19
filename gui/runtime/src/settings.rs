use std::path::PathBuf;

use nerust_gui_settings::{
    app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
};

#[cfg(test)]
use nerust_core_traits::identity::{SystemId, SystemIdentity};
#[cfg(test)]
use nerust_nes_settings::NesSettings;
#[cfg(test)]
use nerust_nes_settings::SystemSettings;
#[cfg(test)]
use std::{collections::BTreeMap, env};

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
    #[error("settings YAML serialization/deserialization failed: {0}")]
    Serialize(Box<dyn std::error::Error + Send + 'static>),
    #[error("settings I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings lock is poisoned")]
    LockPoisoned,
}

impl From<serde_saphyr::Error> for SettingsError {
    fn from(e: serde_saphyr::Error) -> Self {
        SettingsError::Serialize(Box::new(e))
    }
}

impl From<serde_saphyr::ser::Error> for SettingsError {
    fn from(e: serde_saphyr::ser::Error) -> Self {
        SettingsError::Serialize(Box::new(e))
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
pub(crate) fn tao_caps() -> HostBackendCapabilities {
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

#[cfg(test)]
pub(crate) fn gtk_caps() -> HostBackendCapabilities {
    HostBackendCapabilities {
        window: HostWindowCapabilities {
            remembers_window_size: false,
            supports_fullscreen_default: true,
            supports_scaling: true,
        },
        presentation: None,
    }
}

#[cfg(test)]
pub(crate) fn test_system_identity() -> SystemIdentity {
    SystemIdentity::new(SystemId::new("nes"), vec![4, 1, 0x11, 0x22, 0x33])
}

#[cfg(test)]
pub(crate) fn test_shared_defaults() -> DesktopSharedSettings {
    DesktopSharedSettings {
        systems: BTreeMap::from([(
            SystemId::new("nes"),
            SystemSettings::Nes(NesSettings::default()),
        )]),
        ..Default::default()
    }
}

#[cfg(test)]
pub(crate) fn test_local_defaults() -> HostBackendLocalSettings {
    HostBackendLocalSettings::default()
}

#[cfg(test)]
pub(crate) fn test_root(name: &str) -> PathBuf {
    env::current_dir()
        .unwrap()
        .join("target")
        .join("gui-runtime-settings")
        .join(name)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
}
