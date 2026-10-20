use std::sync::{OnceLock, mpsc::Sender};

use jni::{
    JavaVM, jni_sig, jni_str,
    objects::{JObject, JString, JValue},
    refs::Global,
    sys::jobject,
};
use sha2::{Digest, Sha256};
use winit::platform::android::activity::AndroidApp;

use super::bridge::{self, RomPickerResult};
use crate::{fetch_seq::FetchSeq, import_metadata};

const ROM_PICKER_BUFFER_CAPACITY: usize = 64 * 1024;

/// One fetch job for the single picker worker; the latest sequence wins.
enum FetchJob {
    Document {
        seq: u64,
        app: AndroidApp,
        uri: String,
        restore_hidden_state: bool,
    },
    Library {
        seq: u64,
        path: std::path::PathBuf,
        id: String,
    },
}

/// Monotonic fetch generation: every request takes the next id.
static FETCH_SEQ: FetchSeq = FetchSeq::new();

/// Single fetch worker: one thread serves every request in order.
/// Stale completions drop instead of racing the latest request.
static FETCH_WORKER: OnceLock<Sender<FetchJob>> = OnceLock::new();

fn fetch_worker() -> &'static Sender<FetchJob> {
    FETCH_WORKER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<FetchJob>();
        if let Err(error) = std::thread::Builder::new()
            .name("nerust-picker-fetch".to_string())
            .spawn(move || {
                for job in rx {
                    run_fetch_job(job);
                }
            })
        {
            log::error!("failed to spawn Android picker fetch worker: {error}");
        }
        tx
    })
}

/// Latest request wins: a stale completion drops before publishing.
fn publish_if_latest(seq: u64, result: RomPickerResult) {
    let latest = FETCH_SEQ.is_latest(seq);
    bridge::with_picker(|state| {
        state.in_flight = false;
        if latest {
            state.result = Some(result);
        }
    });
    if !latest {
        log::debug!("dropping stale Android picker fetch result");
        return;
    }
    bridge::wake();
}

fn run_fetch_job(job: FetchJob) {
    match job {
        FetchJob::Document {
            seq,
            app,
            uri,
            restore_hidden_state,
        } => {
            match fetch_document_bytes(&app, &uri) {
                Ok((bytes, display_name)) => {
                    let (display, extension) =
                        import_metadata::infer_import_metadata(display_name.as_deref(), &uri);
                    let file_name = if extension.is_empty() {
                        display
                    } else {
                        format!("{display}.{extension}")
                    };
                    // Hash off the event thread; the digest rides the
                    // buffer so the event thread never hashes.
                    let digest: [u8; 32] = Sha256::digest(&bytes).into();
                    publish_if_latest(
                        seq,
                        RomPickerResult::RomBytes {
                            uri,
                            file_name,
                            bytes,
                            digest,
                            restore_hidden_state,
                        },
                    );
                }
                Err(error) => {
                    log::error!("{error}");
                    publish_if_latest(
                        seq,
                        if restore_hidden_state {
                            RomPickerResult::RestoreDocumentFailed(error)
                        } else {
                            RomPickerResult::Failed(error)
                        },
                    );
                }
            }
        }
        FetchJob::Library { seq, path, id } => match std::fs::read(&path) {
            Ok(bytes) => {
                let digest: [u8; 32] = Sha256::digest(&bytes).into();
                publish_if_latest(seq, RomPickerResult::LibraryBytes { id, bytes, digest });
            }
            Err(error) => {
                log::error!("library fetch failed for {id}: {error}");
                publish_if_latest(
                    seq,
                    RomPickerResult::RestoreLibraryFailed(format!(
                        "failed to load ROM from library: {error}"
                    )),
                );
            }
        },
    }
}

pub(super) fn take_result() -> Option<RomPickerResult> {
    bridge::with_picker(|state| state.result.take())
}

pub(super) fn reset_transient() {
    bridge::with_picker(|state| {
        state.result = None;
        state.in_flight = false;
    });
}

