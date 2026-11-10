//! Settings bridge: versioned payload push plus synchronous command JNI.
//! Screen truth (single flight plus draft snapshot) lives here; views and
//! validation live in `settings_views`. Kotlin renders sections and echoes
//! one command per edit; every JNI entry builds a throwaway view model.

use jni::objects::{JObject, JString, JValue};
use winit::platform::android::activity::AndroidApp;

use super::bridge::{self, SettingsDialogResult};
use super::settings_views::{self, SETTINGS_SCHEMA_VERSION};
use crate::settings_flight::SettingsRequestId;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_settings_core::model::local::ScreenOrientation;

/// Versioned JNI screen result: dismissal only. The accepted draft travels
/// through the bridge vault, never through Kotlin.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsResultPayload {
    schema_version: u32,
    request_id: SettingsRequestId,
    #[serde(default)]
    dismissed: bool,
}

/// Open the screen: stash the draft, then push the projected sections.
/// Kotlin echoes commands against this draft until save or dismiss.
pub(super) fn request_show_settings_dialog(
    app: &AndroidApp,
    snapshot: &SettingsSnapshot,
) -> Result<bool, String> {
    // Project before taking the flight: a projection failure must not wedge it.
    let payload = bridge::with_settings(|state| {
        settings_views::open_payload(snapshot, &state.keyboards)
    })
    .map_err(|error| error.to_string())??;
    let payload = bridge::with_settings(|state| {
        let request_id = state.flight.begin()?;
        state.draft = Some(snapshot.clone());
        state.capture = None;
        Some((request_id, payload))
    });
    let Some((request_id, payload)) = payload else {
        return Ok(false);
    };
    let tagged = serde_json::json!({
        "schemaVersion": SETTINGS_SCHEMA_VERSION,
        "requestId": request_id.0,
        "payload": serde_json::from_str::<serde_json::Value>(&payload)
            .map_err(|error| error.to_string())?,
    })
    .to_string();
    bridge::post_to_activity(
        app,
        "show Android settings screen",
        move |env, activity| show_settings_with_env_inner(env, &activity, &tagged),
        Some(Box::new(move || {
            // The push failed: drop the stashed draft with the dead flight.
            bridge::with_settings(|state| {
                state.draft = None;
                state.capture = None;
            });
            publish_result(request_id, SettingsDialogResult::Dismissed)
        })),
    );
    Ok(true)
}

/// Publish the current orientation without opening the screen.
pub(super) fn apply_screen_orientation(app: &AndroidApp, orientation: ScreenOrientation) {
    // Route every raw `run_on_java_main_thread` through the bridge helper
    // so JNI failures share one log label.
    bridge::post_to_activity(
        app,
        "apply Android screen orientation",
        move |env, activity| {
            let value = match orientation {
                ScreenOrientation::Auto => 0,
                ScreenOrientation::Portrait => 1,
                ScreenOrientation::Landscape => 2,
            };
            bridge::call_bridge_method(
                env,
                &activity,
                &bridge::methods::APPLY_SCREEN_ORIENTATION,
                &[JValue::Int(value)],
            )?;
            Ok(())
        },
        None,
    );
}

fn show_settings_with_env_inner(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
    payload: &str,
) -> Result<(), jni::errors::Error> {
    let payload = env.new_string(payload)?;

    bridge::call_bridge_method(
        env,
        activity,
        &bridge::methods::SHOW_SETTINGS_DIALOG,
        &[JValue::Object(payload.as_ref())],
    )?;
    Ok(())
}

fn publish_result(request_id: SettingsRequestId, result: SettingsDialogResult) {
    // Draft ownership moves at the callback: save takes it, dismiss drops it.
    // A stale accept must never touch the live draft, so this only gates flight.
    let accepted = bridge::with_settings(|state| state.flight.accept(request_id, result));
    if !accepted {
        log::warn!("ignoring stale Android settings result for request {request_id:?}");
        return;
    }
    bridge::wake();
}

/// Id-conditional variant: a stale callback must not clear the live flight.
fn resolve_pending_as_dismissed_for(request_id: SettingsRequestId) {
    publish_result(request_id, SettingsDialogResult::Dismissed);
}

pub(super) fn take_result() -> Option<SettingsDialogResult> {
    bridge::with_settings(|state| state.flight.take())
}

pub(super) fn is_dialog_open() -> bool {
    bridge::with_settings(|state| state.flight.is_open())
}

pub(super) fn reset_transient() {
    bridge::with_settings(|state| {
        // Accepted user input survives suspension until the event loop consumes it.
        state.flight.suspend();
    });
}

// JNI callback for the settings screen result: dismissal drops the draft,
// save takes it into the flight for the event thread to validate and apply.
// The standalone SettingsActivity hosts the screen.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_SettingsActivity_onSettingsDialogResult(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    result: JString<'_>,
) {
    on_settings_dialog_result(&mut env, result);
}

