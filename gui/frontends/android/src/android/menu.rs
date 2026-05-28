use jni::objects::{JObject, JString};
use std::mem;
use std::sync::Mutex;
use winit::platform::android::activity::{AndroidApp, AndroidAppWaker};

use super::{library, settings};

const ACTION_LOAD_STATE: &str = "load_state";
const ACTION_OPEN_LIBRARY: &str = "open_library";
const ACTION_OPEN_SETTINGS: &str = "open_settings";
const ACTION_RESET: &str = "reset";
const ACTION_SAVE_STATE: &str = "save_state";
const ACTION_TOGGLE_PAUSE: &str = "toggle_pause";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuAction {
    LoadState,
    OpenLibrary,
    OpenSettings,
    Reset,
    SaveState,
    TogglePause,
}

static MENU_ACTIONS: Mutex<Vec<MenuAction>> = Mutex::new(Vec::new());
static MENU_WAKER: Mutex<Option<AndroidAppWaker>> = Mutex::new(None);

pub(crate) fn bind_app(app: &AndroidApp) {
    *MENU_WAKER.lock().expect("menu waker mutex poisoned") = Some(app.create_waker());
    MENU_ACTIONS
        .lock()
        .expect("menu actions mutex poisoned")
        .clear();
}

pub(crate) fn reset() {
    MENU_ACTIONS
        .lock()
        .expect("menu actions mutex poisoned")
        .clear();
}

pub(crate) fn take_actions() -> Vec<MenuAction> {
    mem::take(&mut *MENU_ACTIONS.lock().expect("menu actions mutex poisoned"))
}

fn decode_action(raw: &str) -> Option<MenuAction> {
    match raw {
        ACTION_LOAD_STATE => Some(MenuAction::LoadState),
        ACTION_OPEN_LIBRARY => Some(MenuAction::OpenLibrary),
        ACTION_OPEN_SETTINGS => Some(MenuAction::OpenSettings),
        ACTION_RESET => Some(MenuAction::Reset),
        ACTION_SAVE_STATE => Some(MenuAction::SaveState),
        ACTION_TOGGLE_PAUSE => Some(MenuAction::TogglePause),
        _ => None,
    }
}

fn publish_action(action: MenuAction) {
    MENU_ACTIONS
        .lock()
        .expect("menu actions mutex poisoned")
        .push(action);
    wake_main_thread();
}

fn wake_main_thread() {
    if let Some(waker) = MENU_WAKER
        .lock()
        .expect("menu waker mutex poisoned")
        .clone()
    {
        waker.wake();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onMenuAction(
    mut env: jni::EnvUnowned<'_>,
    activity: JObject<'_>,
    action: JString<'_>,
) {
    match env
        .with_env(|env| -> jni::errors::Result<Option<MenuAction>> {
            if action.is_null() {
                Ok(None)
            } else {
                let action_str = action.try_to_string(env)?;
                let decoded = decode_action(&action_str);

                // Dialog-showing actions are handled synchronously since we
                // already have env/activity on the Java main thread.
                match decoded {
                    Some(MenuAction::OpenLibrary) => {
                        match library::show_library_dialog_sync(env, &activity) {
                            Ok(_) => {}
                            Err(error) => {
                                log::error!("sync library dialog failed: {error}");
                            }
                        }
                        return Ok(None);
                    }
                    Some(MenuAction::OpenSettings) => {
                        match settings::show_settings_dialog_sync(env, &activity) {
                            Ok(_) => {}
                            Err(error) => {
                                log::error!("sync settings dialog failed: {error}");
                            }
                        }
                        return Ok(None);
                    }
                    _ => {}
                }

                Ok(decoded)
            }
        })
        .into_outcome()
    {
        jni::Outcome::Ok(Some(action)) => publish_action(action),
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
    use super::{ACTION_LOAD_STATE, ACTION_SAVE_STATE, ACTION_TOGGLE_PAUSE, MenuAction};
    use super::{ACTION_OPEN_LIBRARY, ACTION_OPEN_SETTINGS, ACTION_RESET, decode_action};

    #[test]
    fn decode_action_maps_known_ids() {
        assert_eq!(
            decode_action(ACTION_OPEN_LIBRARY),
            Some(MenuAction::OpenLibrary)
        );
        assert_eq!(
            decode_action(ACTION_OPEN_SETTINGS),
            Some(MenuAction::OpenSettings)
        );
        assert_eq!(
            decode_action(ACTION_TOGGLE_PAUSE),
            Some(MenuAction::TogglePause)
        );
        assert_eq!(
            decode_action(ACTION_SAVE_STATE),
            Some(MenuAction::SaveState)
        );
        assert_eq!(
            decode_action(ACTION_LOAD_STATE),
            Some(MenuAction::LoadState)
        );
        assert_eq!(decode_action(ACTION_RESET), Some(MenuAction::Reset));
    }

    #[test]
    fn decode_action_rejects_unknown_ids() {
        assert_eq!(decode_action("mystery"), None);
    }
}
