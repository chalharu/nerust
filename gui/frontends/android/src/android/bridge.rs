use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use jni::objects::JIntArray;
use nerust_core_traits::peripheral::{AccelerationSample, AccelerometerInputHandle};
use winit::{event_loop::EventLoopProxy, platform::android::activity::AndroidApp};

use super::settings_model::{AndroidSettings, SettingDef};
use crate::bridge_events::FrontendEvent;

/// Worker-thread completion produced by the single picker fetch worker.
/// Bytes ride along so the event thread never touches I/O.
pub(super) enum RomPickerResult {
    Cancelled,
    Selected(String),
    TreeSelected(String),
    /// Document buffer fetched off-thread; digest rides along.
    /// `restore_hidden_state` replays a lifecycle restore instead of a fresh load.
    RomBytes {
        uri: String,
        file_name: String,
        bytes: Vec<u8>,
        digest: [u8; 32],
        restore_hidden_state: bool,
    },
    /// Worker-thread library completion: same dispatch shape as `RomBytes`.
    LibraryBytes {
        id: String,
        bytes: Vec<u8>,
        digest: [u8; 32],
    },
    /// Worker-thread failure: surfaces on the event thread through the
    /// normal picker-error path (error dialog queue).
    Failed(String),
    /// Lifecycle-restore fetch failure: document URI gone, hidden state kept.
    RestoreDocumentFailed(String),
    /// Lifecycle-restore fetch failure: library entry gone, restore dropped.
    RestoreLibraryFailed(String),
}

/// Picker slot in the unified bridge vault: result plus the open gate.
pub(super) struct PickerSlot {
    pub(super) result: Option<RomPickerResult>,
    pub(super) in_flight: bool,
}

impl PickerSlot {
    pub(super) const fn empty() -> Self {
        Self {
            result: None,
            in_flight: false,
        }
    }
}

/// Keyed dialog outcome produced by the settings bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SettingsDialogResult {
    Dismissed,
    Applied(BTreeMap<String, usize>),
}

/// Single-flight dialog state; lives in the unified bridge vault.
pub(super) struct SettingsSlot {
    pub(super) result: Option<SettingsDialogResult>,
    pub(super) cache: Option<AndroidSettings>,
    pub(super) next_request_id: u64,
    pub(super) pending_request_id: Option<u64>,
    pub(super) defs_cache: Option<(u64, Arc<Vec<SettingDef>>)>,
}

impl SettingsSlot {
    pub(super) const fn empty() -> Self {
        Self {
            result: None,
            cache: None,
            next_request_id: 1,
            pending_request_id: None,
            defs_cache: None,
        }
    }
}

/// Canonical JNI bridge table: one name+signature pair per method.
/// Call sites reference these consts; literals live here only.
pub(super) mod methods {
    use jni::{jni_sig, jni_str, signature::MethodSignature, strings::JNIStr};

    /// One bridge method: compile-checked name plus signature.
    pub(crate) struct BridgeMethod {
        pub name: &'static JNIStr,
        pub sig: MethodSignature<'static, 'static>,
    }

    macro_rules! bridge_method {
        ($ident:ident, $name:literal, $sig:literal) => {
            pub(crate) const $ident: BridgeMethod = BridgeMethod {
                name: jni_str!($name),
                sig: jni_sig!($sig),
            };
        };
    }

