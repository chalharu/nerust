use std::sync::Mutex;

use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JString, JValue},
    refs::Global,
    sys::jobject,
};
use sha2::{Digest, Sha256};
use winit::platform::android::activity::AndroidApp;

use crate::{
    android::{bridge, messages::RomPickerResult},
    import_metadata,
};

const ROM_PICKER_BUFFER_CAPACITY: usize = 64 * 1024;

struct PickerState {
    result: Option<RomPickerResult>,
    in_flight: bool,
}

static PICKER_STATE: Mutex<PickerState> = Mutex::new(PickerState {
    result: None,
    in_flight: false,
});

/// Lock order for the three JNI-bridge mutexes: never nest `with_state`.
/// Poisoned mutexes recover state and log instead of wedging.
fn with_state<T>(operation: impl FnOnce(&mut PickerState) -> T) -> T {
    // Same lock-order contract as the bridge mutex (see
    // `bridge::lock_reentrancy_guard`): never nest, wake after release.
    let _guard = bridge::lock_reentrancy_guard();
    let mut guard = PICKER_STATE.lock().unwrap_or_else(|poison| {
        log::error!("picker state mutex poisoned; recovering state");
        poison.into_inner()
    });
    operation(&mut guard)
}

pub(crate) fn take_result() -> Option<RomPickerResult> {
    with_state(|state| state.result.take())
}

pub(crate) fn reset_transient() {
    with_state(|state| {
        state.result = None;
        state.in_flight = false;
    });
}

pub(crate) fn request_open_document(app: &AndroidApp) -> Result<bool, String> {
    if with_state(|state| {
        let busy = state.in_flight;
        state.in_flight = true;
        busy
    }) {
        return Ok(false);
    }
    // Route JNI failures through the bridge helper for one shared log label.
    // The on_failure synthesis keeps the gate unwedged.
    bridge::call_on_activity_or(
        app,
        "launch Android ROM picker",
        |env, activity| {
            env.with_local_frame(4, |env| {
                env.call_method(
                    activity.as_ref(),
                    jni_str!("startRomPicker"),
                    jni_sig!("()V"),
                    &[],
                )?;
                Ok::<(), jni::errors::Error>(())
            })?;
            Ok(())
        },
        || {
            with_state(|state| state.in_flight = false);
            bridge::wake();
        },
    );
    Ok(true)
}

