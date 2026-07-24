use std::{collections::BTreeSet, path::PathBuf};
#[cfg(test)]
use std::{collections::HashMap, env};

#[cfg(test)]
use nerust_core_traits::identity::SystemIdentity;
use nerust_gui_settings::{
    app_state::DesktopAppState, local::HostBackendLocalSettings, shared::DesktopSharedSettings,
};
#[cfg(test)]
use nerust_nes_settings::NesSettings;

#[cfg(test)]
use crate::test::DummySystemId;

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

/// Lossless Serde representation of the settings file.
///
/// Unlike `SettingsSnapshot`, this retains entries that the running build does
/// not know how to interpret, such as settings for an unavailable system.
#[derive(Debug, Clone)]
pub(super) struct SettingsDocument {
    value: serde_value::Value,
    known_system_keys: BTreeSet<serde_value::Value>,
    known_input_system_keys: BTreeSet<serde_value::Value>,
}

impl SettingsDocument {
    pub(super) fn from_snapshot(snapshot: &SettingsSnapshot) -> Result<Self, SettingsError> {
        let value = serde_value::to_value(snapshot)
            .map_err(|error| SettingsError::Serialize(Box::new(error)))?;
        Ok(Self::from_value_with_known_systems(value, snapshot))
    }

    pub(super) fn from_value_with_known_systems(
        value: serde_value::Value,
        known: &SettingsSnapshot,
    ) -> Self {
        let known_value =
            serde_value::to_value(known).expect("serializing validated settings should succeed");
        Self {
            value,
            known_system_keys: map_keys_at_path(&known_value, &["shared", "systems"]),
            known_input_system_keys: map_keys_at_path(
                &known_value,
                &["shared", "input", "systems"],
            ),
        }
    }

    pub(super) fn value(&self) -> &serde_value::Value {
        &self.value
    }

    pub(super) fn into_snapshot(self) -> Result<SettingsSnapshot, serde_value::DeserializerError> {
        self.value.deserialize_into()
    }

    pub(super) fn updated_with(&self, snapshot: &SettingsSnapshot) -> Result<Self, SettingsError> {
        let mut updated = Self::from_snapshot(snapshot)?;
        updated
            .known_system_keys
            .extend(self.known_system_keys.iter().cloned());
        updated
            .known_input_system_keys
            .extend(self.known_input_system_keys.iter().cloned());
        preserve_unknown_map_entries(
            &self.value,
            &mut updated.value,
            &["shared", "systems"],
            &updated.known_system_keys,
        );
        preserve_unknown_map_entries(
            &self.value,
            &mut updated.value,
            &["shared", "input", "systems"],
            &updated.known_input_system_keys,
        );
        Ok(updated)
    }
}

fn preserve_unknown_map_entries(
    source: &serde_value::Value,
    destination: &mut serde_value::Value,
    path: &[&str],
    known_keys: &BTreeSet<serde_value::Value>,
) {
    let Some(serde_value::Value::Map(source_map)) = value_at_path(source, path) else {
        return;
    };
    let Some(serde_value::Value::Map(destination_map)) = value_at_path_mut(destination, path)
    else {
        return;
    };
    for (key, value) in source_map {
        if known_keys.contains(key) {
            continue;
        }
        destination_map
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
}

fn map_keys_at_path(value: &serde_value::Value, path: &[&str]) -> BTreeSet<serde_value::Value> {
    match value_at_path(value, path) {
        Some(serde_value::Value::Map(map)) => map.keys().cloned().collect(),
        _ => BTreeSet::new(),
    }
}

fn value_at_path<'a>(
    value: &'a serde_value::Value,
    path: &[&str],
) -> Option<&'a serde_value::Value> {
    path.iter().try_fold(value, |current, segment| {
        let serde_value::Value::Map(map) = current else {
            return None;
        };
        map.get(&serde_value::Value::String((*segment).to_string()))
    })
}

fn value_at_path_mut<'a>(
    value: &'a mut serde_value::Value,
    path: &[&str],
) -> Option<&'a mut serde_value::Value> {
    let Some((segment, remaining)) = path.split_first() else {
        return Some(value);
    };
    let serde_value::Value::Map(map) = value else {
        return None;
    };
    value_at_path_mut(
        map.get_mut(&serde_value::Value::String((*segment).to_string()))?,
        remaining,
    )
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
    SystemIdentity::new(Box::new(DummySystemId), vec![4, 1, 0x11, 0x22, 0x33])
}

#[cfg(test)]
pub(crate) fn test_shared_defaults() -> DesktopSharedSettings {
    DesktopSharedSettings {
        systems: HashMap::from([(
            Box::new(DummySystemId) as Box<_>,
            Box::new(NesSettings::default()) as Box<dyn nerust_settings_traits::SystemSettings>,
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
