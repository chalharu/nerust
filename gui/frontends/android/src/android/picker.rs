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
use crate::{
    fetch_seq::FetchSeq,
    import_metadata,
    picker_lane::{BumperBudget, LatestLane},
};

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

/// Single fetch lane: one pending slot, newer pushes drop the stale one.
/// Total retention is pending plus the active job, never a backlog.
static FETCH_LANE: LatestLane<FetchJob> = LatestLane::new();

/// Starts the single worker once; a failed spawn closes the lane so senders
/// report instead of queueing behind a worker that will never run.
fn ensure_fetch_worker() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        if let Err(error) = std::thread::Builder::new()
            .name("nerust-picker-fetch".to_string())
            .spawn(|| {
                while let Some(job) = FETCH_LANE.take() {
                    run_fetch_job(job);
                }
            })
        {
            log::error!("failed to spawn Android picker fetch worker: {error}");
            FETCH_LANE.close();
        }
    });
}

/// Latest request wins: a stale completion drops before publishing.
fn publish_if_latest(seq: u64, result: RomPickerResult) {
    let latest = bridge::with_picker(|state| {
        if !FETCH_SEQ.is_latest(seq) {
            return false;
        }
        state.in_flight = None;
        state.result = Some((seq, result));
        true
    });
    if !latest {
        log::debug!("dropping stale Android picker fetch result");
        return;
    }
    bridge::wake();
}

fn run_fetch_job(job: FetchJob) {
    let seq = match &job {
        FetchJob::Document { seq, .. } | FetchJob::Library { seq, .. } => *seq,
    };
    if !FETCH_SEQ.is_latest(seq) {
        return;
    }
    match job {
        FetchJob::Document {
            seq,
            app,
            uri,
            restore_hidden_state,
        } => {
            match fetch_document_bytes(&app, &uri, seq) {
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
        FetchJob::Library { seq, path, id } => {
            match std::fs::File::open(&path).and_then(|file| FETCH_SEQ.read_latest(seq, file)) {
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
            }
        }
    }
}

pub(super) fn take_result() -> Option<RomPickerResult> {
    bridge::with_picker(|state| {
        state
            .result
            .take()
            .filter(|(seq, _)| FETCH_SEQ.is_latest(*seq))
            .map(|(_, result)| result)
    })
}

pub(super) fn has_pending_result() -> bool {
    bridge::with_picker(|state| {
        state.in_flight.is_some_and(|seq| FETCH_SEQ.is_latest(seq))
            || state
                .result
                .as_ref()
                .is_some_and(|(seq, _)| FETCH_SEQ.is_latest(*seq))
    })
}

pub(super) fn reset_transient() {
    // Take the slot handles here so a reset job never starts provider I/O
    // the resetter cannot cancel; lanes stay available via the cached VM.
    let (signal, stream) = bridge::with_picker(|state| {
        FETCH_SEQ.next();
        state.result = None;
        state.in_flight = None;
        (state.in_flight_signal.take(), state.in_flight_stream.take())
    });
    match CACHED_VM.get() {
        Some(vm) => send_to_bumper(ensure_bumper(vm), signal, stream),
        None => {
            debug_assert!(signal.is_none() && stream.is_none());
            drop(signal);
            drop(stream);
        }
    }
}

pub(super) fn request_open_document(app: &AndroidApp) -> Result<bool, String> {
    // Rejecting a busy request must not consume a generation: the in-flight
    // fetch keeps its claim until its own producer finishes or cancels it.
    if bridge::with_picker(|state| state.in_flight.is_some_and(|seq| FETCH_SEQ.is_latest(seq))) {
        return Ok(false);
    }
    bridge::with_picker(|state| state.in_flight = Some(FETCH_SEQ.next()));
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
            bridge::with_picker(|state| state.in_flight = None);
            bridge::wake();
        })),
    );
    Ok(true)
}

pub(super) fn request_open_document_tree(app: &AndroidApp) -> Result<bool, String> {
    if bridge::with_picker(|state| state.in_flight.is_some_and(|seq| FETCH_SEQ.is_latest(seq))) {
        return Ok(false);
    }
    bridge::with_picker(|state| state.in_flight = Some(FETCH_SEQ.next()));
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
            bridge::with_picker(|state| state.in_flight = None);
            bridge::wake();
        })),
    );
    Ok(true)
}