    bridge_method!(START_ROM_PICKER, "startRomPicker", "()V");
    bridge_method!(START_DIRECTORY_PICKER, "startDirectoryPicker", "()V");
    bridge_method!(
        SHOW_SETTINGS_DIALOG,
        "showSettingsDialog",
        "(Ljava/lang/String;)V"
    );
    bridge_method!(
        SHOW_ERROR_DIALOG,
        "showErrorDialog",
        "(Ljava/lang/String;)V"
    );
    bridge_method!(
        NOTIFY_ROM_LOADED,
        "notifyRomLoaded",
        "(Ljava/lang/String;)V"
    );
    bridge_method!(FINISH_ACTIVITY, "finish", "()V");
    bridge_method!(
        CONFIGURE_CONTROLS_OVERLAY,
        "configureControlsOverlay",
        "(Ljava/lang/String;IZLjava/lang/String;Ljava/lang/String;)V"
    );
    bridge_method!(PERFORM_CONTROL_HAPTIC, "performControlHaptic", "()V");
    bridge_method!(UPDATE_FLOATING_DPAD, "updateFloatingDpad", "(ZFFFF)V");
    bridge_method!(
        CONFIGURE_CARTRIDGE_PERIPHERALS,
        "configureCartridgePeripherals",
        "(ZZILjava/lang/String;)V"
    );
    bridge_method!(SET_CARTRIDGE_RUMBLE, "setCartridgeRumble", "(I)V");
    bridge_method!(APPLY_SCREEN_ORIENTATION, "applyScreenOrientation", "(I)V");
    bridge_method!(
        DELETE_SAF_FILE,
        "deleteSafFile",
        "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"
    );
    bridge_method!(
        LIST_SAF_FILES_WITH_SIZES,
        "listSafFilesWithSizes",
        "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;"
    );
    bridge_method!(
        WRITE_SAF_FILE_CHUNK_BYTES,
        "writeSafFileChunkBytes",
        "(Ljava/lang/String;Ljava/lang/String;[BZ)Z"
    );
    bridge_method!(
        OPEN_SAF_READ_SESSION,
        "openSafReadSession",
        "(Ljava/lang/String;Ljava/lang/String;)J"
    );
    bridge_method!(READ_SAF_READ_SESSION, "readSafReadSession", "(JI)[B");
    bridge_method!(CLOSE_SAF_READ_SESSION, "closeSafReadSession", "(J)V");
}

/// Call one tabled bridge method on the activity.
/// Lifetimes stay independent: the handle outlives the argument frame.
pub(super) fn call_bridge_method<'env, 'obj, 'arg>(
    env: &mut jni::Env<'env>,
    activity: &jni::objects::JObject<'obj>,
    method: &methods::BridgeMethod,
    args: &[jni::objects::JValue<'arg>],
) -> jni::errors::Result<jni::objects::JValueOwned<'env>> {
    env.call_method(activity.as_ref(), method.name, method.sig.clone(), args)
}

pub(super) struct AndroidBridgeState {
    event_loop_proxy: Option<EventLoopProxy<()>>,
    /// Priority lane: dialog acks jump ahead of the menu burst.
    priority_events: VecDeque<FrontendEvent>,
    normal_events: VecDeque<FrontendEvent>,
    exit_requested: bool,
    /// Last UI-thread insets push, [top, right, bottom, left] in physical px.
    /// The render thread reads the cache only and never blocks on JNI.
    insets_cache: Option<[i32; 4]>,
}

impl AndroidBridgeState {
    const fn empty() -> Self {
        Self {
            event_loop_proxy: None,
            priority_events: VecDeque::new(),
            normal_events: VecDeque::new(),
            exit_requested: false,
            insets_cache: None,
        }
    }
}

/// Single bridge vault: queue, picker, and settings share one mutex.
/// One lock kills cross-thread order; same-thread nesting still panics.
struct BridgeVault {
    queue: AndroidBridgeState,
    picker: PickerSlot,
    settings: SettingsSlot,
}

impl BridgeVault {
    const fn empty() -> Self {
        Self {
            queue: AndroidBridgeState::empty(),
            picker: PickerSlot::empty(),
            settings: SettingsSlot::empty(),
        }
    }
}

static BRIDGE: Mutex<BridgeVault> = Mutex::new(BridgeVault::empty());

/// Monotonic insets generation: readers skip the lock when unchanged.
static INSETS_VERSION: AtomicU64 = AtomicU64::new(0);

/// Accelerometer leaf lock: sensor pushes never contend with the vault.
/// Never take the vault while holding it.
static ACCEL_HANDLE: Mutex<Option<AccelerometerInputHandle>> = Mutex::new(None);

