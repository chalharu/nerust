use std::{collections::BTreeMap, fmt::Debug, path::PathBuf};

use nerust_core_traits::identity::SystemId;
use nerust_settings_traits::SystemSettings as SystemSettingsTrait;

use crate::{input::InputSettings, language::AppLanguage};

pub const DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DesktopSharedSettings {
    pub schema_version: u32,
    pub general: GeneralSettings,
    pub persistence: PersistenceSettings,
    pub input: InputSettings,
    pub systems: BTreeMap<SystemId, Box<dyn SystemSettingsTrait>>,
}

impl Default for DesktopSharedSettings {
    fn default() -> Self {
        Self {
            schema_version: DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION,
            general: GeneralSettings::default(),
            persistence: PersistenceSettings::default(),
            input: InputSettings::default(),
            systems: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    pub language: AppLanguage,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PersistenceSettings {
    pub storage_policy: StoragePolicy,
    pub storage_directory: Option<PathBuf>,
}

impl Default for PersistenceSettings {
    fn default() -> Self {
        Self {
            storage_policy: StoragePolicy::Sidecar,
            storage_directory: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoragePolicy {
    #[default]
    Sidecar,
    AppSharedData,
    CustomDirectory,
}