/// Fetch the picked document on the single worker; the event thread
/// only dispatches the completed buffer published here.
pub(super) fn spawn_document_fetch(app: AndroidApp, uri: String, restore_hidden_state: bool) {
    // A stuck previous fetch wedges the single worker: cancel its provider
    // calls before queuing behind it.
    cancel_in_flight(&app);
    let seq = begin_fetch();
    ensure_fetch_worker();
    let dropped_before = FETCH_LANE.dropped();
    if !FETCH_LANE.send(FetchJob::Document {
        seq,
        app,
        uri,
        restore_hidden_state,
    }) {
        publish_if_latest(
            seq,
            RomPickerResult::Failed("picker fetch worker is gone".to_string()),
        );
    } else {
        // Latest wins: a replaced queue entry owned the previous push.
        if FETCH_LANE.dropped() != dropped_before {
            log::debug!("picker fetch superseded a queued job");
        }
        debug_assert!(FETCH_LANE.pending() <= 1);
    }
}

/// Fetch a library ROM on the single worker; same dispatch shape as documents.
/// Reads through an owned path so the worker never borrows session storage.
pub(super) fn spawn_library_fetch(app: AndroidApp, path: std::path::PathBuf, id: String) {
    // A stuck previous fetch wedges the single worker the same way as for
    // documents: cancel its provider calls before queuing behind it.
    cancel_in_flight(&app);
    let seq = begin_fetch();
    ensure_fetch_worker();
    let dropped_before = FETCH_LANE.dropped();
    if !FETCH_LANE.send(FetchJob::Library { seq, path, id }) {
        publish_if_latest(
            seq,
            RomPickerResult::RestoreLibraryFailed("picker fetch worker is gone".to_string()),
        );
    } else {
        if FETCH_LANE.dropped() != dropped_before {
            log::debug!("picker fetch superseded a queued job");
        }
        debug_assert!(FETCH_LANE.pending() <= 1);
    }
}

/// One attach per fetch: resolver, parsed URI, display name, and bytes
/// share a single frame; the interned URI string is created once.
fn fetch_document_bytes(
    app: &AndroidApp,
    uri: &str,
    seq: u64,
) -> Result<(Vec<u8>, Option<String>), String> {
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
            // One signal per job for the cancellable query and open below.
            // A missing signal degrades to the old uninterruptible calls.
            let signal = env
                .new_object(
                    jni_str!("android/os/CancellationSignal"),
                    jni_sig!("()V"),
                    &[],
                )
                .ok();
            if let Some(signal) = signal.as_ref() {
                // Refused registration means this job lost the fetch: starting
                // provider I/O now would be uncancellable and wedge the worker.
                if !store_job_signal(env, seq, signal) {
                    return Err(jni::errors::Error::NullPtr(
                        "ROM fetch superseded before provider query",
                    ));
                }
            }
            let result = (|| {
                let display_name =
                    read_display_name_inner(env, &resolver, &uri_object, signal.as_ref())
                        .unwrap_or_else(|error| {
                            // Display metadata is optional. Do not let its exception
                            // prevent the independent ROM stream from opening.
                            if env.exception_check() {
                                env.exception_clear();
                            }
                            log::warn!(
                                "failed to read Android document display name for {uri}: {error:?}"
                            );
                            None
                        });
                let bytes =
                    read_stream_bytes_inner(env, &resolver, &uri_object, seq, signal.as_ref())?;
                Ok::<_, jni::errors::Error>((bytes, display_name))
            })();
            release_job_handles(seq);
            result
        })
    })
    .map_err(|error| format!("failed to read Android document URI {uri}: {error:?}"))
}

