//! Typed Kotlin bridge codec: v1 JSON strings into frontend events.
//! JNI glue stays in `android::menu`; this module is host-testable.

use nerust_input_traits::AbstractKey;

/// Menu button presses from the Kotlin overflow menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuAction {
    ControllerInput {
        device_id: i32,
        key: AbstractKey,
        pressed: bool,
    },
    Exit,
    LoadState,
    OpenRom,
    OpenSettings,
    OrientationChanged,
    Reset,
    SaveState,
    TogglePause,
    Unload,
}

/// Single Kotlin→Rust event channel: menu presses and dialog dismiss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontendEvent {
    Menu(MenuAction),
    ErrorDialogDismissed,
}

/// Bridge event schema version: decoder rejects anything else with a warn.
pub(crate) const BRIDGE_EVENT_VERSION: u32 = 1;

pub(crate) const ACTION_EXIT: &str = "exit";
pub(crate) const ACTION_LOAD_STATE: &str = "load_state";
pub(crate) const ACTION_OPEN_ROM: &str = "open_rom";
pub(crate) const ACTION_OPEN_SETTINGS: &str = "open_settings";
pub(crate) const ACTION_ORIENTATION_CHANGED: &str = "orientation_changed";
pub(crate) const ACTION_RESET: &str = "reset";
pub(crate) const ACTION_SAVE_STATE: &str = "save_state";
pub(crate) const ACTION_TOGGLE_PAUSE: &str = "toggle_pause";
pub(crate) const ACTION_UNLOAD: &str = "unload";

/// Canonical controller key table: the single source for the decoder, the JSON schema enum, and the
/// Kotlin `KEY_*` constants. A new key fails loudly in all three pins, never as a silent drop.
pub(crate) const CONTROLLER_KEY_TABLE: &[(&str, AbstractKey)] = &[
    ("button1", AbstractKey::Button1),
    ("button2", AbstractKey::Button2),
    ("button5", AbstractKey::Button5),
    ("button6", AbstractKey::Button6),
    ("start", AbstractKey::Start),
    ("select", AbstractKey::Select),
    ("up", AbstractKey::DpadUp),
    ("down", AbstractKey::DpadDown),
    ("left", AbstractKey::DpadLeft),
    ("right", AbstractKey::DpadRight),
];

/// Canonical settings section ids in payload order; Rust owns the
/// id→label mapping and pushes the label per section.
pub(crate) const SETTINGS_SECTION_IDS: &[&str] = &[
    "audio",
    "video",
    "controls",
    "storage",
    "notifications",
    "system.nes",
    "system.gbc",
    "system.gba",
];

/// Canonical section id→label table, owned by Rust and pushed per
/// section; Kotlin renders the pushed label and keeps its map as fallback.
pub(crate) fn section_label(id: &str) -> String {
    match id {
        "audio" => "Audio".to_string(),
        "video" => "Video".to_string(),
        "controls" => "Controls".to_string(),
        "storage" => "Storage".to_string(),
        "notifications" => "Notifications".to_string(),
        "general" => "General".to_string(),
        "system.nes" => "Nintendo Entertainment System".to_string(),
        "system.gbc" => "Game Boy Color".to_string(),
        "system.gba" => "Game Boy Advance".to_string(),
        other => {
            let base = other.rsplit('.').next().unwrap_or(other).replace('_', " ");
            let mut chars = base.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        }
    }
}

/// Versioned bridge event envelope: v1 JSON only; anything else is `None`.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeEventV1 {
    Menu {
        #[allow(dead_code)]
        v: u32,
        action: String,
        #[serde(default)]
        device_id: Option<i32>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        pressed: Option<bool>,
    },
    ErrorDialogDismissed {
        #[allow(dead_code)]
        v: u32,
    },
}

fn decode_json_event(raw: &str) -> Option<FrontendEvent> {
    let envelope: BridgeEventV1 = serde_json::from_str(raw).ok()?;
    let version = match &envelope {
        BridgeEventV1::Menu { v, .. } => *v,
        BridgeEventV1::ErrorDialogDismissed { v } => *v,
    };
    if version != BRIDGE_EVENT_VERSION {
        log::warn!("ignoring Android bridge event with unsupported version {version}");
        return None;
    }
    match envelope {
        BridgeEventV1::ErrorDialogDismissed { .. } => Some(FrontendEvent::ErrorDialogDismissed),
        BridgeEventV1::Menu {
            action,
            device_id,
            key,
            pressed,
            ..
        } => decode_json_menu_action(&action, device_id, key.as_deref(), pressed),
    }
}