fn lock_accel() -> std::sync::MutexGuard<'static, Option<AccelerometerInputHandle>> {
    ACCEL_HANDLE.lock().unwrap_or_else(|poison| {
        log::error!("accelerometer handle mutex poisoned; recovering state");
        poison.into_inner()
    })
}

fn with_vault<T>(operation: impl FnOnce(&mut BridgeVault) -> T) -> T {
    crate::reentrancy::with_lock(&BRIDGE, operation)
}

pub(super) fn with_queue<T>(operation: impl FnOnce(&mut AndroidBridgeState) -> T) -> T {
    with_vault(|vault| operation(&mut vault.queue))
}

pub(super) fn with_picker<T>(operation: impl FnOnce(&mut PickerSlot) -> T) -> T {
    with_vault(|vault| operation(&mut vault.picker))
}

pub(super) fn with_settings<T>(operation: impl FnOnce(&mut SettingsSlot) -> T) -> T {
    with_vault(|vault| operation(&mut vault.settings))
}

/// Set the event proxy once at construction; rebinds only warn.
/// Queue state is never cleared here; call `clear_queues` explicitly.
pub(super) fn bind_event_loop(event_loop_proxy: EventLoopProxy<()>) {
    with_queue(|state| {
        if state.event_loop_proxy.is_some() {
            log::warn!("Android event proxy already bound; ignoring rebind");
            return;
        }
        state.event_loop_proxy = Some(event_loop_proxy);
    });
}

/// Explicit queue reset: suspend and construction paths call this.
pub(super) fn clear_queues() {
    with_queue(|state| {
        state.priority_events.clear();
        state.normal_events.clear();
        state.exit_requested = false;
    });
}

pub(super) fn reset_transient() {
    clear_queues();
    *lock_accel() = None;
}

pub(super) fn bind_accelerometer(handle: Option<AccelerometerInputHandle>) {
    *lock_accel() = handle;
}

pub(super) fn request_exit() {
    with_queue(|state| state.exit_requested = true);
    wake();
}

pub(super) fn take_exit_request() -> bool {
    with_queue(|state| std::mem::take(&mut state.exit_requested))
}

pub(super) fn wake() {
    let event_loop_proxy = with_queue(|state| state.event_loop_proxy.clone());
    if let Some(event_loop_proxy) = event_loop_proxy {
        let _ = event_loop_proxy.send_event(());
    }
}

/// Batch publish: one wake per JNI batch, not per event.
/// Dialog dismissals take the priority lane; the rest keeps arrival order.
pub(super) fn publish_all(events: &[FrontendEvent]) {
    if events.is_empty() {
        return;
    }
    if log::log_enabled!(log::Level::Debug) {
        for event in events {
            log::debug!("Android bridge: published event: {event:?}");
        }
    }
    with_queue(|state| {
        for event in events {
            if matches!(event, FrontendEvent::ErrorDialogDismissed) {
                state.priority_events.push_back(*event);
            } else {
                state.normal_events.push_back(*event);
            }
        }
    });
    wake();
}

/// Drain into a caller-owned buffer: priority lane first, then the normal lane.
/// Idle ticks reuse capacity instead of allocating per pass.
pub(super) fn take_events_into(out: &mut Vec<FrontendEvent>) {
    with_queue(|state| {
        out.extend(state.priority_events.drain(..));
        out.extend(state.normal_events.drain(..));
    });
}

/// UI-thread insets push from the Kotlin layout listener.
/// Bumps the generation so readers skip the lock when unchanged.
pub(super) fn push_system_insets(values: [i32; 4]) {
    with_queue(|state| {
        state.insets_cache = Some(values);
    });
    INSETS_VERSION.fetch_add(1, Ordering::Release);
}

/// Current insets generation for lock-free change checks.
pub(super) fn insets_version() -> u64 {
    INSETS_VERSION.load(Ordering::Acquire)
}

pub(super) fn cached_system_insets() -> Option<[i32; 4]> {
    with_queue(|state| state.insets_cache)
}