fn read_display_name_inner(
    env: &mut jni::Env<'_>,
    resolver: &JObject<'_>,
    uri_object: &JObject<'_>,
    signal: Option<&JObject<'_>>,
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
    let signal_arg = signal.unwrap_or(&null);
    let cursor = env
        .call_method(
            resolver,
            jni_str!("query"),
            jni_sig!(
                "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;Landroid/os/CancellationSignal;)Landroid/database/Cursor;"
            ),
            &[
                JValue::Object(uri_object),
                JValue::Object(projection.as_ref()),
                JValue::Object(&null),
                JValue::Object(&null),
                JValue::Object(&null),
                JValue::Object(signal_arg),
            ],
        )?
        .l()?;
    if cursor.is_null() {
        return Ok(None);
    }
    let mut read_result: jni::errors::Result<Option<String>> = (|| {
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
    if env.exception_check() {
        env.exception_clear();
        read_result = Err(read_result
            .err()
            .unwrap_or(jni::errors::Error::JavaException));
    }
    let close_result = env.call_method(&cursor, jni_str!("close"), jni_sig!("()V"), &[]);
    let display_name = read_result?;
    close_result?;
    Ok(display_name)
}

fn read_stream_bytes_inner(
    env: &mut jni::Env<'_>,
    resolver: &JObject<'_>,
    uri_object: &JObject<'_>,
    seq: u64,
    signal: Option<&JObject<'_>>,
) -> Result<Vec<u8>, jni::errors::Error> {
    let null = JObject::null();
    let mode = env.new_string("r")?;
    let descriptor = env
        .call_method(
            resolver,
            jni_str!("openAssetFileDescriptor"),
            jni_sig!(
                "(Landroid/net/Uri;Ljava/lang/String;Landroid/os/CancellationSignal;)Landroid/content/res/AssetFileDescriptor;"
            ),
            &[
                JValue::Object(uri_object),
                JValue::Object(mode.as_ref()),
                JValue::Object(signal.unwrap_or(&null)),
            ],
        )?
        .l()?;
    if descriptor.is_null() {
        return Err(jni::errors::Error::NullPtr(
            "openAssetFileDescriptor returned null",
        ));
    }
    let stream = env
        .call_method(
            &descriptor,
            jni_str!("createInputStream"),
            jni_sig!("()Ljava/io/FileInputStream;"),
            &[],
        )?
        .l()?;
    if stream.is_null() {
        return Err(jni::errors::Error::NullPtr(
            "AssetFileDescriptor.createInputStream returned null",
        ));
    }
    // Registered for cross-thread close; a refused registration closes the stream
    // here and aborts instead of reading uncancellable.
    if !store_job_stream(env, seq, &stream) {
        let _ = env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[]);
        let _ = env.call_method(&descriptor, jni_str!("close"), jni_sig!("()V"), &[]);
        return Err(jni::errors::Error::NullPtr(
            "ROM fetch superseded before provider read",
        ));
    }
    let mut bytes = Vec::new();
    // Reused JNI scratch: one 64KB i8 buffer serves the whole stream,
    // so chunk reads allocate nothing per round-trip.
    let mut chunk: Vec<i8> = Vec::new();
    let mut read_result = (|| {
        // Allocation can fail too: keep it inside the close-protected scope.
        let buffer = env.new_byte_array(ROM_PICKER_BUFFER_CAPACITY)?;
        loop {
            if !FETCH_SEQ.is_latest(seq) {
                return Err(jni::errors::Error::NullPtr("ROM fetch cancelled"));
            }
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
                // Force progress for providers that return zero without EOF.
                let byte = env
                    .call_method(&stream, jni_str!("read"), jni_sig!("()I"), &[])?
                    .i()?;
                if byte < 0 {
                    break;
                }
                bytes.push(byte as u8);
                continue;
            }
            // Copy only the bytes read, not the whole 64KB scratch.
            // The u8 reinterpretation below is a per-byte no-op.
            chunk.resize(read as usize, 0);
            buffer.get_region(env, 0, &mut chunk)?;
            bytes.extend(chunk.iter().map(|byte| *byte as u8));
        }
        Ok::<(), jni::errors::Error>(())
    })();
    if env.exception_check() {
        env.exception_clear();
        read_result = Err(read_result
            .err()
            .unwrap_or(jni::errors::Error::JavaException));
    }
    // Ownership decides the single stream closer: a bumper-owned handle skips
    // the local close and ignores racy close errors instead of failing the read.
    let owned = take_job_stream_if_ours(seq);
    let stream_error = if owned {
        env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[])
            .err()
    } else {
        None
    };
    let descriptor_error = env
        .call_method(&descriptor, jni_str!("close"), jni_sig!("()V"), &[])
        .err();
    read_result?;
    if owned {
        if let Some(error) = stream_error {
            return Err(error);
        }
        if let Some(error) = descriptor_error {
            return Err(error);
        }
    }
    Ok(bytes)
}

/// Reclaim the stream handle before the local close: the slot take decides the single closer.
fn take_job_stream_if_ours(seq: u64) -> bool {
    bridge::with_picker(|state| {
        if state.in_flight == Some(seq) {
            state.in_flight_stream.take().is_some()
        } else {
            false
        }
    })
}

fn begin_fetch() -> u64 {
    bridge::with_picker(|state| {
        state.result = None;
        let seq = FETCH_SEQ.next();
        state.in_flight = Some(seq);
        seq
    })
}

/// Cancel the previous generation's provider calls before a new fetch bumps it.
/// Runs on the event thread, which owns an `AndroidApp` for the VM.
fn cancel_in_flight(app: &AndroidApp) {
    let vm = CACHED_VM
        .get_or_init(|| unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) })
        .clone();
    let (signal, stream) =
        bridge::with_picker(|state| (state.in_flight_signal.take(), state.in_flight_stream.take()));
    send_to_bumper(ensure_bumper(&vm), signal, stream);
}

