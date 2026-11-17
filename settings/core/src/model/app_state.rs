use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use nerust_core_traits::identity::SystemId;

pub const DESKTOP_APP_STATE_SCHEMA_VERSION: u32 = 3;

pub type ControllerAssignmentsBySystem = HashMap<Box<dyn SystemId>, Vec<(String, Option<String>)>>;

/// Global player-port device membership: port id ("p1"/"p2") → member ids.
/// Member ids: "kbd:builtin" and "kbd:<descriptor>" (keyboards, shareable
/// across ports), "pad:<guid>" (gamepads, exclusive to one port — backend
/// pending, but the schema and the exclusivity rule already cover them).
/// Touch is a global setting, never a port member.
pub type PortDeviceSets = BTreeMap<String, Vec<String>>;

/// Member-id prefixes for global port device sets. Keyboards ("kbd:")
/// share ports; gamepads ("pad:") occupy exactly one port at a time.
/// Touch is a global setting, never a port member. Canonical home is
/// `nerust_input_traits` (the HID layer); re-exported here so persisted
/// defaults and the schema docs read from one source.
pub use nerust_input_traits::{
    BUILTIN_KEYBOARD_MEMBER, GAMEPAD_MEMBER_PREFIX, KEYBOARD_MEMBER_PREFIX,
};

/// Player port ids. Ports are global (not per machine): every system's
/// player-N slot fans out from the same port set.
pub const PORT_PLAYER_1: &str = "p1";
pub const PORT_PLAYER_2: &str = "p2";

/// Port membership for fresh installs and for files predating the field:
/// the built-in keyboard drives every player port (keyboards are
/// shareable, and the pre-routing behavior drove all ports unconditionally).
/// One source so deserialized defaults and `Default` can never drift apart.
fn default_port_devices() -> PortDeviceSets {
    BTreeMap::from([
        (
            PORT_PLAYER_1.to_string(),
            vec![BUILTIN_KEYBOARD_MEMBER.to_string()],
        ),
        (
            PORT_PLAYER_2.to_string(),
            vec![BUILTIN_KEYBOARD_MEMBER.to_string()],
        ),
    ])
}

/// The pre-routing default (schema 2): built-in keyboard on player 1 only.
/// Files carrying exactly this set never had routing enforced, so upgrading
/// them preserves their effective behavior. Anything else is a user
/// configuration and is left untouched.
fn legacy_p1_only_port_devices() -> PortDeviceSets {
    BTreeMap::from([(
        PORT_PLAYER_1.to_string(),
        vec![BUILTIN_KEYBOARD_MEMBER.to_string()],
    )])
}

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
    /// "pad:*" exclusive). Default: built-in keyboard drives every player port.
    #[serde(default = "default_port_devices")]
    pub port_devices: PortDeviceSets,
}

impl DesktopAppState {
    /// Schema 2→3 migration: routing enforcement is new, and every file
    /// written before it behaves as if the keyboard drove all ports
    /// (nothing consumed the sets). Files carrying exactly the old default
    /// adopt the new default; user-configured sets are left untouched.
    /// Idempotent: migrated sets never match the old default again.
    pub fn migrate_port_devices(&mut self) {
        if self.port_devices == legacy_p1_only_port_devices() {
            self.port_devices = default_port_devices();
            self.schema_version = DESKTOP_APP_STATE_SCHEMA_VERSION;
        }
    }
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
            port_devices: default_port_devices(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_predating_port_devices_get_the_same_default() {
        // Old files lack the field; fresh defaults must agree with them.
        let migrated: DesktopAppState =
            serde_json::from_str(r#"{"schema_version": 2}"#).expect("old file parses");
        assert_eq!(
            migrated.port_devices,
            DesktopAppState::default().port_devices
        );
        for port in [PORT_PLAYER_1, PORT_PLAYER_2] {
            assert_eq!(
                migrated.port_devices.get(port).map(Vec::as_slice),
                Some(["kbd:builtin".to_string()].as_slice()),
                "built-in keyboard drives {port} by default",
            );
        }
    }

    #[test]
    fn migrate_port_devices_upgrades_only_the_legacy_default() {
        use serde_json::json;
        // Exactly the schema-2 default: adopt the new both-ports default.
        let mut legacy = DesktopAppState {
            schema_version: 2,
            port_devices: BTreeMap::from([(
                PORT_PLAYER_1.to_string(),
                vec![BUILTIN_KEYBOARD_MEMBER.to_string()],
            )]),
            ..DesktopAppState::default()
        };
        legacy.migrate_port_devices();
        assert_eq!(legacy.port_devices, DesktopAppState::default().port_devices);
        assert_eq!(legacy.schema_version, DESKTOP_APP_STATE_SCHEMA_VERSION);
        // Idempotent: a second pass changes nothing.
        let snapshot = legacy.clone();
        legacy.migrate_port_devices();
        assert_eq!(legacy, snapshot);
        // User-configured sets (extra member, missing builtin) are untouched.
        for devices in [
            serde_json::json!({"p1": ["kbd:builtin", "kbd:usb-1"]}),
            serde_json::json!({"p1": [], "p2": ["kbd:builtin"]}),
            serde_json::json!({"p1": ["pad:aa"]}),
        ] {
            let mut state = DesktopAppState {
                schema_version: 2,
                port_devices: serde_json::from_value(devices.clone()).expect("test set parses"),
                ..DesktopAppState::default()
            };
            state.migrate_port_devices();
            assert_eq!(
                state.port_devices,
                serde_json::from_value::<PortDeviceSets>(devices).expect("test set parses"),
                "custom sets survive migration",
            );
            assert_eq!(state.schema_version, 2, "custom sets keep their version");
        }
    }
}