fn on_settings_dialog_result(env: &mut jni::EnvUnowned<'_>, result: JString<'_>) {
    match bridge::with_callback_env(&mut *env, |env| -> jni::errors::Result<Option<String>> {
        if result.is_null() {
            Ok(None)
        } else {
            Ok(Some(result.try_to_string(env)?))
        }
    }) {
        jni::Outcome::Ok(Some(raw)) => match serde_json::from_str::<SettingsResultPayload>(&raw) {
            Ok(payload) if payload.schema_version == SETTINGS_SCHEMA_VERSION => {
                let result = bridge::with_settings(|state| {
                    if payload.dismissed {
                        state.draft = None;
                        state.capture = None;
                        Some(SettingsDialogResult::Dismissed)
                    } else {
                        state.capture = None;
                        state
                            .draft
                            .take()
                            .map(SettingsDialogResult::Applied)
                            .or(Some(SettingsDialogResult::Dismissed))
                    }
                });
                if let Some(result) = result {
                    publish_result(payload.request_id, result);
                }
            }
            Ok(payload) => {
                log::error!(
                    "unsupported Android settings result schema {}",
                    payload.schema_version
                );
                // The payload parsed, so its id is trustworthy:
                // clear only its own flight, never a newer rerequest.
                resolve_pending_as_dismissed_for(payload.request_id);
            }
            Err(error) => {
                log::error!("failed to parse Android settings result: {error}");
                // No trustworthy correlation id: never dismiss a newer flight.
            }
        },
        jni::Outcome::Ok(None) => {
            log::warn!("ignoring uncorrelated Android settings dismissal");
        }
        jni::Outcome::Err(_) => {}
        jni::Outcome::Panic(_) => {
            log::error!("Android settings dialog callback panicked");
        }
    }
}

/// Hardware key routing for capture: consumes the key when a capture is
/// pending and returns the refreshed payload (or an error envelope).
/// Non-capture keys pass through untouched (null payload marker).
/// The standalone SettingsActivity hosts capture, so only it binds this.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_SettingsActivity_nativeHardwareKey(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    keycode: jni::sys::jint,
    pressed: jni::sys::jboolean,
) -> jni::sys::jstring {
    hardware_key(&mut env, keycode, pressed)
}

fn hardware_key(
    env: &mut jni::EnvUnowned<'_>,
    keycode: jni::sys::jint,
    pressed: jni::sys::jboolean,
) -> jni::sys::jstring {
    let parsed = bridge::with_callback_env(&mut *env, |_env| -> jni::errors::Result<(i32, bool)> {
        Ok((keycode, pressed))
    });
    let out = match parsed {
        jni::Outcome::Ok((keycode, pressed)) => bridge::with_settings(|state| {
            let Some(draft) = state.draft.as_mut() else {
                return serde_json::json!({"ok": false, "error": "no settings draft"}).to_string();
            };
            if state.capture.is_none() {
                return serde_json::json!({"ok": true, "consumed": false}).to_string();
            }
            match settings_views::apply_hardware_key(
                keycode,
                pressed,
                draft,
                &mut state.capture,
                &state.keyboards,
            )
            {
                Ok((payload, consumed)) => serde_json::json!({"ok": true, "consumed": consumed, "payload": serde_json::from_str::<serde_json::Value>(&payload).unwrap_or(serde_json::Value::Null)}).to_string(),
                Err(error) => serde_json::json!({"ok": false, "error": error}).to_string(),
            }
        }),
        _ => serde_json::json!({"ok": false, "error": "jni decode failed"}).to_string(),
    };
    match bridge::with_callback_env(&mut *env, |env| -> jni::errors::Result<jni::sys::jstring> {
        Ok(env.new_string(out)?.into_raw())
    }) {
        jni::Outcome::Ok(ptr) => ptr,
        _ => std::ptr::null_mut(),
    }
}
/// Synchronous settings command: one edit in, refreshed payload
/// (or `{"ok":false,"error"}`) out. The draft lives in the bridge vault;
/// the view model lives only for this call, so any calling thread is safe.
/// The standalone SettingsActivity hosts the screen, so only it binds this.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_SettingsActivity_nativeSettingsCommand(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    command: JString<'_>,
) -> jni::sys::jstring {
    settings_command(&mut env, command)
}

fn settings_command(env: &mut jni::EnvUnowned<'_>, command: JString<'_>) -> jni::sys::jstring {
    let parsed = bridge::with_callback_env(&mut *env, |env| -> jni::errors::Result<String> {
        command.try_to_string(env)
    });
    let out = match parsed {
        jni::Outcome::Ok(command_json) => bridge::with_settings(|state| {
            let Some(draft) = state.draft.as_mut() else {
                return serde_json::json!({"ok": false, "error": "no settings draft"}).to_string();
            };
            match settings_views::apply_command_to(
                draft,
                &command_json,
                &mut state.capture,
                &mut state.keyboards,
            ) {
                Ok(sections) => serde_json::json!({"ok": true, "payload": serde_json::from_str::<serde_json::Value>(&sections).unwrap_or(serde_json::Value::Null)}).to_string(),
                Err(error) => serde_json::json!({"ok": false, "error": error}).to_string(),
            }
        }),
        _ => serde_json::json!({"ok": false, "error": "jni decode failed"}).to_string(),
    };
    match bridge::with_callback_env(&mut *env, |env| -> jni::errors::Result<jni::sys::jstring> {
        Ok(env.new_string(out)?.into_raw())
    }) {
        jni::Outcome::Ok(ptr) => ptr,
        _ => std::ptr::null_mut(),
    }
}
