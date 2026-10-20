use super::bridge;
use crate::bridge_events::{FrontendEvent, MenuAction, decode_frontend_event};
use jni::objects::{JObject, JString};

/// Failure synthesis path: releases the gate, flag, and auto-pause.
/// Single writer into the priority lane.
pub(super) fn publish_dialog_dismissed() {
    bridge::publish_all(core::slice::from_ref(&FrontendEvent::ErrorDialogDismissed));
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
                    match super::settings_bridge::show_settings_dialog_sync(env, &activity) {
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
        jni::Outcome::Ok(Some(event)) => bridge::publish_all(core::slice::from_ref(&event)),
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
        jni::Outcome::Ok(event) => bridge::publish_all(core::slice::from_ref(&event)),
        jni::Outcome::Err(_) => {}
        jni::Outcome::Panic(_) => {
            log::error!("Android error-dismiss callback panicked");
        }
    }
}