pub(crate) fn request_open_document_tree(app: &AndroidApp) -> Result<bool, String> {
    if with_state(|state| {
        let busy = state.in_flight;
        state.in_flight = true;
        busy
    }) {
        return Ok(false);
    }
    bridge::call_on_activity_or(
        app,
        "launch Android directory picker",
        |env, activity| {
            env.call_method(
                activity.as_ref(),
                jni_str!("startDirectoryPicker"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok(())
        },
        || {
            with_state(|state| state.in_flight = false);
            bridge::wake();
        },
    );
    Ok(true)
}

/// Fetch the picked document off the event-loop thread; the event thread
/// only dispatches the completed buffer published here.
pub(crate) fn spawn_document_fetch(app: AndroidApp, uri: String, restore_hidden_state: bool) {
    let job = move || {
        let (display_name, extension) = infer_import_metadata(&app, &uri);
        let file_name = if extension.is_empty() {
            display_name
        } else {
            format!("{display_name}.{extension}")
        };
        match read_uri_bytes(&app, &uri) {
            Ok(bytes) => {
                // Hash off the event thread; the digest rides the
                // buffer so the event thread never hashes.
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                publish_result(RomPickerResult::RomBytes {
                    uri,
                    file_name,
                    bytes,
                    digest,
                    restore_hidden_state,
                })
            }
            Err(error) => {
                log::error!("{error}");
                publish_result(if restore_hidden_state {
                    RomPickerResult::RestoreDocumentFailed(error)
                } else {
                    RomPickerResult::Failed(error)
                });
            }
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name("nerust-picker-fetch".to_string())
        .spawn(job)
    {
        publish_result(RomPickerResult::Failed(format!(
            "failed to spawn ROM fetch thread: {error}"
        )));
    }
}

/// Fetch a library ROM off the event-loop thread; same dispatch shape as documents.
/// Reads through an owned path so the worker never borrows session storage.
pub(crate) fn spawn_library_fetch(path: std::path::PathBuf, id: String) {
    let job = move || match std::fs::read(&path) {
        Ok(bytes) => {
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            publish_result(RomPickerResult::LibraryBytes { id, bytes, digest });
        }
        Err(error) => {
            log::error!("library fetch failed for {id}: {error}");
            publish_result(RomPickerResult::RestoreLibraryFailed(format!(
                "failed to load ROM from library: {error}"
            )));
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name("nerust-library-fetch".to_string())
        .spawn(job)
    {
        publish_result(RomPickerResult::RestoreLibraryFailed(format!(
            "failed to spawn library fetch thread: {error}"
        )));
    }
}

pub(crate) fn read_uri_bytes(app: &AndroidApp, uri: &str) -> Result<Vec<u8>, String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
    bridge::with_java_env(&vm, |env| {
        env.with_local_frame(16, |env| {
            let activity_raw = app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let resolver = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getContentResolver"),
                    jni_sig!("()Landroid/content/ContentResolver;"),
                    &[],
                )?
                .l()?;
            let uri_string = JString::from_str(env, uri)?;
            let uri_object = env
                .call_static_method(
                    jni_str!("android/net/Uri"),
                    jni_str!("parse"),
                    jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                    &[JValue::Object(uri_string.as_ref())],
                )?
                .l()?;
            let stream = env
                .call_method(
                    &resolver,
                    jni_str!("openInputStream"),
                    jni_sig!("(Landroid/net/Uri;)Ljava/io/InputStream;"),
                    &[JValue::Object(&uri_object)],
                )?
                .l()?;
            if stream.is_null() {
                return Err(jni::errors::Error::NullPtr("openInputStream returned null"));
            }
            let buffer = env.new_byte_array(ROM_PICKER_BUFFER_CAPACITY)?;
            let mut bytes = Vec::new();
            let read_result = (|| {
                loop {
                    let read = env
                        .call_method(
                            &stream,
                            jni_str!("read"),
                            jni_sig!("([B)I"),
                            &[JValue::Object(buffer.as_ref())],
                        )?
                        .i()?;
                    if read < 0 {
                        break;
                    }
                    if read == 0 {
                        continue;
                    }
                    // Copy only the bytes read, not the whole 64KB scratch.
                    let mut chunk = vec![0i8; read as usize];
                    buffer.get_region(env, 0, &mut chunk)?;
                    bytes.extend(chunk.iter().map(|byte| *byte as u8));
                }
                Ok::<(), jni::errors::Error>(())
            })();
            let close_result = env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[]);
            read_result?;
            close_result?;
            Ok(bytes)
        })
    })
    .map_err(|error| format!("failed to read Android document URI {uri}: {error:?}"))
}

pub(crate) fn infer_import_metadata(app: &AndroidApp, uri: &str) -> (String, String) {
    let display_name_hint = match read_display_name(app, uri) {
        Ok(display_name_hint) => display_name_hint,
        Err(error) => {
            log::warn!("{error}");
            None
        }
    };
    import_metadata::infer_import_metadata(display_name_hint.as_deref(), uri)
}

fn read_display_name(app: &AndroidApp, uri: &str) -> Result<Option<String>, String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
    bridge::with_java_env(&vm, |env| -> jni::errors::Result<Option<String>> {
        env.with_local_frame(32, |env| -> jni::errors::Result<Option<String>> {
            let activity_raw = app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let resolver = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getContentResolver"),
                    jni_sig!("()Landroid/content/ContentResolver;"),
                    &[],
                )?
                .l()?;
            let uri_string = JString::from_str(env, uri)?;
            let uri_object = env
                .call_static_method(
                    jni_str!("android/net/Uri"),
                    jni_str!("parse"),
                    jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                    &[JValue::Object(uri_string.as_ref())],
                )?
                .l()?;
            let display_name_column = env
                .get_static_field(
                    jni_str!("android/provider/OpenableColumns"),
                    jni_str!("DISPLAY_NAME"),
                    jni_sig!("Ljava/lang/String;"),
                )?
                .l()?;
            let string_class = env.find_class(jni_str!("java/lang/String"))?;
            let projection = env.new_object_array(1, &string_class, JObject::null())?;
            projection.set_element(env, 0, &display_name_column)?;
            let null = JObject::null();
            let cursor = env
                .call_method(
                    &resolver,
                    jni_str!("query"),
                    jni_sig!(
                        "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"
                    ),
                    &[
                        JValue::Object(&uri_object),
                        JValue::Object(projection.as_ref()),
                        JValue::Object(&null),
                        JValue::Object(&null),
                        JValue::Object(&null),
                    ],
                )?
                .l()?;
            if cursor.is_null() {
                return Ok(None);
            }
            let read_result: jni::errors::Result<Option<String>> = (|| {
                if !env
                    .call_method(&cursor, jni_str!("moveToFirst"), jni_sig!("()Z"), &[])?
                    .z()?
                {
                    return Ok(None);
                }
                let column_index = env
                    .call_method(
                        &cursor,
                        jni_str!("getColumnIndex"),
                        jni_sig!("(Ljava/lang/String;)I"),
                        &[JValue::Object(&display_name_column)],
                    )?
                    .i()?;
                if column_index < 0 {
                    return Ok(None);
                }
                let value = env
                    .call_method(
                        &cursor,
                        jni_str!("getString"),
                        jni_sig!("(I)Ljava/lang/String;"),
                        &[JValue::Int(column_index)],
                    )?
                    .l()?;
                if value.is_null() {
                    return Ok(None);
                }
                let value = unsafe { JString::from_raw(env, value.into_raw()) };
                let value = value.try_to_string(env)?;
                let value = value.trim();
                Ok((!value.is_empty()).then(|| value.to_string()))
            })();
            let close_result = env.call_method(&cursor, jni_str!("close"), jni_sig!("()V"), &[]);
            let display_name = read_result?;
            close_result?;
            Ok(display_name)
        })
    })
    .map_err(|error: jni::errors::Error| {
        format!("failed to read Android document display name for {uri}: {error:?}")
    })
}

fn publish_result(result: RomPickerResult) {
    with_state(|state| {
        state.result = Some(result);
        state.in_flight = false;
    });
    bridge::wake();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onFilePickerResult(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    uri: JString<'_>,
) {
    match bridge::with_callback_env(&mut env, |env| -> jni::errors::Result<RomPickerResult> {
        if uri.is_null() {
            Ok(RomPickerResult::Cancelled)
        } else {
            let uri = uri.try_to_string(env)?;
            Ok(RomPickerResult::Selected(uri))
        }
    }) {
        jni::Outcome::Ok(result) => {
            log::info!("received Android ROM picker result");
            publish_result(result);
        }
        jni::Outcome::Err(_) => {
            publish_result(RomPickerResult::Cancelled);
        }
        jni::Outcome::Panic(_) => {
            log::error!("Android ROM picker callback panicked");
            publish_result(RomPickerResult::Cancelled);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onDirectoryPickerResult(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    uri: JString<'_>,
) {
    match bridge::with_callback_env(&mut env, |env| -> jni::errors::Result<RomPickerResult> {
        if uri.is_null() {
            Ok(RomPickerResult::Cancelled)
        } else {
            Ok(RomPickerResult::TreeSelected(uri.try_to_string(env)?))
        }
    }) {
        jni::Outcome::Ok(result) => publish_result(result),
        jni::Outcome::Err(_) => {
            publish_result(RomPickerResult::Cancelled);
        }
        jni::Outcome::Panic(_) => publish_result(RomPickerResult::Cancelled),
    }
}
