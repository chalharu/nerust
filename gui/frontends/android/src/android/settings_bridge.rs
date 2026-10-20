//! Settings bridge: dialog flight, cache slot, and JNI handoff.
//! Model and codec stay pure; this module owns the single-flight state.

use std::{collections::BTreeMap, sync::Arc};

use jni::objects::{JObject, JString, JValue};
use winit::platform::android::activity::AndroidApp;

use super::bridge::{self, SettingsDialogResult};
use super::settings_model::{AndroidSettings, SETTINGS_SCHEMA_VERSION, SettingDef};
use nerust_settings_core::model::local::ScreenOrientation;

/// Cached [`AndroidSettings::setting_defs`], keyed by a fingerprint.
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
    request_id: u64,
    #[serde(default)]
    dismissed: bool,
    #[serde(default)]
    values: BTreeMap<String, usize>,
}

/// Update cached settings so `show_settings_dialog_sync` can present current data.
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

fn begin_request(current: &AndroidSettings) -> Option<(u64, String)> {
    bridge::with_settings(|state| {
        if state.pending_request_id.is_some() {
            return None;
        }
        let request_id = state.next_request_id;
        state.next_request_id = state.next_request_id.wrapping_add(1).max(1);
        state.pending_request_id = Some(request_id);
        let defs = AndroidSettings::cached_setting_defs_locked(&mut state.defs_cache, current);
        Some((
            request_id,
            AndroidSettings::dialog_payload_from_defs(request_id, &defs),
        ))
    })
}

pub(super) fn show_settings_dialog_sync(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
) -> Result<bool, String> {
    let current = bridge::with_settings(|state| state.cache.clone())
        .ok_or_else(|| "Android settings cache is empty".to_string())?;
    let Some((request_id, payload)) = begin_request(&current) else {
        return Ok(false);
    };
    if let Err(error) = show_settings_with_env(env, activity, &payload) {
        bridge::with_settings(|state| {
            if state.pending_request_id == Some(request_id) {
                state.pending_request_id = None;
            }
        });
        return Err(error);
    }
    Ok(true)
}

pub(super) fn take_result() -> Option<SettingsDialogResult> {
    bridge::with_settings(|state| state.result.take())
}

pub(super) fn is_dialog_open() -> bool {
    bridge::with_settings(|state| state.pending_request_id.is_some())
}

pub(super) fn reset_transient() {
    bridge::with_settings(|state| {
        state.result = None;
        state.pending_request_id = None;
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

fn show_settings_with_env(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
    payload: &str,
) -> Result<(), String> {
    env.with_local_frame(8, |env| {
        show_settings_with_env_inner(env, activity, payload)
    })
    .map_err(|error| format!("failed to show Android settings dialog: {error:?}"))
}

fn show_settings_with_env_inner(
    env: &mut jni::Env<'_>,
    activity: &JObject<'_>,
    payload: &str,
) -> Result<(), jni::errors::Error> {
    let payload = env.new_string(payload)?;

    bridge::call_bridge_method(
        env,
        &activity,
        &bridge::methods::SHOW_SETTINGS_DIALOG,
        &[JValue::Object(payload.as_ref())],
    )?;
    Ok(())
}

fn publish_result(request_id: u64, result: SettingsDialogResult) {
    let accepted = bridge::with_settings(|state| {
        if state.pending_request_id != Some(request_id) {
            return false;
        }
        state.result = Some(result);
        state.pending_request_id = None;
        true
    });
    if !accepted {
        log::warn!("ignoring stale Android settings result for request {request_id}");
        return;
    }
    bridge::wake();
}

fn resolve_pending_as_dismissed() {
    bridge::with_settings(|state| {
        state.result = Some(SettingsDialogResult::Dismissed);
        state.pending_request_id = None;
    });
    bridge::wake();
}

/// Id-conditional variant: a stale callback must not clear the live flight.
fn resolve_pending_as_dismissed_for(request_id: u64) {
    let cleared = bridge::with_settings(|state| {
        if state.pending_request_id != Some(request_id) {
            return false;
        }
        state.result = Some(SettingsDialogResult::Dismissed);
        state.pending_request_id = None;
        true
    });
    if !cleared {
        log::warn!("ignoring stale Android settings failure for request {request_id}");
        return;
    }
    bridge::wake();
}

// JNI callback for the settings dialog result.
// Null means dismissed; otherwise versioned JSON with ID and values.

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
                resolve_pending_as_dismissed();
            }
        },
        jni::Outcome::Ok(None) => {
            // Null is the documented dismissed encoding: resolve the
            // single flight instead of wedging `pending_request_id`.
            log::debug!("Android settings dialog dismissed without result");
            resolve_pending_as_dismissed();
        }
        jni::Outcome::Err(_) => {
            resolve_pending_as_dismissed();
        }
        jni::Outcome::Panic(_) => {
            log::error!("Android settings dialog callback panicked");
            resolve_pending_as_dismissed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_result_resolves_pending_as_dismissed() {
        // A garbage close must not wedge the open flag.
        // The flight resolves as dismissed; the bridge reports closed.
        bridge::with_settings(|state| state.pending_request_id = Some(7));
        assert!(is_dialog_open());
        resolve_pending_as_dismissed();
        assert!(!is_dialog_open());
        assert_eq!(take_result(), Some(SettingsDialogResult::Dismissed));
        assert_eq!(take_result(), None);
    }
}
