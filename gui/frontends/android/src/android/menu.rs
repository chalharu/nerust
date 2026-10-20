use jni::objects::{JObject, JString};
use nerust_input_traits::AbstractKey;

use super::{
    bridge,
    messages::{FrontendEvent, MenuAction},
    settings,
};

/// Kotlin→Rust event IDs. Despite the name this is the general event
/// confluence, not menu-only (dialog dismissals ride the same bridge).
const ACTION_ERROR_DIALOG_DISMISSED: &str = "error_dialog_dismissed";
const ACTION_EXIT: &str = "exit";
const ACTION_LOAD_STATE: &str = "load_state";
const ACTION_OPEN_ROM: &str = "open_rom";
const ACTION_OPEN_SETTINGS: &str = "open_settings";
const ACTION_ORIENTATION_CHANGED: &str = "orientation_changed";
const ACTION_RESET: &str = "reset";
const ACTION_SAVE_STATE: &str = "save_state";
const ACTION_TOGGLE_PAUSE: &str = "toggle_pause";
const ACTION_UNLOAD: &str = "unload";

/// Drain into a caller-owned buffer: priority lane (dialog acks) first,
/// then the normal lane. Idle ticks reuse capacity instead of allocating
/// a fresh vector per pass; no per-tick sort.
pub(crate) fn take_events_into(out: &mut Vec<FrontendEvent>) {
    bridge::take_events_into(out);
}

/// Versioned bridge event envelope. Kotlin sends either the legacy
/// string IDs (`"open_rom"`, `"controller:7:button1:1"`) or v1 JSON
/// (`{"v":1,"type":"menu","action":"open_rom"}`). Unknown versions and
/// unknown actions decode to `None` with a warn log: never a silent drop.
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
        return decode_controller_parts(
            &device_id.to_string(),
            key,
            if pressed { "1" } else { "0" },
        )
        .map(FrontendEvent::Menu);
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

fn decode_controller_parts(device_id: &str, key: &str, pressed: &str) -> Option<MenuAction> {
    let device_id = device_id.parse().ok()?;
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
    let pressed = match pressed {
        "1" => true,
        "0" => false,
        unknown => {
            log::warn!("ignoring Android controller press with bad state: {unknown}");
            return None;
        }
    };
    Some(MenuAction::ControllerInput {
        device_id,
        key,
        pressed,
    })
}

/// Legacy string IDs (`"open_rom"`, `"controller:7:button1:1"`).
/// Unknown IDs log a warn and decode to `None`: never a silent drop.
fn decode_menu_action(raw: &str) -> Option<MenuAction> {
    if let Some(payload) = raw.strip_prefix("controller:") {
        let mut parts = payload.split(':');
        let device_id = parts.next()?;
        let key = parts.next()?;
        let pressed = parts.next()?;
        if parts.next().is_some() {
            log::warn!("ignoring malformed Android controller event: {raw}");
            return None;
        }
        return decode_controller_parts(device_id, key, pressed);
    }
    match raw {
        ACTION_EXIT => Some(MenuAction::Exit),
        ACTION_LOAD_STATE => Some(MenuAction::LoadState),
        ACTION_OPEN_ROM => Some(MenuAction::OpenRom),
        ACTION_OPEN_SETTINGS => Some(MenuAction::OpenSettings),
        ACTION_ORIENTATION_CHANGED => Some(MenuAction::OrientationChanged),
        ACTION_RESET => Some(MenuAction::Reset),
        ACTION_SAVE_STATE => Some(MenuAction::SaveState),
        ACTION_TOGGLE_PAUSE => Some(MenuAction::TogglePause),
        ACTION_UNLOAD => Some(MenuAction::Unload),
        _ => {
            log::warn!("ignoring unknown Android menu action: {raw}");
            None
        }
    }
}

/// Decode one raw bridge string into the typed event. v1 JSON takes the
/// versioned path; anything else falls back to the legacy string IDs.
/// Dialog dismissal bypasses menu decoding; menu IDs never produce
/// dialog events.
fn decode_frontend_event(raw: &str) -> Option<FrontendEvent> {
    if raw == ACTION_ERROR_DIALOG_DISMISSED {
        return Some(FrontendEvent::ErrorDialogDismissed);
    }
    if raw.trim_start().starts_with('{') {
        return decode_json_event(raw);
    }
    decode_menu_action(raw).map(FrontendEvent::Menu)
}