fn decode_json_menu_action(
    action: &str,
    device_id: Option<i32>,
    key: Option<&str>,
    pressed: Option<bool>,
) -> Option<FrontendEvent> {
    if action == "controller" {
        let (device_id, key, pressed) = match (device_id, key, pressed) {
            (Some(device_id), Some(key), Some(pressed)) => (device_id, key, pressed),
            _ => {
                log::warn!("ignoring controller bridge event with missing fields");
                return None;
            }
        };
        return decode_controller_parts(device_id, key, pressed).map(FrontendEvent::Menu);
    }
    match action {
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
    let Some(key) = CONTROLLER_KEY_TABLE
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, key)| *key)
    else {
        log::warn!("ignoring unknown Android controller key: {key}");
        return None;
    };
    Some(MenuAction::ControllerInput {
        device_id,
        key,
        pressed,
    })
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
        ACTION_EXIT, ACTION_LOAD_STATE, ACTION_OPEN_ROM, ACTION_OPEN_SETTINGS,
        ACTION_ORIENTATION_CHANGED, ACTION_RESET, ACTION_SAVE_STATE, ACTION_TOGGLE_PAUSE,
        ACTION_UNLOAD, BRIDGE_EVENT_VERSION, FrontendEvent, MenuAction, decode_frontend_event,
    };

    #[test]
    fn decodes_every_menu_action_from_kotlin_envelope() {
        for (action, expected) in [
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
    }

    #[test]
    fn decode_json_event_maps_versioned_menu() {
        assert_eq!(
            decode_frontend_event(r#"{"v":1,"type":"menu","action":"open_rom"}"#),
            Some(FrontendEvent::Menu(MenuAction::OpenRom))
        );
        assert_eq!(
            decode_frontend_event(
                r#"{"v":1,"type":"menu","action":"controller","device_id":7,"key":"button1","pressed":true}"#
            ),
            Some(FrontendEvent::Menu(MenuAction::ControllerInput {
                device_id: 7,
                key: nerust_input_traits::AbstractKey::Button1,
                pressed: true,
            }))
        );
        assert_eq!(
            decode_frontend_event(r#"{"v":1,"type":"error_dialog_dismissed"}"#),
            Some(FrontendEvent::ErrorDialogDismissed)
        );
    }

    #[test]
    fn decode_json_event_rejects_bad_version_and_unknown() {
        assert_eq!(
            decode_frontend_event(r#"{"v":2,"type":"menu","action":"open_rom"}"#),
            None
        );
        assert_eq!(
            decode_frontend_event(r#"{"v":1,"type":"menu","action":"mystery"}"#),
            None
        );
        assert_eq!(
            decode_frontend_event(r#"{"v":1,"type":"menu","action":"controller","device_id":7}"#),
            None
        );
        for legacy in [
            ACTION_OPEN_ROM,
            ACTION_OPEN_SETTINGS,
            ACTION_TOGGLE_PAUSE,
            ACTION_SAVE_STATE,
            ACTION_LOAD_STATE,
            ACTION_RESET,
            ACTION_EXIT,
            ACTION_UNLOAD,
            ACTION_ORIENTATION_CHANGED,
            "controller:7:button1:1",
            "error_dialog_dismissed",
        ] {
            assert_eq!(decode_frontend_event(legacy), None);
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
        let actions = doc["definitions"]["menuEvent"]["properties"]["action"]["enum"]
            .as_array()
            .expect("action enum must list every menu action");
        let names: Vec<&str> = actions
            .iter()
            .map(|value| value.as_str().expect("action enum entries are strings"))
            .collect();
        for action in [
            ACTION_EXIT,
            ACTION_LOAD_STATE,
            ACTION_OPEN_ROM,
            ACTION_OPEN_SETTINGS,
            ACTION_ORIENTATION_CHANGED,
            ACTION_RESET,
            ACTION_SAVE_STATE,
            ACTION_TOGGLE_PAUSE,
            ACTION_UNLOAD,
            "controller",
        ] {
            assert!(names.contains(&action), "schema is missing action {action}");
        }
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
        for (name, key) in CONTROLLER_KEY_TABLE {
            assert!(
                schema_keys.contains(name),
                "schema is missing controller key {name}"
            );
            let raw = format!(
                r#"{{"v":1,"type":"menu","action":"controller","device_id":3,"key":"{name}","pressed":true}}"#
            );
            assert_eq!(
                decode_frontend_event(&raw),
                Some(FrontendEvent::Menu(MenuAction::ControllerInput {
                    device_id: 3,
                    key: *key,
                    pressed: true,
                })),
                "key {name} must round-trip"
            );
        }
        assert_eq!(
            decode_frontend_event(
                r#"{"v":1,"type":"menu","action":"controller","device_id":3,"key":"mystery","pressed":true}"#
            ),
            None
        );
    }

    /// Section taxonomy: ids stay unique and every id owns a label.
    /// Kotlin renders the pushed label; its map is a fallback only.
    #[test]
    fn section_ids_own_labels() {
        use super::{SETTINGS_SECTION_IDS, section_label};
        let mut seen = std::collections::BTreeSet::new();
        for id in SETTINGS_SECTION_IDS {
            assert!(seen.insert(*id), "duplicate section id {id}");
            assert!(!section_label(id).is_empty(), "section {id} needs a label");
        }
        assert_eq!(section_label("audio"), "Audio");
        assert_eq!(section_label("notifications"), "Notifications");
        assert_eq!(section_label("system.gbc"), "Game Boy Color");
        assert_eq!(section_label("system.unknown"), "Unknown");
        assert_eq!(section_label("foo_bar"), "Foo bar");
    }
}
