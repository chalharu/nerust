use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use nerust_core_traits::identity::SystemId;

pub const DESKTOP_APP_STATE_SCHEMA_VERSION: u32 = 2;

pub type ControllerAssignmentsBySystem = HashMap<Box<dyn SystemId>, Vec<(String, Option<String>)>>;

/// Global player-port device membership: port id ("p1"/"p2") → member ids.
/// Member ids: "kbd:builtin" and "kbd:<descriptor>" (keyboards, shareable
/// across ports), "pad:<guid>" (gamepads, exclusive to one port — backend
/// pending, but the schema and the exclusivity rule already cover them).
/// Touch is a global setting, never a port member.
pub type PortDeviceSets = BTreeMap<String, Vec<String>>;

/// Player port ids. Ports are global (not per machine): every system's
/// player-N slot fans out from the same port set.
pub const PORT_PLAYER_1: &str = "p1";
pub const PORT_PLAYER_2: &str = "p2";

/// Application state not synced per ROM: window geometry, controller
/// assignments, and global port device sets.

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
    pub controller_assignments: ControllerAssignmentsBySystem,
    /// Global port device sets: port id → member ids ("kbd:*" shareable,
    /// "pad:*" exclusive). Default: built-in keyboard drives player 1.
    pub port_devices: PortDeviceSets,
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
            controller_assignments: HashMap::new(),
            port_devices: BTreeMap::from([(
                PORT_PLAYER_1.to_string(),
                vec!["kbd:builtin".to_string()],
            )]),
        }
    }
}