/// Run a JNI closure, always clearing a pending Java exception.
/// A pending exception always forces an error, even on Rust `Ok`.
pub(super) fn with_java_env<T>(
    vm: &jni::JavaVM,
    operation: impl FnOnce(&mut jni::Env<'_>) -> jni::errors::Result<T>,
) -> Result<T, jni::errors::Error> {
    vm.attach_current_thread(|env| {
        let result = operation(env);
        if env.exception_check() {
            env.exception_clear();
            log::debug!("cleared pending Java exception after JNI call");
            // The JVM already failed the call; the return value is garbage.
            // Always force an error here.
            return Err(result.err().unwrap_or(jni::errors::Error::JavaException));
        }
        result
    })
}

/// Run a JNI callback with the same exception-clear semantics as [`with_java_env`].
/// Every native callback routes through here; a pending exception never wedges the thread.
pub(super) fn with_callback_env<T>(
    env: &mut jni::EnvUnowned<'_>,
    operation: impl FnOnce(&mut jni::Env<'_>) -> jni::errors::Result<T>,
) -> jni::Outcome<T, jni::errors::Error> {
    let outcome = env.with_env(|env| {
        let result = operation(env);
        if env.exception_check() {
            env.exception_clear();
            log::debug!("cleared pending Java exception after JNI callback");
            return Err(result.err().unwrap_or(jni::errors::Error::JavaException));
        }
        result
    });
    let outcome = outcome.into_outcome();
    if let jni::Outcome::Err(ref error) = outcome {
        log::warn!("JNI callback failed: {error:?}");
    }
    outcome
}

/// Post to the Java main thread: fire-and-forget, never blocks the caller.
/// `on_failure` runs on the Java thread when the JNI call fails.
pub(super) fn post_to_activity(
    app: &AndroidApp,
    label: &'static str,
    operation: impl FnOnce(&mut jni::Env<'_>, jni::objects::JObject<'_>) -> jni::errors::Result<()>
    + Send
    + 'static,
    on_failure: Option<Box<dyn FnOnce() + Send + 'static>>,
) {
    let callback_app = app.clone();
    let owned = app.clone();
    owned.run_on_java_main_thread(Box::new(move || {
        let vm = unsafe { jni::JavaVM::from_raw(callback_app.vm_as_ptr() as _) };
        let result = with_java_env(&vm, |env| {
            let activity_raw = callback_app.activity_as_ptr() as jni::sys::jobject;
            let activity = unsafe { jni::objects::JObject::from_raw(env, activity_raw) };
            operation(env, activity)
        });
        if let Err(error) = result {
            log::warn!("{label} failed: {error:?}");
            if let Some(on_failure) = on_failure {
                on_failure();
            }
        }
    }));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onActivityDestroyed(
    _env: jni::EnvUnowned<'_>,
    _activity: jni::objects::JObject<'_>,
) {
    request_exit();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onCartridgeAcceleration(
    _env: jni::EnvUnowned<'_>,
    _activity: jni::objects::JObject<'_>,
    x_g: jni::sys::jfloat,
    y_g: jni::sys::jfloat,
) {
    let handle = lock_accel().clone();
    if let Some(handle) = handle {
        handle.publish(AccelerationSample::new(x_g, y_g));
    }
}

/// UI-thread push for the insets cache, called on every layout pass.
/// The render thread reads the cached generation without blocking on JNI.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onSystemInsetsChanged(
    mut env: jni::EnvUnowned<'_>,
    _activity: jni::objects::JObject<'_>,
    insets: JIntArray<'_>,
) {
    match with_callback_env(&mut env, |env| -> jni::errors::Result<[i32; 4]> {
        let mut out = [0i32; 4];
        insets.get_region(env, 0, &mut out)?;
        Ok(out)
    }) {
        jni::Outcome::Ok(values) => push_system_insets(values),
        jni::Outcome::Err(_) => {}
        jni::Outcome::Panic(_) => {
            log::error!("system insets callback panicked");
        }
    }
}
