//! Typed Kotlin bridge codec: v1 JSON strings into frontend events.
//! JNI glue stays in `android::menu`; this module is host-testable.

use nerust_input_traits::{GamepadAxis, StandardButton};

/// Menu button presses from the Kotlin overflow menu.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MenuAction {
    ControllerInput {
        device_id: i32,
        button: StandardButton,
        pressed: bool,
    },
    AnalogInput {
        device_id: i32,
        axis: GamepadAxis,
        value: f32,
    },
    DeviceAdded {
        device_id: i32,
        descriptor: String,
    },
    DeviceRemoved {
        device_id: i32,
    },
    CreateSlot,
    DeleteSlot(u64),
    Exit,
    LoadState,
    OpenRom,
    OpenSettings,
    OrientationChanged,
    Reset,
    SaveState,
    SelectSlot(u64),
    TogglePause,
    TouchGate {
        respond: bool,
    },
    Unload,
}

/// Single Kotlin→Rust event channel: menu presses and dialog dismiss.
/// `Eq` and `Copy` are absent: analog values ride `f32`, and device
/// descriptors ride `String`; compared by tests exactly but never hashed
/// or keyed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FrontendEvent {
    Menu(MenuAction),
    ErrorDialogDismissed(crate::error_dialog_controller::DialogRequestId),
}

use crate::bridge_contract::{
    ACTION_ANALOG, ACTION_CONTROLLER, ACTION_CREATE_SLOT, ACTION_DELETE_SLOT, ACTION_DEVICE_ADDED,
    ACTION_DEVICE_REMOVED, ACTION_EXIT, ACTION_LOAD_STATE, ACTION_OPEN_ROM, ACTION_OPEN_SETTINGS,
    ACTION_ORIENTATION_CHANGED, ACTION_RESET, ACTION_SAVE_STATE, ACTION_SELECT_SLOT,
    ACTION_TOGGLE_PAUSE, ACTION_TOUCH_GATE, ACTION_UNLOAD, BRIDGE_EVENT_VERSION,
    validate_bridge_event,
};

/// Typed controller-key mapping, checked against the shared JSON schema.
/// Single source: the decoder reads this table, no parallel match.
/// Names are bridge ids (stable for compat); positions resolve in Rust so
/// one OS keycode set serves every vendor layout.
pub(crate) const CONTROLLER_KEY_TABLE: &[(&str, StandardButton)] = &[
    ("button1", StandardButton::South),
    ("button2", StandardButton::East),
    ("button3", StandardButton::West),
    ("button4", StandardButton::North),
    ("button5", StandardButton::LeftShoulder),
    ("button6", StandardButton::RightShoulder),
    ("start", StandardButton::Start),
    ("select", StandardButton::Back),
    ("guide", StandardButton::Guide),
    ("up", StandardButton::DpadUp),
    ("down", StandardButton::DpadDown),
    ("left", StandardButton::DpadLeft),
    ("right", StandardButton::DpadRight),
    ("l3", StandardButton::LeftStick),
    ("r3", StandardButton::RightStick),
];

/// One drawer-submenu row: slot id plus its save instant. Plain data so
/// the builder stays host-testable without persistence types; the JNI
/// query maps summaries into this.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SlotListEntry {
    pub id: u64,
    pub saved_at: std::time::SystemTime,
}

/// Slot list for the drawer submenu: saved ids with epoch-millis stamps
/// plus the active id. Pure builder (host-testable); the JNI query in
/// `android::menu` only feeds it the vault mirror. Title/time/active copy
/// renders in Kotlin (app locale) from this; Rust ships ids and instants.
/// A pre-epoch stamp rides as null so Kotlin shows its unknown label
/// instead of guessing.
pub(crate) fn build_slot_list_json(slots: &[SlotListEntry], active: Option<u64>) -> String {
    let rows: Vec<serde_json::Value> = slots
        .iter()
        .map(|slot| {
            let saved_at_ms = slot
                .saved_at
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|age| age.as_millis() as i64);
            serde_json::json!({"id": slot.id, "saved_at_ms": saved_at_ms})
        })
        .collect();
    serde_json::json!({"slots": rows, "active": active}).to_string()
}

