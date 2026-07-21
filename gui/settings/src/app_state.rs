use std::{collections::BTreeMap, path::PathBuf};

pub const DESKTOP_APP_STATE_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RememberedWindowSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DesktopAppState {
    pub schema_version: u32,
    pub last_successful_rom_directory: Option<PathBuf>,
    pub window_sizes: BTreeMap<String, RememberedWindowSize>,
    /// Per-system controller assignments: system_id → [(slot_id, controller_id or None)]
    pub controller_assignments: BTreeMap<String, Vec<(String, Option<String>)>>,
}

impl DesktopAppState {
    pub fn window_size(&self, host_backend: &str) -> Option<RememberedWindowSize> {
        self.window_sizes.get(host_backend).copied()
    }

    pub fn set_window_size(&mut self, host_backend: impl Into<String>, size: RememberedWindowSize) {
        self.window_sizes.insert(host_backend.into(), size);
    }
}

impl Default for DesktopAppState {
    fn default() -> Self {
        Self {
            schema_version: DESKTOP_APP_STATE_SCHEMA_VERSION,
            last_successful_rom_directory: None,
            window_sizes: BTreeMap::new(),
            controller_assignments: BTreeMap::new(),
        }
    }
}