/// Route taken handles to the bumper lanes, or drop them when teardown is underway.
fn send_to_bumper(
    lanes: &BumperLanes,
    signal: Option<Global<JObject<'static>>>,
    stream: Option<Global<JObject<'static>>>,
) {
    if signal.is_none() && stream.is_none() {
        return;
    }
    // Never touch the provider here: enqueue for the bumper so the event thread never stalls.
    if let Some(signal) = signal {
        if !CANCEL_BUDGET.pushed() {
            log::warn!(
                "picker cancel lane over cap (pending={}); provider may be stuck",
                CANCEL_BUDGET.pending()
            );
        }
        if lanes.cancel.send(signal).is_err() {
            CANCEL_BUDGET.popped();
            log::error!("picker cancel lane is gone; signal dropped uncancelled");
        }
    }
    if let Some(stream) = stream {
        if !CLOSE_BUDGET.pushed() {
            log::warn!(
                "picker close lane over cap (pending={}); provider may be stuck",
                CLOSE_BUDGET.pending()
            );
        }
        if lanes.close.send(stream).is_err() {
            CLOSE_BUDGET.popped();
            log::error!("picker close lane is gone; stream dropped unclosed");
        }
    }
}

/// Process-global VM cached from the first cancelling call for handle-free resets.
static CACHED_VM: OnceLock<JavaVM> = OnceLock::new();

/// Bumper pressure gauges: a count past the cap means a stuck provider.
static CANCEL_BUDGET: BumperBudget = BumperBudget::new(16);
static CLOSE_BUDGET: BumperBudget = BumperBudget::new(16);

/// Cancel/close lanes plus the process VM: separate threads keep closes behind slow cancels.
/// No provider call ever runs on the event thread.
struct BumperLanes {
    cancel: Sender<Global<JObject<'static>>>,
    close: Sender<Global<JObject<'static>>>,
}

static BUMPER_LANES: OnceLock<BumperLanes> = OnceLock::new();

/// Lanes for an explicit VM: the cached process VM in production.
/// Split from the spawner so resets route without an app handle.
fn ensure_bumper(vm: &JavaVM) -> &'static BumperLanes {
    BUMPER_LANES.get_or_init(|| {
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<Global<JObject<'static>>>();
        let (close_tx, close_rx) = std::sync::mpsc::channel::<Global<JObject<'static>>>();
        let cancel_vm = vm.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("nerust-picker-cancel".to_string())
            .spawn(move || {
                for signal in cancel_rx {
                    cancel_bumper_signal(&cancel_vm, signal);
                    CANCEL_BUDGET.popped();
                }
            })
        {
            log::error!("failed to spawn Android picker cancel lane: {error}");
        }
        let close_vm = vm.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("nerust-picker-close".to_string())
            .spawn(move || {
                for stream in close_rx {
                    cancel_bumper_stream(&close_vm, stream);
                    CLOSE_BUDGET.popped();
                }
            })
        {
            log::error!("failed to spawn Android picker close lane: {error}");
        }
        BumperLanes {
            cancel: cancel_tx,
            close: close_tx,
        }
    })
}

fn cancel_bumper_signal(vm: &JavaVM, signal: Global<JObject<'static>>) {
    let _ = bridge::with_java_env(vm, |env| {
        // `Global` deletes its reference on drop; the lane is attached
        // here, so no cross-thread delete call is needed.
        let _ = env.call_method(signal.as_obj(), jni_str!("cancel"), jni_sig!("()V"), &[]);
        Ok::<(), jni::errors::Error>(())
    });
}

fn cancel_bumper_stream(vm: &JavaVM, stream: Global<JObject<'static>>) {
    let _ = bridge::with_java_env(vm, |env| {
        let _ = env.call_method(stream.as_obj(), jni_str!("close"), jni_sig!("()V"), &[]);
        Ok::<(), jni::errors::Error>(())
    });
}

/// Publish the job's signal for cross-thread cancellation; superseded jobs keep local handles.
fn store_job_signal(env: &mut jni::Env<'_>, seq: u64, signal: &JObject<'_>) -> bool {
    let global = env.new_global_ref(signal).ok();
    bridge::with_picker(|state| publish_handle(state, seq, global, true))
}