/// Versioned bridge event envelope: v1 JSON only; anything else is `None`.
/// Explicit optional fields (never flattened): `verify-contract.py`
/// parses this enum textually as a schema-parity gate.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeEventV1 {
    Menu {
        v: u32,
        action: String,
        #[serde(default)]
        device_id: Option<i32>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        pressed: Option<bool>,
        #[serde(default)]
        axis: Option<String>,
        #[serde(default)]
        value: Option<f32>,
        #[serde(default)]
        descriptor: Option<String>,
        #[serde(default)]
        slot_id: Option<i64>,
    },
    ErrorDialogDismissed {
        v: u32,
        request_id: crate::error_dialog_controller::DialogRequestId,
    },
}

fn decode_json_event(raw: &str) -> Option<FrontendEvent> {
    let mut value: serde_json::Value = serde_json::from_str(raw).ok()?;
    // Generated-contract validation stays: it normalizes Kotlin integers
    // (float-encoded ids) and rejects out-of-range axes before dispatch.
    validate_bridge_event(&mut value)?;
    let envelope: BridgeEventV1 = serde_json::from_value(value).ok()?;
    let version = match &envelope {
        BridgeEventV1::Menu { v, .. } => *v,
        BridgeEventV1::ErrorDialogDismissed { v, .. } => *v,
    };
    if version != BRIDGE_EVENT_VERSION {
        log::warn!("ignoring Android bridge event with unsupported version {version}");
        return None;
    }
    match envelope {
        BridgeEventV1::ErrorDialogDismissed { request_id, .. } => {
            Some(FrontendEvent::ErrorDialogDismissed(request_id))
        }
        BridgeEventV1::Menu {
            action,
            device_id,
            key,
            pressed,
            axis,
            value,
            descriptor,
            slot_id,
            ..
        } => decode_json_menu_action(
            &action,
            MenuParts {
                device_id,
                key: key.as_deref(),
                pressed,
                axis: axis.as_deref(),
                value,
                descriptor: descriptor.as_deref(),
                slot_id,
            },
        ),
    }
}

/// Slot id for select/delete: a missing or negative id fails loud
/// instead of guessing a slot.
fn decode_slot_id(slot_id: Option<i64>) -> Option<u64> {
    match slot_id.and_then(|id| u64::try_from(id).ok()) {
        Some(slot_id) => Some(slot_id),
        None => {
            log::warn!("ignoring slot bridge event with missing or negative slot_id");
            None
        }
    }
}

/// Optional envelope fields for one menu action. Bundled so the decoder
/// stays under the argument cap as gated fields grow.
struct MenuParts<'a> {
    device_id: Option<i32>,
    key: Option<&'a str>,
    pressed: Option<bool>,
    axis: Option<&'a str>,
    value: Option<f32>,
    descriptor: Option<&'a str>,
    slot_id: Option<i64>,
}