fn publish_event(event: FrontendEvent) {
    publish_all(core::slice::from_ref(&event));
}

/// Batch publish: one wake per JNI batch, not per event. Bursts from
/// rapid menu taps share the wakeup instead of spamming the looper.
/// Dialog dismissals take the priority lane (see `bridge`).
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

/// Failure synthesis path: a dead JNI bridge cannot deliver a Kotlin
/// dismiss, so the gate, flag, and auto-pause are released here.
/// Single writer alongside `publish_event` (see module docs).
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
                    // The dialog is already up: the follow-up request
                    // no-ops on the bridge pending id (single
                    // DialogOwnership truth, no cached flag).
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

/// Dedicated error-dialog dismiss entry point. New APKs call this
/// directly (`onErrorDialogDismissed`); the legacy `"error_dialog_dismissed"`
/// menu-string path in [`decode_frontend_event`] stays as the decoder for
/// old APKs that still send the dismiss over `onMenuAction`.
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
        ACTION_ERROR_DIALOG_DISMISSED, ACTION_EXIT, ACTION_LOAD_STATE, ACTION_OPEN_ROM,
        ACTION_OPEN_SETTINGS, ACTION_ORIENTATION_CHANGED, ACTION_RESET, ACTION_SAVE_STATE,
        ACTION_TOGGLE_PAUSE, ACTION_UNLOAD, FrontendEvent, MenuAction, decode_frontend_event,
        decode_menu_action,
    };

    #[test]
    fn decode_menu_action_maps_known_ids() {
        assert_eq!(
            decode_menu_action(ACTION_OPEN_ROM),
            Some(MenuAction::OpenRom)
        );
        assert_eq!(
            decode_menu_action(ACTION_OPEN_SETTINGS),
            Some(MenuAction::OpenSettings)
        );
        assert_eq!(
            decode_menu_action(ACTION_TOGGLE_PAUSE),
            Some(MenuAction::TogglePause)
        );
        assert_eq!(
            decode_menu_action(ACTION_SAVE_STATE),
            Some(MenuAction::SaveState)
        );
        assert_eq!(
            decode_menu_action(ACTION_LOAD_STATE),
            Some(MenuAction::LoadState)
        );
        assert_eq!(decode_menu_action(ACTION_RESET), Some(MenuAction::Reset));
        assert_eq!(decode_menu_action(ACTION_EXIT), Some(MenuAction::Exit));
        assert_eq!(decode_menu_action(ACTION_UNLOAD), Some(MenuAction::Unload));
        assert_eq!(
            decode_menu_action(ACTION_ORIENTATION_CHANGED),
            Some(MenuAction::OrientationChanged)
        );
    }

    #[test]
    fn decode_menu_action_rejects_unknown_and_dialog_ids() {
        assert_eq!(decode_menu_action("mystery"), None);
        // Dialog dismissal is not a menu ID.
        assert_eq!(decode_menu_action(ACTION_ERROR_DIALOG_DISMISSED), None);
    }

    #[test]
    fn decode_menu_action_maps_controller_input() {
        assert_eq!(
            decode_menu_action("controller:7:button1:1"),
            Some(MenuAction::ControllerInput {
                device_id: 7,
                key: nerust_input_traits::AbstractKey::Button1,
                pressed: true,
            })
        );
        assert_eq!(decode_menu_action("controller:7:unknown:1"), None);
    }

    #[test]
    fn decode_frontend_event_separates_dialog_from_menu() {
        assert_eq!(
            decode_frontend_event(ACTION_ERROR_DIALOG_DISMISSED),
            Some(FrontendEvent::ErrorDialogDismissed)
        );
        assert_eq!(
            decode_frontend_event(ACTION_OPEN_ROM),
            Some(FrontendEvent::Menu(MenuAction::OpenRom))
        );
        assert_eq!(decode_frontend_event("mystery"), None);
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
    }
}