/// Publish the job's open stream for cross-thread close. Same ownership rule
/// as the signal: the slot take decides which side deletes the global ref.
fn store_job_stream(env: &mut jni::Env<'_>, seq: u64, stream: &JObject<'_>) -> bool {
    let global = env.new_global_ref(stream).ok();
    bridge::with_picker(|state| publish_handle(state, seq, global, false))
}

/// Slot decision for signal/stream registration: only the owning generation may publish.
/// Mirrors `picker_lane::SlotState::publish`.
fn publish_handle(
    state: &mut bridge::PickerSlot,
    seq: u64,
    global: Option<Global<JObject<'static>>>,
    signal: bool,
) -> bool {
    if state.in_flight != Some(seq) {
        return false;
    }
    let Some(global) = global else { return true };
    let slot = if signal {
        &mut state.in_flight_signal
    } else {
        &mut state.in_flight_stream
    };
    if let Some(previous) = slot.replace(global) {
        drop(previous);
    }
    true
}

/// Drop this job's slot handles if no bumper took them first; stale completions take nothing.
fn release_job_handles(seq: u64) {
    let (signal, stream) = bridge::with_picker(|state| {
        if state.in_flight == Some(seq) {
            (state.in_flight_signal.take(), state.in_flight_stream.take())
        } else {
            (None, None)
        }
    });
    drop(signal);
    drop(stream);
}

fn publish_result(result: RomPickerResult) {
    bridge::with_picker(|state| {
        let seq = FETCH_SEQ.next();
        state.result = Some((seq, result));
        state.in_flight = None;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_and_take_agree_on_stale_results() {
        reset_transient();
        // A leftover from a superseded request counts as nothing.
        let stale = FETCH_SEQ.next();
        let current = FETCH_SEQ.next();
        bridge::with_picker(|state| {
            state.result = Some((stale, RomPickerResult::Failed("old".into())));
        });
        assert!(!has_pending_result());
        assert!(take_result().is_none());
        // The latest unconsumed result counts until taken.
        bridge::with_picker(|state| {
            state.result = Some((current, RomPickerResult::Failed("new".into())));
        });
        assert!(has_pending_result());
        assert!(take_result().is_some());
        assert!(!has_pending_result());
        reset_transient();
        assert!(!has_pending_result());
    }

    /// Only the owning generation may publish; refused jobs start no provider I/O.
    #[test]
    fn handle_publish_blocks_superseded_provider_io() {
        use bridge::PickerSlot;
        // Superseded before registering: refused, handle untouched.
        let mut slot = PickerSlot::empty();
        slot.in_flight = Some(7);
        assert!(!publish_handle(&mut slot, 6, None, true));
        assert!(slot.in_flight_signal.is_none());
        assert!(!publish_handle(&mut slot, 6, None, false));
        assert!(slot.in_flight_stream.is_none());
        // Idle slot refuses everything: no owner, no I/O.
        let mut idle = PickerSlot::empty();
        assert!(!publish_handle(&mut idle, 1, None, true));
        // Owner without a handle degrades to local-only calls.
        assert!(publish_handle(&mut slot, 7, None, true));
        assert!(slot.in_flight_signal.is_none());
        assert!(publish_handle(&mut slot, 7, None, false));
        assert!(slot.in_flight_stream.is_none());
    }

    /// Reset, request, and completion keep a single owner at state level.
    #[test]
    fn reset_and_request_orders_keep_one_owner() {
        reset_transient();
        // Normal order: request owns the slot, completion publishes.
        let seq = begin_fetch();
        assert!(bridge::with_picker(|state| state.in_flight == Some(seq)));
        publish_if_latest(seq, RomPickerResult::Failed("ok".into()));
        assert!(take_result().is_some());
        // Reset between the generation check and registration: the stale
        // job loses the slot and a fresh request owns it again.
        let stale = begin_fetch();
        reset_transient();
        assert!(!bridge::with_picker(|state| {
            publish_handle(state, stale, None, true)
        }));
        let next = begin_fetch();
        assert!(bridge::with_picker(|state| {
            publish_handle(state, next, None, true)
        }));
        // Stale completion after reset never publishes.
        publish_if_latest(stale, RomPickerResult::Failed("stale".into()));
        assert!(take_result().is_none());
        publish_if_latest(next, RomPickerResult::Failed("fresh".into()));
        assert!(take_result().is_some());
        reset_transient();
        // Reset then release: the next generation owns the slot cleanly.
        let generation = begin_fetch();
        reset_transient();
        release_job_handles(generation);
        let after = begin_fetch();
        assert!(bridge::with_picker(|state| {
            publish_handle(state, after, None, true)
        }));
        reset_transient();
    }
}
