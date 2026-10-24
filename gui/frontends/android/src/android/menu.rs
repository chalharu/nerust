use super::bridge;
use crate::bridge_events::{FrontendEvent, decode_frontend_event};
use crate::error_dialog_controller::DialogRequestId;
use jni::objects::{JObject, JString};

/// Failure synthesis path: releases the gate, flag, and auto-pause.
/// Single writer into the priority lane.
pub(super) fn publish_dialog_dismissed(id: DialogRequestId) {
    bridge::publish_all(core::slice::from_ref(&FrontendEvent::ErrorDialogDismissed(
        id,
    )));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onMenuAction(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
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
    request_id: jni::sys::jlong,
) {
    match bridge::with_callback_env(&mut env, |_env| -> jni::errors::Result<FrontendEvent> {
        Ok(FrontendEvent::ErrorDialogDismissed(DialogRequestId(
            request_id as u64,
        )))
    }) {
        jni::Outcome::Ok(event) => bridge::publish_all(core::slice::from_ref(&event)),
        jni::Outcome::Err(_) => {}
        jni::Outcome::Panic(_) => {
            log::error!("Android error-dismiss callback panicked");
        }
    }
}
