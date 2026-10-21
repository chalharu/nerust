//! Settings bridge: dialog flight, cache slot, and JNI handoff.
//! Model and codec stay pure; this module owns the single-flight state.

use std::{collections::BTreeMap, sync::Arc};

use jni::objects::{JObject, JString, JValue};
use winit::platform::android::activity::AndroidApp;

use super::bridge::{self, SettingsDialogResult};
use super::settings_model::{AndroidSettings, SETTINGS_SCHEMA_VERSION};
use super::settings_schema::SettingDef;
use crate::settings_flight::SettingsRequestId;
use nerust_settings_core::model::local::ScreenOrientation;

/// Cached [`AndroidSettings::setting_defs`], keyed by the complete model.
/// Shared by handle, so cache hits never re-clone the whole table.
pub(super) fn cached_setting_defs(current: &AndroidSettings) -> Arc<Vec<SettingDef>> {
    bridge::with_settings(|state| {
        AndroidSettings::cached_setting_defs_locked(&mut state.defs_cache, current)
    })
}

/// Versioned JNI dialog result: schema plus keyed values or dismissal.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsResultPayload {
    schema_version: u32,
    request_id: SettingsRequestId,
    #[serde(default)]
    dismissed: bool,
    #[serde(default)]
    values: BTreeMap<String, usize>,
}

/// Update cached settings for the event-loop-owned asynchronous display request.
pub(super) fn update_cached_settings(current: &AndroidSettings) {
    bridge::with_settings(|state| {
        state.cache = Some(current.clone());
        state.defs_cache = None;
    });
}

/// Thread the dialog cache instead of rebuilding from the snapshot.
/// `None` before the first refresh; callers fall back to `from_snapshot`.
pub(super) fn cloned_cached_settings() -> Option<AndroidSettings> {
    bridge::with_settings(|state| state.cache.clone())
}

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

fn begin_request(current: &AndroidSettings) -> Option<(SettingsRequestId, String)> {
    bridge::with_settings(|state| {
        let request_id = state.flight.begin()?;
        let defs = AndroidSettings::cached_setting_defs_locked(&mut state.defs_cache, current);
        Some((
            request_id,
            AndroidSettings::dialog_payload_from_defs(request_id.0, &defs),
        ))
    })
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

pub(super) fn request_show_settings_dialog(
    app: &AndroidApp,
    current: &AndroidSettings,
) -> Result<bool, String> {
    let Some((request_id, payload)) = begin_request(current) else {
        return Ok(false);
    };

    // Route every raw `run_on_java_main_thread` through the bridge helper;
    // the on_failure synthesis keeps the gate unwedged on a dead bridge.
    bridge::post_to_activity(
        app,
        "show Android settings dialog",
        move |env, activity| show_settings_with_env_inner(env, &activity, &payload),
        Some(Box::new(move || {
            publish_result(request_id, SettingsDialogResult::Dismissed)
        })),
    );
    Ok(true)
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

// JNI callback for the settings dialog result.
// Every dismissal carries versioned JSON with its request ID.

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onSettingsDialogResult(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    result: JString<'_>,
) {
    match bridge::with_callback_env(&mut env, |env| -> jni::errors::Result<Option<String>> {
        if result.is_null() {
            Ok(None)
        } else {
            Ok(Some(result.try_to_string(env)?))
        }
    }) {
        jni::Outcome::Ok(Some(raw)) => match serde_json::from_str::<SettingsResultPayload>(&raw) {
            Ok(payload) if payload.schema_version == SETTINGS_SCHEMA_VERSION => {
                let result = if payload.dismissed {
                    SettingsDialogResult::Dismissed
                } else {
                    SettingsDialogResult::Applied(payload.values)
                };
                publish_result(payload.request_id, result);
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
