use jni::objects::{JObject, JString, JValue};
use jni::refs::Global;
use jni::sys::jobject;
use jni::{JavaVM, jni_sig, jni_str};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use winit::platform::android::activity::{AndroidApp, AndroidAppWaker};

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
