use jni::objects::{JObject, JString};
use nerust_input_traits::AbstractKey;

use super::{
    bridge,
    messages::{FrontendEvent, MenuAction},
    settings,
};

/// Kotlin→Rust event IDs. This is the general event bridge;
/// menu taps and dialog acks share the envelope but decode separately.
const ACTION_EXIT: &str = "exit";
const ACTION_LOAD_STATE: &str = "load_state";
const ACTION_OPEN_ROM: &str = "open_rom";
const ACTION_OPEN_SETTINGS: &str = "open_settings";
const ACTION_ORIENTATION_CHANGED: &str = "orientation_changed";
const ACTION_RESET: &str = "reset";
const ACTION_SAVE_STATE: &str = "save_state";
const ACTION_TOGGLE_PAUSE: &str = "toggle_pause";
const ACTION_UNLOAD: &str = "unload";

/// Drain into a caller-owned buffer: priority lane first, then the normal lane.
/// Idle ticks reuse capacity instead of allocating per pass.
pub(crate) fn take_events_into(out: &mut Vec<FrontendEvent>) {
    bridge::take_events_into(out);
}

/// Versioned bridge event envelope: v1 JSON only; anything else decodes to `None`.
#[derive(Debug, serde::Deserialize)]
struct BridgeEventV1 {
    #[allow(dead_code)]
    v: u32,
    #[serde(rename = "type")]
    kind: BridgeEventKind,
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeEventKind {
    Menu {
        action: String,
        #[serde(default)]
        device_id: Option<i32>,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        pressed: Option<bool>,
    },
    ErrorDialogDismissed,
}

fn decode_json_event(raw: &str) -> Option<FrontendEvent> {
    let envelope: BridgeEventV1 = serde_json::from_str(raw).ok()?;
    if envelope.v != BRIDGE_EVENT_VERSION {
        log::warn!(
            "ignoring Android bridge event with unsupported version {}",
            envelope.v
        );
        return None;
    }
    match envelope.kind {
        BridgeEventKind::ErrorDialogDismissed => Some(FrontendEvent::ErrorDialogDismissed),
        BridgeEventKind::Menu {
            action,
            device_id,
            key,
            pressed,
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

/// Bridge event schema version. Bump when the Kotlin sender gains a new
/// shape; the decoder rejects anything else with a warn, never silently.
const BRIDGE_EVENT_VERSION: u32 = 1;

fn decode_controller_parts(device_id: i32, key: &str, pressed: bool) -> Option<MenuAction> {
    let key = match key {
        "button1" => AbstractKey::Button1,
        "button2" => AbstractKey::Button2,
        "button5" => AbstractKey::Button5,
        "button6" => AbstractKey::Button6,
        "start" => AbstractKey::Start,
        "select" => AbstractKey::Select,
        "up" => AbstractKey::DpadUp,
        "down" => AbstractKey::DpadDown,
        "left" => AbstractKey::DpadLeft,
        "right" => AbstractKey::DpadRight,
        unknown => {
            log::warn!("ignoring unknown Android controller key: {unknown}");
            return None;
        }
    };
    Some(MenuAction::ControllerInput {
        device_id,
        key,
        pressed,
    })
}

/// Legacy bare-string IDs are gone: Kotlin emits v1 JSON only.
/// The dedicated `onErrorDialogDismissed` native covers the dialog ack.

/// Decode one raw bridge string into the typed event.
/// Only v1 JSON decodes; anything else logs a warn, never a silent drop.
fn decode_frontend_event(raw: &str) -> Option<FrontendEvent> {
    if !raw.trim_start().starts_with('{') {
        log::warn!("ignoring non-JSON Android bridge event");
        return None;
    }
    decode_json_event(raw)
}

fn publish_event(event: FrontendEvent) {
    publish_all(core::slice::from_ref(&event));
}

/// Batch publish: one wake per JNI batch, not per event.
/// Dialog dismissals take the priority lane.
fn publish_all(events: &[FrontendEvent]) {
    if events.is_empty() {
        return;
    }
    if log::log_enabled!(log::Level::Debug) {
        for event in events {
            log::debug!("Android menu: published event: {:?}", event);
        }
    }
    bridge::publish_all(events);
}

/// Failure synthesis path: releases the gate, flag, and auto-pause.
/// Single writer alongside `publish_event`.
pub(crate) fn publish_dialog_dismissed() {
    publish_event(FrontendEvent::ErrorDialogDismissed);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onMenuAction(
    mut env: jni::EnvUnowned<'_>,
    activity: JObject<'_>,
    action: JString<'_>,
) {
    match bridge::with_callback_env(
        &mut env,
        |env| -> jni::errors::Result<Option<FrontendEvent>> {
            if action.is_null() {
                Ok(None)
            } else {
                let action_str = action.try_to_string(env)?;
                let decoded = decode_frontend_event(&action_str);

                // Dialog-showing actions are handled synchronously since we
                // already have env/activity on the Java main thread.
                if let Some(FrontendEvent::Menu(MenuAction::OpenSettings)) = decoded {
                    match settings::show_settings_dialog_sync(env, &activity) {
                        Ok(_) => {}
                        Err(error) => {
                            log::error!("sync settings dialog failed: {error}");
                        }
                    }
                    // The dialog is already up: the follow-up request no-ops.
                    // The bridge pending id is the single truth.
                    return Ok(Some(FrontendEvent::Menu(MenuAction::OpenSettings)));
                }

                Ok(decoded)
            }
        },
    ) {
        jni::Outcome::Ok(Some(event)) => publish_event(event),
        jni::Outcome::Ok(None) => {}
        jni::Outcome::Err(_) => {}
        jni::Outcome::Panic(_) => {
            log::error!("Android menu action callback panicked");
        }
    }
}

/// Dedicated error-dialog dismiss entry point.
/// Every Kotlin sender calls this; no menu-string fallback remains.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onErrorDialogDismissed(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
) {
    match bridge::with_callback_env(&mut env, |_env| -> jni::errors::Result<FrontendEvent> {
        Ok(FrontendEvent::ErrorDialogDismissed)
    }) {
        jni::Outcome::Ok(event) => publish_event(event),
        jni::Outcome::Err(_) => {}
        jni::Outcome::Panic(_) => {
            log::error!("Android error-dismiss callback panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ACTION_EXIT, ACTION_LOAD_STATE, ACTION_OPEN_ROM, ACTION_OPEN_SETTINGS,
        ACTION_ORIENTATION_CHANGED, ACTION_RESET, ACTION_SAVE_STATE, ACTION_TOGGLE_PAUSE,
        ACTION_UNLOAD, BRIDGE_EVENT_VERSION, FrontendEvent, MenuAction, decode_frontend_event,
    };

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
        // Legacy bare strings no longer decode: Kotlin emits v1 JSON only.
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
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../../bridge_events.v1.json"))
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
}
