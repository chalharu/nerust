//! Settings bridge: versioned payload push plus synchronous command JNI.
//! Screen truth (single flight plus draft snapshot) lives here; views and
//! validation live in `settings_views`. Kotlin renders sections and echoes
//! one command per edit; every JNI entry builds a throwaway view model.

use jni::objects::{JObject, JString, JValue};
use winit::platform::android::activity::AndroidApp;

use super::bridge::{self, SettingsDialogResult};
use super::settings_views;
use crate::bridge_contract::SETTINGS_SCHEMA_VERSION;
use crate::settings_flight::SettingsRequestId;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_settings_core::i18n::language_from_locale_tag;
use nerust_settings_core::model::language::AppLanguage;
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
        settings_views::open_payload(
            snapshot,
            &state.keyboards,
            state.touch_capable,
            &state.gamepads,
            state.system_locale,
        )
    })
    .map_err(|error| error.to_string())?;
    let payload = bridge::with_settings(|state| {
        let request_id = state.flight.begin()?;
        state.draft = Some(snapshot.clone());
        state.pristine = Some(snapshot.clone());
        state.capture = None;
        Some((request_id, payload))
    });
    let Some((request_id, payload)) = payload else {
        return Ok(false);
    };
    let tagged = serde_json::json!({
        "schemaVersion": SETTINGS_SCHEMA_VERSION,
        "requestId": request_id.0,
        "dirty": false,
        "payload": payload,
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
                state.pristine = None;
                state.capture = None;
            });
            publish_result(request_id, SettingsDialogResult::Dismissed)
        })),
    );
    Ok(true)
}

/// Display language for live surfaces (OSD notifications, error queue):
/// the saved choice, with SystemDefault following the vault-pushed
/// terminal locale. Settings projections resolve per call through `I18n`;
/// this is the same rule for surfaces that never open the screen.
/// The resolved value is never persisted; the draft keeps the choice.
pub(super) fn display_language(choice: AppLanguage) -> AppLanguage {
    let system = bridge::with_settings(|state| state.system_locale);
    settings_views::resolve_display(choice, system).language()
}

/// Publish the saved in-app language without opening the screen.
pub(super) fn apply_app_language(app: &AndroidApp, tag: &str) {
    let tag = tag.to_string();
    bridge::post_to_activity(
        app,
        "apply Android app language",
        move |env, activity| {
            let tag = env.new_string(tag)?;
            bridge::call_bridge_method(
                env,
                &activity,
                &bridge::methods::APPLY_APP_LANGUAGE,
                &[JValue::Object(tag.as_ref())],
            )?;
            Ok(())
        },
        None,
    );
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
                        state.pristine = None;
                        state.capture = None;
                        Some(SettingsDialogResult::Dismissed)
                    } else {
                        state.capture = None;
                        state.pristine = None;
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
                publish_result(payload.request_id, SettingsDialogResult::Dismissed);
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

/// Vault push for the terminal locale tag (MainActivity onResume):
/// resolves SystemDefault for display, never persisted. Empty tags are
/// ignored so a transient unknown never wipes a known locale.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_pushSystemLocale(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    tag: JString<'_>,
) {
    let parsed = bridge::with_callback_env(&mut env, |env| -> jni::errors::Result<String> {
        tag.try_to_string(env)
    });
    let tag = match parsed {
        jni::Outcome::Ok(tag) if !tag.is_empty() => tag,
        _ => return,
    };
    bridge::with_settings(|state| {
        state.system_locale = Some(language_from_locale_tag(&tag));
    });
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

/// Fail envelope: every JNI error tail shares one shape.
fn err_json(error: impl std::fmt::Display) -> String {
    serde_json::json!({"ok": false, "error": error.to_string()}).to_string()
}

/// Success envelope: one serialization at this JNI edge, no re-parse or
/// string concat of an already-built payload. `dirty` compares the draft
/// against the pristine open copy, so reverts read clean.
#[derive(serde::Serialize)]
struct Envelope<'a> {
    ok: bool,
    dirty: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    consumed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<&'a settings_views::ViewPayload>,
}

/// Success envelope: the payload serializes exactly once at this JNI edge.
/// The envelope shape is unchanged (`{"ok":true,"payload":{...}}`); only the
/// old re-parse/concat round-trip is gone.
fn envelope_ok(payload: &settings_views::ViewPayload, dirty: bool) -> Result<String, String> {
    serde_json::to_string(&Envelope {
        ok: true,
        dirty,
        consumed: None,
        payload: Some(payload),
    })
    .map_err(|error| error.to_string())
}

/// Hardware-key envelope: consumed presses carry the refreshed payload;
/// ignored keys carry only the flag (Kotlin ignores the payload then).
fn envelope_key(
    payload: Option<&settings_views::ViewPayload>,
    consumed: bool,
    dirty: bool,
) -> Result<String, String> {
    serde_json::to_string(&Envelope {
        ok: true,
        dirty,
        consumed: Some(consumed),
        payload,
    })
    .map_err(|error| error.to_string())
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
        jni::Outcome::Ok((keycode, pressed)) => {
            // Compute under the lock, serialize outside it: the payload
            // string is the expensive part and needs no vault access.
            // Dirty compares the draft against the pristine open copy in
            // a second trivial read lock, so no borrow spans the mutation.
            let applied = bridge::with_settings(|state| {
                let Some(draft) = state.draft.as_mut() else {
                    return Err("no settings draft".to_string());
                };
                if state.capture.is_none() {
                    return Ok((None, false));
                }
                settings_views::apply_hardware_key(
                    keycode,
                    pressed,
                    draft,
                    &mut state.capture,
                    settings_views::projection(
                        &state.keyboards,
                        state.touch_capable,
                        &state.gamepads,
                        settings_views::resolve_display(
                            draft.shared.general.language,
                            state.system_locale,
                        ),
                    ),
                )
            });
            let dirty = bridge::with_settings(|state| state.draft != state.pristine);
            match applied {
                Ok((payload, consumed)) => {
                    if payload.is_some() || consumed {
                        envelope_key(payload.as_ref(), consumed, dirty).unwrap_or_else(err_json)
                    } else {
                        serde_json::json!({"ok": true, "consumed": false, "dirty": dirty})
                            .to_string()
                    }
                }
                Err(error) => err_json(error),
            }
        }
        _ => err_json("jni decode failed"),
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
        jni::Outcome::Ok(command_json) => {
            // Compute under the lock, serialize outside it.
            // Dirty compares the draft against the pristine open copy in
            // a second trivial read lock, so no borrow spans the mutation.
            let applied = bridge::with_settings(|state| {
                let Some(draft) = state.draft.as_mut() else {
                    return Err("no settings draft".to_string());
                };
                settings_views::apply_command_to(
                    draft,
                    &command_json,
                    &mut state.capture,
                    &mut state.keyboards,
                    &mut state.touch_capable,
                    &mut state.gamepads,
                    &mut state.system_locale,
                )
            });
            let dirty = bridge::with_settings(|state| state.draft != state.pristine);
            match applied {
                Ok(payload) => envelope_ok(&payload, dirty).unwrap_or_else(err_json),
                Err(error) => err_json(error),
            }
        }
        _ => err_json("jni decode failed"),
    };
    match bridge::with_callback_env(&mut *env, |env| -> jni::errors::Result<jni::sys::jstring> {
        Ok(env.new_string(out)?.into_raw())
    }) {
        jni::Outcome::Ok(ptr) => ptr,
        _ => std::ptr::null_mut(),
    }
}