pub(super) fn request_open_document(app: &AndroidApp) -> Result<bool, String> {
    if bridge::with_picker(|state| {
        let busy = state.in_flight;
        state.in_flight = true;
        busy
    }) {
        return Ok(false);
    }
    bridge::post_to_activity(
        app,
        "launch Android ROM picker",
        |env, activity| {
            env.with_local_frame(4, |env| {
                bridge::call_bridge_method(
                    env,
                    &activity,
                    &bridge::methods::START_ROM_PICKER,
                    &[],
                )?;
                Ok::<(), jni::errors::Error>(())
            })?;
            Ok(())
        },
        Some(Box::new(|| {
            bridge::with_picker(|state| state.in_flight = false);
            bridge::wake();
        })),
    );
    Ok(true)
}

pub(super) fn request_open_document_tree(app: &AndroidApp) -> Result<bool, String> {
    if bridge::with_picker(|state| {
        let busy = state.in_flight;
        state.in_flight = true;
        busy
    }) {
        return Ok(false);
    }
    bridge::post_to_activity(
        app,
        "launch Android directory picker",
        |env, activity| {
            bridge::call_bridge_method(
                env,
                &activity,
                &bridge::methods::START_DIRECTORY_PICKER,
                &[],
            )?;
            Ok(())
        },
        Some(Box::new(|| {
            bridge::with_picker(|state| state.in_flight = false);
            bridge::wake();
        })),
    );
    Ok(true)
}

/// Fetch the picked document on the single worker; the event thread
/// only dispatches the completed buffer published here.
pub(super) fn spawn_document_fetch(app: AndroidApp, uri: String, restore_hidden_state: bool) {
    let seq = FETCH_SEQ.next();
    if fetch_worker()
        .send(FetchJob::Document {
            seq,
            app,
            uri,
            restore_hidden_state,
        })
        .is_err()
    {
        publish_if_latest(
            seq,
            RomPickerResult::Failed("picker fetch worker is gone".to_string()),
        );
    }
}

/// Fetch a library ROM on the single worker; same dispatch shape as documents.
/// Reads through an owned path so the worker never borrows session storage.
pub(super) fn spawn_library_fetch(path: std::path::PathBuf, id: String) {
    let seq = FETCH_SEQ.next();
    if fetch_worker()
        .send(FetchJob::Library { seq, path, id })
        .is_err()
    {
        publish_if_latest(
            seq,
            RomPickerResult::RestoreLibraryFailed("picker fetch worker is gone".to_string()),
        );
    }
}

/// One attach per fetch: resolver, parsed URI, display name, and bytes
/// share a single frame; the interned URI string is created once.
fn fetch_document_bytes(app: &AndroidApp, uri: &str) -> Result<(Vec<u8>, Option<String>), String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
    bridge::with_java_env(&vm, |env| {
        env.with_local_frame(32, |env| {
            let activity_raw = app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };
            let uri_string = JString::from_str(env, uri)?;
            let resolver = env
                .call_method(
                    activity.as_ref(),
                    jni_str!("getContentResolver"),
                    jni_sig!("()Landroid/content/ContentResolver;"),
                    &[],
                )?
                .l()?;
            let uri_object = env
                .call_static_method(
                    jni_str!("android/net/Uri"),
                    jni_str!("parse"),
                    jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                    &[JValue::Object(uri_string.as_ref())],
                )?
                .l()?;
            let display_name =
                read_display_name_inner(env, &resolver, &uri_object).unwrap_or_else(|error| {
                    log::warn!("failed to read Android document display name for {uri}: {error:?}");
                    None
                });
            let bytes = read_stream_bytes_inner(env, &resolver, &uri_object)?;
            Ok::<_, jni::errors::Error>((bytes, display_name))
        })
    })
    .map_err(|error| format!("failed to read Android document URI {uri}: {error:?}"))
}

fn read_display_name_inner(
    env: &mut jni::Env<'_>,
    resolver: &JObject<'_>,
    uri_object: &JObject<'_>,
) -> Result<Option<String>, jni::errors::Error> {
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
            resolver,
            jni_str!("query"),
            jni_sig!(
                "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"
            ),
            &[
                JValue::Object(uri_object),
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
}

fn read_stream_bytes_inner(
    env: &mut jni::Env<'_>,
    resolver: &JObject<'_>,
    uri_object: &JObject<'_>,
) -> Result<Vec<u8>, jni::errors::Error> {
    let stream = env
        .call_method(
            resolver,
            jni_str!("openInputStream"),
            jni_sig!("(Landroid/net/Uri;)Ljava/io/InputStream;"),
            &[JValue::Object(uri_object)],
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
}

fn publish_result(result: RomPickerResult) {
    bridge::with_picker(|state| {
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
