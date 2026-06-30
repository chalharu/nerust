use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JString, JValue},
    refs::Global,
    sys::jobject,
};
use winit::platform::android::activity::{AndroidApp, AndroidAppWaker};

use crate::import_metadata;

const ROM_PICKER_BUFFER_CAPACITY: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RomPickerResult {
    Cancelled,
    Selected(String),
}

static PICKER_RESULT: Mutex<Option<RomPickerResult>> = Mutex::new(None);
static PICKER_WAKER: Mutex<Option<AndroidAppWaker>> = Mutex::new(None);
static PICKER_REQUEST_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

pub(crate) fn bind_app(app: &AndroidApp) {
    *PICKER_WAKER.lock().expect("picker waker mutex poisoned") = Some(app.create_waker());
    *PICKER_RESULT.lock().expect("picker result mutex poisoned") = None;
    PICKER_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
}

pub(crate) fn reset() {
    *PICKER_RESULT.lock().expect("picker result mutex poisoned") = None;
    PICKER_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
}

pub(crate) fn take_result() -> Option<RomPickerResult> {
    PICKER_RESULT
        .lock()
        .expect("picker result mutex poisoned")
        .take()
}

pub(crate) fn request_open_document(app: &AndroidApp) -> Result<bool, String> {
    if PICKER_REQUEST_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return Ok(false);
    }
    let app = app.clone();
    let callback_app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = start_picker_on_java_main_thread(&callback_app) {
            log::error!("{error}");
            PICKER_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
            wake_main_thread();
        }
    }));
    Ok(true)
}

pub(crate) fn read_uri_bytes(app: &AndroidApp, uri: &str) -> Result<Vec<u8>, String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
    vm.attach_current_thread(|env| {
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
                    let chunk = env.convert_byte_array(&buffer)?;
                    bytes.extend_from_slice(&chunk[..read as usize]);
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
    vm.attach_current_thread(|env| -> jni::errors::Result<Option<String>> {
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

fn start_picker_on_java_main_thread(app: &AndroidApp) -> Result<(), String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
    vm.attach_current_thread(|env| {
        env.with_local_frame(4, |env| {
            let activity_raw = app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            env.call_method(
                activity.as_ref(),
                jni_str!("startRomPicker"),
                jni_sig!("()V"),
                &[],
            )?;
            Ok::<(), jni::errors::Error>(())
        })
    })
    .map_err(|error| format!("failed to launch Android ROM picker: {error:?}"))
}

fn publish_result(result: RomPickerResult) {
    *PICKER_RESULT.lock().expect("picker result mutex poisoned") = Some(result);
    PICKER_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
    wake_main_thread();
}

fn wake_main_thread() {
    if let Some(waker) = PICKER_WAKER
        .lock()
        .expect("picker waker mutex poisoned")
        .clone()
    {
        waker.wake();
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onFilePickerResult(
    mut env: jni::EnvUnowned<'_>,
    _activity: JObject<'_>,
    uri: JString<'_>,
) {
    match env
        .with_env(|env| -> jni::errors::Result<RomPickerResult> {
            if uri.is_null() {
                Ok(RomPickerResult::Cancelled)
            } else {
                let uri = uri.try_to_string(env)?;
                Ok(RomPickerResult::Selected(uri))
            }
        })
        .into_outcome()
    {
        jni::Outcome::Ok(result) => publish_result(result),
        jni::Outcome::Err(error) => {
            log::error!("failed to decode Android ROM picker result: {error:?}");
            publish_result(RomPickerResult::Cancelled);
        }
        jni::Outcome::Panic(_) => {
            log::error!("Android ROM picker callback panicked");
            publish_result(RomPickerResult::Cancelled);
        }
    }
}
