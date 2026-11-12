use super::bridge;
use crate::bridge_events::{
    FrontendEvent, SlotListEntry, build_slot_list_json, decode_frontend_event,
};
use crate::error_dialog_controller::DialogRequestId;
use jni::objects::{JObject, JString};

/// Failure synthesis path: releases the gate, flag, and auto-pause.
/// Single writer into the priority lane.
pub(super) fn publish_dialog_dismissed(id: DialogRequestId) {
    bridge::publish_all(core::slice::from_ref(&FrontendEvent::ErrorDialogDismissed(
        id,
    )));
}

/// Shared menu-action publish: MainActivity and SlotActivity decode the
/// same wire envelope and feed the same lane.
fn publish_menu_action(
    env: &jni::Env<'_>,
    action: JString<'_>,
) -> jni::errors::Result<Option<FrontendEvent>> {
    if action.is_null() {
        Ok(None)
    } else {
        let action_str = action.try_to_string(env)?;
        Ok(decode_frontend_event(&action_str))
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onMenuAction(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    action: JString<'_>,
) {
    match bridge::with_callback_env(
        &mut env,
        |env| -> jni::errors::Result<Option<FrontendEvent>> { publish_menu_action(env, action) },
    ) {
        jni::Outcome::Ok(Some(event)) => bridge::publish_all(core::slice::from_ref(&event)),
        jni::Outcome::Ok(None) => {}
        jni::Outcome::Err(_) => {}
        jni::Outcome::Panic(_) => {
            log::error!("Android menu action callback panicked");
        }
    }
}

/// Synchronous pause query for the drawer: vault truth published by
/// `AndroidFrontend::exec`, never the live session (this runs on the Java
/// main thread). The drawer reads it once per open for Pause/Resume.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_nativeIsEmulationPaused(
    _env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
) -> jni::sys::jboolean {
    bridge::with_settings(|state| state.emulation_paused) as jni::sys::jboolean
}

/// Synchronous slot-list query for the drawer submenu (MainActivity reads
/// it when the submenu opens and after every slot op it sends).
/// The standalone slot screen reads the same vault through its own class.
fn query_slot_list_json() -> String {
    bridge::with_settings(|state| {
        build_slot_list_json(
            &state
                .slots
                .iter()
                .map(|slot| SlotListEntry {
                    id: slot.slot_id,
                    saved_at: slot.saved_at,
                })
                .collect::<Vec<_>>(),
            state.active_slot,
        )
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_nativeSlotList(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
) -> jni::sys::jstring {
    let out = query_slot_list_json();
    match bridge::with_callback_env(&mut env, |env| -> jni::errors::Result<jni::sys::jstring> {
        Ok(env.new_string(out)?.into_raw())
    }) {
        jni::Outcome::Ok(ptr) => ptr,
        _ => std::ptr::null_mut(),
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

/// Standalone slot screen query: same vault, own activity class.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_SlotActivity_nativeQuerySlotList(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
) -> jni::sys::jstring {
    let out = query_slot_list_json();
    match bridge::with_callback_env(&mut env, |env| -> jni::errors::Result<jni::sys::jstring> {
        Ok(env.new_string(out)?.into_raw())
    }) {
        jni::Outcome::Ok(ptr) => ptr,
        _ => std::ptr::null_mut(),
    }
}

/// Standalone slot screen action: same envelope, same lane as the drawer.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_SlotActivity_sendSlotAction(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    action: JString<'_>,
) {
    match bridge::with_callback_env(
        &mut env,
        |env| -> jni::errors::Result<Option<FrontendEvent>> { publish_menu_action(env, action) },
    ) {
        jni::Outcome::Ok(Some(event)) => bridge::publish_all(core::slice::from_ref(&event)),
        jni::Outcome::Ok(None) => {}
        jni::Outcome::Err(_) => {}
        jni::Outcome::Panic(_) => {
            log::error!("Android slot action callback panicked");
        }
    }
}
