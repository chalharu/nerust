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

/// Drain into a caller-owned buffer: idle ticks reuse capacity
/// instead of allocating a fresh vector per pass.
pub(crate) fn take_events_into(out: &mut Vec<FrontendEvent>) {
    bridge::with_state(|state| out.extend(state.frontend_events.drain(..)));
}

fn decode_menu_action(raw: &str) -> Option<MenuAction> {
    if let Some(payload) = raw.strip_prefix("controller:") {
        let mut parts = payload.split(':');
        let device_id = parts.next()?.parse().ok()?;
        let key = parts.next()?;
        let pressed = parts.next()?;
        if parts.next().is_some() {
            return None;
        }
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
            _ => return None,
        };
        let pressed = match pressed {
            "1" => true,
            "0" => false,
            _ => return None,
        };
        return Some(MenuAction::ControllerInput {
            device_id,
            key,
            pressed,
        });
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
        _ => None,
    }
}

/// Decode one raw bridge string into the typed event. Dialog dismissal
/// bypasses menu decoding; menu IDs never produce dialog events.
fn decode_frontend_event(raw: &str) -> Option<FrontendEvent> {
    if raw == ACTION_ERROR_DIALOG_DISMISSED {
        return Some(FrontendEvent::ErrorDialogDismissed);
    }
    decode_menu_action(raw).map(FrontendEvent::Menu)
}

fn publish_event(event: FrontendEvent) {
    log::debug!("Android menu: published event: {:?}", event);
    bridge::with_state(|state| state.frontend_events.push_back(event));
    bridge::wake();
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
    match env
        .with_env(|env| -> jni::errors::Result<Option<FrontendEvent>> {
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
                    // Record the open for the frontend cache even though the
                    // dialog is already up: the follow-up request no-ops.
                    return Ok(Some(FrontendEvent::Menu(MenuAction::OpenSettings)));
                }

                Ok(decoded)
            }
        })
        .into_outcome()
    {
        jni::Outcome::Ok(Some(event)) => publish_event(event),
        jni::Outcome::Ok(None) => {}
        jni::Outcome::Err(error) => {
            log::error!("failed to decode Android menu action: {error:?}");
        }
        jni::Outcome::Panic(_) => {
            log::error!("Android menu action callback panicked");
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
}