fn decode_json_menu_action(action: &str, parts: MenuParts) -> Option<FrontendEvent> {
    let MenuParts {
        device_id,
        key,
        pressed,
        axis,
        value,
        descriptor,
        slot_id,
    } = parts;
    let key = key.as_deref();
    let axis = axis.as_deref();
    let descriptor = descriptor.as_deref();
    if action == ACTION_CONTROLLER {
        let (device_id, key, pressed) = match (device_id, key, pressed) {
            (Some(device_id), Some(key), Some(pressed)) => (device_id, key, pressed),
            _ => {
                log::warn!("ignoring controller bridge event with missing fields");
                return None;
            }
        };
        return decode_controller_parts(device_id, key, pressed).map(FrontendEvent::Menu);
    }
    if action == ACTION_ANALOG {
        let (device_id, axis, value) = match (device_id, axis, value) {
            (Some(device_id), Some(axis), Some(value)) => (device_id, axis, value),
            _ => {
                log::warn!("ignoring analog bridge event with missing fields");
                return None;
            }
        };
        return decode_analog_parts(device_id, axis, value).map(FrontendEvent::Menu);
    }
    if action == ACTION_DEVICE_ADDED {
        let (device_id, descriptor) = match (device_id, descriptor) {
            (Some(device_id), Some(descriptor)) if !descriptor.is_empty() => {
                (device_id, descriptor)
            }
            _ => {
                log::warn!("ignoring device bridge event with missing fields");
                return None;
            }
        };
        return Some(FrontendEvent::Menu(MenuAction::DeviceAdded {
            device_id,
            descriptor: descriptor.to_string(),
        }));
    }
    if action == ACTION_DEVICE_REMOVED {
        let Some(device_id) = device_id else {
            log::warn!("ignoring device bridge event with missing fields");
            return None;
        };
        return Some(FrontendEvent::Menu(MenuAction::DeviceRemoved { device_id }));
    }
    if action == ACTION_TOUCH_GATE {
        return match pressed {
            Some(respond) => Some(FrontendEvent::Menu(MenuAction::TouchGate { respond })),
            None => {
                log::warn!("ignoring touch gate bridge event with missing pressed");
                None
            }
        };
    }
    match action {
        ACTION_CREATE_SLOT => Some(FrontendEvent::Menu(MenuAction::CreateSlot)),
        ACTION_DELETE_SLOT => decode_slot_id(slot_id)
            .map(|slot_id| FrontendEvent::Menu(MenuAction::DeleteSlot(slot_id))),
        ACTION_SELECT_SLOT => decode_slot_id(slot_id)
            .map(|slot_id| FrontendEvent::Menu(MenuAction::SelectSlot(slot_id))),
        ACTION_EXIT => Some(FrontendEvent::Menu(MenuAction::Exit)),
        ACTION_LOAD_STATE => Some(FrontendEvent::Menu(MenuAction::LoadState)),
        ACTION_OPEN_ROM => Some(FrontendEvent::Menu(MenuAction::OpenRom)),
        ACTION_OPEN_SETTINGS => Some(FrontendEvent::Menu(MenuAction::OpenSettings)),
        ACTION_ORIENTATION_CHANGED => Some(FrontendEvent::Menu(MenuAction::OrientationChanged)),
        ACTION_RESET => Some(FrontendEvent::Menu(MenuAction::Reset)),
        ACTION_SAVE_STATE => Some(FrontendEvent::Menu(MenuAction::SaveState)),
        ACTION_TOGGLE_PAUSE => Some(FrontendEvent::Menu(MenuAction::TogglePause)),
        ACTION_UNLOAD => Some(FrontendEvent::Menu(MenuAction::Unload)),
        unknown => {
            log::warn!("ignoring unknown Android menu action: {unknown}");
            None
        }
    }
}

fn decode_controller_parts(device_id: i32, key: &str, pressed: bool) -> Option<MenuAction> {
    let Some(button) = controller_button(key) else {
        log::warn!("ignoring unknown Android controller key: {key}");
        return None;
    };
    Some(MenuAction::ControllerInput {
        device_id,
        button,
        pressed,
    })
}

fn decode_analog_parts(device_id: i32, axis: &str, value: f32) -> Option<MenuAction> {
    let Some(axis) = controller_axis(axis) else {
        log::warn!("ignoring unknown Android controller axis: {axis}");
        return None;
    };
    if !(-1.0..=1.0).contains(&value) {
        log::warn!("ignoring out-of-range Android axis value: {value}");
        return None;
    }
    Some(MenuAction::AnalogInput {
        device_id,
        axis,
        value,
    })
}

/// Typed controller-axis mapping: SDL-style names to normalized axes.
/// Single source: the decoder reads this table, no parallel match.
pub(crate) const CONTROLLER_AXIS_TABLE: &[(&str, GamepadAxis)] = &[
    ("left_x", GamepadAxis::LeftStickX),
    ("left_y", GamepadAxis::LeftStickY),
    ("right_x", GamepadAxis::RightStickX),
    ("right_y", GamepadAxis::RightStickY),
    ("trigger_l", GamepadAxis::LeftTrigger),
    ("trigger_r", GamepadAxis::RightTrigger),
];

/// Single-table lookups; the schema test iterates the tables above.
fn controller_button(key: &str) -> Option<StandardButton> {
    CONTROLLER_KEY_TABLE
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, button)| *button)
}

fn controller_axis(axis: &str) -> Option<GamepadAxis> {
    CONTROLLER_AXIS_TABLE
        .iter()
        .find(|(name, _)| *name == axis)
        .map(|(_, value)| *value)
}

