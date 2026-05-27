use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::refs::Global;
use jni::sys::jobject;
use jni::{JavaVM, jni_sig, jni_str};
use nerust_gui_runtime::rom_library::RomLibraryEntry;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use winit::platform::android::activity::{AndroidApp, AndroidAppWaker};

/// Sent back to Rust when the user taps "Import new ROM…" in the library dialog.
///
/// Must match `MainActivity.IMPORT_ACTION_ID`.
const IMPORT_ACTION_ID: &str = "__import__";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LibraryDialogResult {
    /// The dialog was dismissed without a selection.
    Dismissed,
    /// The user picked an existing library entry with this id.
    Selected(String),
    /// The user requested to import a new ROM via the SAF picker.
    ImportRequested,
}

static LIBRARY_RESULT: Mutex<Option<LibraryDialogResult>> = Mutex::new(None);
static LIBRARY_WAKER: Mutex<Option<AndroidAppWaker>> = Mutex::new(None);
static LIBRARY_REQUEST_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

pub(crate) fn bind_app(app: &AndroidApp) {
    *LIBRARY_WAKER.lock().expect("library waker mutex poisoned") = Some(app.create_waker());
    *LIBRARY_RESULT
        .lock()
        .expect("library result mutex poisoned") = None;
    LIBRARY_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
}

pub(crate) fn reset() {
    *LIBRARY_RESULT
        .lock()
        .expect("library result mutex poisoned") = None;
    LIBRARY_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
}

pub(crate) fn take_result() -> Option<LibraryDialogResult> {
    LIBRARY_RESULT
        .lock()
        .expect("library result mutex poisoned")
        .take()
}

/// Request that the Android side show a ROM library chooser dialog.
///
/// Returns `Ok(false)` when a dialog is already in flight (idempotent guard).
pub(crate) fn request_show_library(
    app: &AndroidApp,
    entries: &[RomLibraryEntry],
) -> Result<bool, String> {
    if LIBRARY_REQUEST_IN_FLIGHT.swap(true, Ordering::AcqRel) {
        return Ok(false);
    }

    let names: Vec<String> = entries.iter().map(|e| e.display_name.clone()).collect();
    let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();

    let app = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = show_dialog_on_java_main_thread(&app, &names, &ids) {
            log::error!("{error}");
            LIBRARY_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
            wake_main_thread();
        }
    }));
    Ok(true)
}

fn show_dialog_on_java_main_thread(
    app: &AndroidApp,
    names: &[String],
    ids: &[String],
) -> Result<(), String> {
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr() as _) };
    vm.attach_current_thread(|mut env| {
        env.with_local_frame(16 + names.len() as i32 * 2, |env| {
            let activity_raw = app.activity_as_ptr() as jobject;
            let activity = unsafe { env.as_cast_raw::<Global<JObject<'static>>>(&activity_raw)? };

            let string_class = env.find_class("java/lang/String")?;

            let names_array: JObjectArray<'_> =
                env.new_object_array(names.len() as _, &string_class, JObject::null())?;
            for (i, name) in names.iter().enumerate() {
                let jname = env.new_string(name.as_str())?;
                env.set_object_array_element(&names_array, i as _, jname)?;
            }

            let ids_array: JObjectArray<'_> =
                env.new_object_array(ids.len() as _, &string_class, JObject::null())?;
            for (i, id) in ids.iter().enumerate() {
                let jid = env.new_string(id.as_str())?;
                env.set_object_array_element(&ids_array, i as _, jid)?;
            }

            env.call_method(
                activity.as_ref(),
                jni_str!("showRomLibraryDialog"),
                jni_sig!("([Ljava/lang/String;[Ljava/lang/String;)V"),
                &[
                    JValue::Object(names_array.as_ref()),
                    JValue::Object(ids_array.as_ref()),
                ],
            )?;
            Ok(())
        })
    })
    .map_err(|error| format!("failed to show Android ROM library dialog: {error:?}"))
}

fn publish_result(result: LibraryDialogResult) {
    *LIBRARY_RESULT
        .lock()
        .expect("library result mutex poisoned") = Some(result);
    LIBRARY_REQUEST_IN_FLIGHT.store(false, Ordering::Release);
    wake_main_thread();
}

fn wake_main_thread() {
    if let Some(waker) = LIBRARY_WAKER
        .lock()
        .expect("library waker mutex poisoned")
        .clone()
    {
        waker.wake();
    }
}

/// JNI callback invoked by `MainActivity.onRomLibrarySelected`.
///
/// * `id == null`             → dialog was dismissed
/// * `id == IMPORT_ACTION_ID` → user wants to import a new ROM
/// * otherwise               → user selected the library entry with that id
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onRomLibrarySelected(
    env: jni::Env<'_>,
    _activity: JObject<'_>,
    id: JString<'_>,
) {
    let result = if id.is_null() {
        Ok(LibraryDialogResult::Dismissed)
    } else {
        id.try_to_string(&env).map(|s| {
            if s == IMPORT_ACTION_ID {
                LibraryDialogResult::ImportRequested
            } else {
                LibraryDialogResult::Selected(s)
            }
        })
    };
    match result {
        Ok(result) => publish_result(result),
        Err(error) => {
            log::error!("failed to decode Android ROM library dialog result: {error:?}");
            publish_result(LibraryDialogResult::Dismissed);
        }
    }
}