/// Decode one raw bridge string: only v1 JSON decodes, never a silent drop.
pub(crate) fn decode_frontend_event(raw: &str) -> Option<FrontendEvent> {
    if !raw.trim_start().starts_with('{') {
        log::warn!("ignoring non-JSON Android bridge event");
        return None;
    }
    decode_json_event(raw)
}

#[cfg(test)]
mod tests {
    use super::{
        ACTION_ANALOG, ACTION_CREATE_SLOT, ACTION_DELETE_SLOT, ACTION_DEVICE_ADDED,
        ACTION_DEVICE_REMOVED, ACTION_EXIT, ACTION_LOAD_STATE, ACTION_OPEN_ROM,
        ACTION_OPEN_SETTINGS, ACTION_ORIENTATION_CHANGED, ACTION_RESET, ACTION_SAVE_STATE,
        ACTION_SELECT_SLOT, ACTION_TOGGLE_PAUSE, ACTION_TOUCH_GATE, ACTION_UNLOAD, FrontendEvent,
        MenuAction, decode_frontend_event,
    };
    use crate::bridge_contract::BRIDGE_EVENT_VERSION;

    #[test]
    fn decodes_every_menu_action_from_kotlin_envelope() {
        for (action, expected) in [
            (ACTION_CREATE_SLOT, MenuAction::CreateSlot),
            (ACTION_EXIT, MenuAction::Exit),
            (ACTION_LOAD_STATE, MenuAction::LoadState),
            (ACTION_OPEN_ROM, MenuAction::OpenRom),
            (ACTION_OPEN_SETTINGS, MenuAction::OpenSettings),
            (ACTION_ORIENTATION_CHANGED, MenuAction::OrientationChanged),
            (ACTION_RESET, MenuAction::Reset),
            (ACTION_SAVE_STATE, MenuAction::SaveState),
            (ACTION_TOGGLE_PAUSE, MenuAction::TogglePause),
            (ACTION_UNLOAD, MenuAction::Unload),
        ] {
            let raw =
                format!(r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{action}"}}"#);
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(expected)),
                "action {action}"
            );
        }
        // The touch gate carries its respond flag in `pressed`; a missing
        // flag fails loud instead of guessing a default.
        for respond in [true, false] {
            let raw = format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{ACTION_TOUCH_GATE}","pressed":{respond}}}"#
            );
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(MenuAction::TouchGate { respond })),
                "touch gate {respond}"
            );
        }
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"touch_gate"}}"#
            )),
            None,
            "touch gate without pressed never defaults"
        );
    }

    #[test]
    fn decode_json_event_maps_versioned_menu() {
        // open_rom is covered by decodes_every_menu_action_from_kotlin_envelope;
        // this pins the gated controller shape and the second envelope type.
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"controller","device_id":7,"key":"button1","pressed":true}}"#
            )),
            Some(FrontendEvent::Menu(MenuAction::ControllerInput {
                device_id: 7,
                button: nerust_input_traits::StandardButton::South,
                pressed: true,
            }))
        );
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"error_dialog_dismissed","request_id":7}}"#
            )),
            Some(FrontendEvent::ErrorDialogDismissed(
                crate::error_dialog_controller::DialogRequestId(7)
            ))
        );
    }

    #[test]
    fn slot_list_json_ships_ids_instants_and_active() {
        use std::time::{Duration, UNIX_EPOCH};

        use super::{SlotListEntry, build_slot_list_json};

        fn entry(id: u64, secs: u64) -> SlotListEntry {
            SlotListEntry {
                id,
                saved_at: UNIX_EPOCH + Duration::from_secs(secs),
            }
        }

        let raw = build_slot_list_json(&[entry(2, 1_700_000_001), entry(1, 1_700_000_000)], None);
        let value: serde_json::Value = serde_json::from_str(&raw).expect("slot list is JSON");
        assert_eq!(value["slots"][0]["id"], 2);
        assert_eq!(value["slots"][0]["saved_at_ms"], 1_700_000_001_000_i64);
        assert!(value["active"].is_null());
        let raw = build_slot_list_json(&[entry(1, 1_700_000_000)], Some(1));
        let value: serde_json::Value = serde_json::from_str(&raw).expect("slot list is JSON");
        assert_eq!(value["active"], 1);
    }

    #[test]
    fn slot_actions_round_trip_and_reject_bad_ids() {
        for (action, expected) in [
            (ACTION_SELECT_SLOT, MenuAction::SelectSlot(3)),
            (ACTION_DELETE_SLOT, MenuAction::DeleteSlot(3)),
        ] {
            let raw = format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{action}","slot_id":3}}"#
            );
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(expected)),
                "action {action}"
            );
        }
        for raw in [
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{ACTION_SELECT_SLOT}"}}"#
            ),
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"{ACTION_DELETE_SLOT}","slot_id":-1}}"#
            ),
        ] {
            assert_eq!(
                decode_frontend_event(&raw),
                None,
                "slot action without a usable id never defaults"
            );
        }
    }

    #[test]
    fn decode_json_event_rejects_bad_version_and_unknown() {
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{},"type":"menu","action":"open_rom"}}"#,
                BRIDGE_EVENT_VERSION + 1
            )),
            None
        );
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"mystery"}}"#
            )),
            None
        );
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"controller","device_id":7}}"#
            )),
            None
        );
        // Bare action names and colon-separated payloads never reach the
        // typed dispatch: the JSON prefix guard rejects them first.
        for legacy in [
            ACTION_OPEN_ROM,
            "controller:7:button1:1",
            "error_dialog_dismissed",
        ] {
            assert_eq!(decode_frontend_event(legacy), None);
        }
        // Unknown actions inside a valid envelope reach the typed dispatch
        // and fail there (not at the prefix guard).
        for unknown in [
            ACTION_OPEN_SETTINGS,
            ACTION_TOGGLE_PAUSE,
            ACTION_SAVE_STATE,
            ACTION_LOAD_STATE,
            ACTION_RESET,
            ACTION_EXIT,
            ACTION_UNLOAD,
            ACTION_ORIENTATION_CHANGED,
        ] {
            assert_eq!(
                decode_frontend_event(&format!(
                    r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"legacy_{unknown}"}}"#
                )),
                None
            );
        }
    }

    /// Contract pin: the JSON schema and the Rust decoder share one version
    /// and one action table; a one-side drift fails loudly.
    #[test]
    fn bridge_schema_matches_versioned_contract() {
        let doc: serde_json::Value = serde_json::from_str(include_str!("../bridge_events.v1.json"))
            .expect("bridge schema must parse");
        assert_eq!(
            doc["definitions"]["menuEvent"]["properties"]["v"]["const"],
            BRIDGE_EVENT_VERSION
        );
        assert_eq!(
            doc["definitions"]["errorDialogDismissedEvent"]["properties"]["v"]["const"],
            BRIDGE_EVENT_VERSION
        );
        assert_eq!(
            doc["x-settings-schema-version"],
            crate::bridge_contract::SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(
            doc["x-saf-chunk-bytes"],
            crate::bridge_contract::SAF_CHUNK_BYTES
        );
        let actions = doc["definitions"]["menuEvent"]["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum must list every menu action");
        let names: Vec<&str> = actions
            .iter()
            .map(|value| value.as_str().expect("action enum entries are strings"))
            .collect();
        assert_eq!(
            names.len(),
            17,
            "new actions must also be handled by the decoder"
        );
        for action in [
            ACTION_ANALOG,
            ACTION_CREATE_SLOT,
            ACTION_DELETE_SLOT,
            ACTION_DEVICE_ADDED,
            ACTION_DEVICE_REMOVED,
            ACTION_EXIT,
            ACTION_LOAD_STATE,
            ACTION_OPEN_ROM,
            ACTION_OPEN_SETTINGS,
            ACTION_ORIENTATION_CHANGED,
            ACTION_RESET,
            ACTION_SAVE_STATE,
            ACTION_SELECT_SLOT,
            ACTION_TOGGLE_PAUSE,
            ACTION_TOUCH_GATE,
            ACTION_UNLOAD,
            "controller",
        ] {
            assert!(names.contains(&action), "schema is missing action {action}");
        }
    }

    /// Device connect/disconnect round-trips with descriptors; empty
    /// descriptors and missing ids fail loud instead of registering junk.
    #[test]
    fn device_events_carry_descriptors() {
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_added","device_id":7,"descriptor":"93c16c27e6378d0"}}"#
            )),
            Some(FrontendEvent::Menu(MenuAction::DeviceAdded {
                device_id: 7,
                descriptor: "93c16c27e6378d0".to_string(),
            }))
        );
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_removed","device_id":7}}"#
            )),
            Some(FrontendEvent::Menu(MenuAction::DeviceRemoved {
                device_id: 7
            }))
        );
        for raw in [
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_added","device_id":7}}"#
            ),
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_added","device_id":7,"descriptor":""}}"#
            ),
            format!(r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"device_removed"}}"#),
        ] {
            assert_eq!(decode_frontend_event(&raw), None);
        }
    }

    /// Axis parity: the decoder table and the schema axis enum list
    /// exactly the same axes, and every one round-trips with a float.
    #[test]
    fn controller_axis_table_matches_schema() {
        use super::{CONTROLLER_AXIS_TABLE, decode_frontend_event};
        use nerust_input_traits::GamepadAxis;
        let doc: serde_json::Value = serde_json::from_str(include_str!("../bridge_events.v1.json"))
            .expect("bridge schema must parse");
        let axes = doc["definitions"]["menuEvent"]["properties"]["axis"]["enum"]
            .as_array()
            .expect("axis enum must list every controller axis");
        let schema_axes: Vec<&str> = axes
            .iter()
            .map(|value| value.as_str().expect("axis enum entries are strings"))
            .collect();
        assert_eq!(schema_axes.len(), CONTROLLER_AXIS_TABLE.len());
        for (name, axis) in CONTROLLER_AXIS_TABLE {
            assert!(
                schema_axes.contains(name),
                "schema is missing controller axis {name}"
            );
            let raw = format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"analog","device_id":3,"axis":"{name}","value":0.5}}"#
            );
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(MenuAction::AnalogInput {
                    device_id: 3,
                    axis: *axis,
                    value: 0.5,
                })),
                "axis {name} must round-trip"
            );
        }
        for raw in [
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"analog","device_id":3,"axis":"mystery","value":0.5}}"#
            ),
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"analog","device_id":3,"axis":"left_x","value":1.5}}"#
            ),
            format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"analog","device_id":3,"axis":"left_x"}}"#
            ),
        ] {
            assert_eq!(decode_frontend_event(&raw), None);
        }
        assert_eq!(
            GamepadAxis::LeftStickX,
            CONTROLLER_AXIS_TABLE[0].1,
            "axis order stays SDL-canonical"
        );
    }
    /// Key parity: the decoder table and the schema key enum list
    /// exactly the same keys; Kotlin `KEY_*` pins the same set in CI.
    #[test]
    fn controller_key_table_matches_schema() {
        use super::{CONTROLLER_KEY_TABLE, decode_frontend_event};
        let doc: serde_json::Value = serde_json::from_str(include_str!("../bridge_events.v1.json"))
            .expect("bridge schema must parse");
        let keys = doc["definitions"]["menuEvent"]["properties"]["key"]["enum"]
            .as_array()
            .expect("key enum must list every controller key");
        let schema_keys: Vec<&str> = keys
            .iter()
            .map(|value| value.as_str().expect("key enum entries are strings"))
            .collect();
        assert_eq!(schema_keys.len(), CONTROLLER_KEY_TABLE.len());
        for (name, button) in CONTROLLER_KEY_TABLE {
            assert!(
                schema_keys.contains(name),
                "schema is missing controller key {name}"
            );
            let raw = format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"controller","device_id":3,"key":"{name}","pressed":true}}"#
            );
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(MenuAction::ControllerInput {
                    device_id: 3,
                    button: *button,
                    pressed: true,
                })),
                "key {name} must round-trip"
            );
        }
        assert_eq!(
            decode_frontend_event(&format!(
                r#"{{"v":{BRIDGE_EVENT_VERSION},"type":"menu","action":"controller","device_id":3,"key":"mystery","pressed":true}}"#
            )),
            None
        );
    }
}
