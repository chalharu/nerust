use std::{collections::VecDeque, sync::Mutex};

use jni::objects::JIntArray;
use nerust_core_traits::peripheral::{AccelerationSample, AccelerometerInputHandle};
use winit::{event_loop::EventLoopProxy, platform::android::activity::AndroidApp};

use super::messages::FrontendEvent;

pub(super) struct AndroidBridgeState {
    pub(super) event_loop_proxy: Option<EventLoopProxy<()>>,
    /// Priority lane: dialog acks (dismissals) jump ahead of the input
    /// burst so a modal gate release never waits behind menu taps.
    priority_events: VecDeque<FrontendEvent>,
    normal_events: VecDeque<FrontendEvent>,
    accelerometer: Option<AccelerometerInputHandle>,
    exit_requested: bool,
    /// Last UI-thread insets push in physical px, [top, right, bottom,
    /// left]. The render thread reads the cache only and never blocks on
    /// JNI; the direct query covers the first frame.
    insets_cache: Option<[i32; 4]>,
}

impl AndroidBridgeState {
    const fn empty() -> Self {
        Self {
            event_loop_proxy: None,
            priority_events: VecDeque::new(),
            normal_events: VecDeque::new(),
            accelerometer: None,
            exit_requested: false,
            insets_cache: None,
        }
    }
}

static ANDROID_BRIDGE: Mutex<AndroidBridgeState> = Mutex::new(AndroidBridgeState::empty());

pub(super) fn bind_event_loop(event_loop_proxy: EventLoopProxy<()>) {
    with_state(|state| {
        state.event_loop_proxy = Some(event_loop_proxy);
        state.priority_events.clear();
        state.normal_events.clear();
        state.exit_requested = false;
    });
}

pub(super) fn reset_transient() {
    with_state(|state| {
        state.priority_events.clear();
        state.normal_events.clear();
        state.accelerometer = None;
    });
}

pub(super) fn bind_accelerometer(handle: Option<AccelerometerInputHandle>) {
    with_state(|state| state.accelerometer = handle);
}

pub(super) fn request_exit() {
    with_state(|state| state.exit_requested = true);
    wake();
}

pub(super) fn take_exit_request() -> bool {
    with_state(|state| std::mem::take(&mut state.exit_requested))
}

pub(super) fn with_state<T>(operation: impl FnOnce(&mut AndroidBridgeState) -> T) -> T {
    // Lock order for the three bridge mutexes: never nest `with_state`
    // calls; wake the looper after releasing. A panicking JNI callback
    // must not wedge the process on the next lock: recover the state
    // and keep serving events.
    let mut guard = ANDROID_BRIDGE.lock().unwrap_or_else(|poison| {
        log::error!("Android bridge mutex poisoned; recovering state");
        poison.into_inner()
    });
    operation(&mut guard)
}

pub(super) fn wake() {
    let event_loop_proxy = with_state(|state| state.event_loop_proxy.clone());
    if let Some(event_loop_proxy) = event_loop_proxy {
        let _ = event_loop_proxy.send_event(());
    }
}

/// Batch publish: one wake per JNI batch, not per event. Bursts from
/// rapid menu taps share the wakeup instead of spamming the looper.
/// Dialog dismissals take the priority lane so a modal gate release never
/// waits behind an input burst; everything else keeps arrival order.
pub(super) fn publish_all(events: &[FrontendEvent]) {
    if events.is_empty() {
        return;
    }
    with_state(|state| {
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

/// Drain into a caller-owned buffer: priority lane first, then the normal
/// lane. Idle ticks reuse capacity instead of allocating per pass, and the
/// two-lane order removes the old per-tick stable sort.
pub(super) fn take_events_into(out: &mut Vec<FrontendEvent>) {
    with_state(|state| {
        out.extend(state.priority_events.drain(..));
        out.extend(state.normal_events.drain(..));
    });
}

/// UI-thread insets push, invoked from the Kotlin layout listener via
/// `onSystemInsetsChanged`. The render thread reads the cached value
/// without another JNI round-trip.
pub(super) fn push_system_insets(values: [i32; 4]) {
    with_state(|state| {
        state.insets_cache = Some(values);
    });
}

pub(super) fn cached_system_insets() -> Option<[i32; 4]> {
    with_state(|state| state.insets_cache)
}

/// Run a JNI closure, always clearing a pending Java exception.
///
/// Without this, a `NoSuchMethodError` (e.g. after a Kotlin rename)
/// stays pending on the thread and poisons every later JNI call; the
/// Rust `Err` alone never clears it. The check runs on success too: a
/// JVM-thrown exception with a Rust `Ok` still leaves garbage (often
/// null) in the return value, so a pending exception always forces
/// an error.
pub(super) fn with_java_env<T>(
    vm: &jni::JavaVM,
    operation: impl FnOnce(&mut jni::Env<'_>) -> jni::errors::Result<T>,
) -> Result<T, jni::errors::Error> {
    vm.attach_current_thread(|env| {
        let result = operation(env);
        if env.exception_check() {
            env.exception_clear();
            log::debug!("cleared pending Java exception after JNI call");
            // A pending exception means the JVM already failed the call,
            // even when the Rust closure reported Ok: the return value is
            // garbage (often null), so always force an error here.
            return Err(result.err().unwrap_or(jni::errors::Error::JavaException));
        }
        result
    })
}

/// Single helper for the fire-and-forget activity calls that used to be
/// six copy-paste `run_on_java_main_thread` + `with_java_env` blocks.
/// Marshals to the Java main thread, resolves the activity, runs the
/// operation, and logs failures with the call-site label. JNI failures
/// are reported to the session by the caller (it owns the session);
/// this helper only guarantees the label in logcat.
pub(super) fn call_on_activity(
    app: &AndroidApp,
    label: &'static str,
    operation: impl FnOnce(&mut jni::Env<'_>, jni::objects::JObject<'_>) -> jni::errors::Result<()>
    + Send
    + 'static,
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
        }
    }));
}

/// Fallible variant: `on_failure` runs on the Java main thread when the
/// JNI call fails (e.g. synthesize a dialog dismiss so a dead bridge
/// cannot wedge the gate). Logging matches `call_on_activity`.
pub(super) fn call_on_activity_or(
    app: &AndroidApp,
    label: &'static str,
    operation: impl FnOnce(&mut jni::Env<'_>, jni::objects::JObject<'_>) -> jni::errors::Result<()>
    + Send
    + 'static,
    on_failure: impl FnOnce() + Send + 'static,
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
            on_failure();
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
    let handle = with_state(|state| state.accelerometer.clone());
    if let Some(handle) = handle {
        handle.publish(AccelerationSample::new(x_g, y_g));
    }
}

/// UI-thread push for the insets cache: the Kotlin layout listener calls
/// this on every layout pass so the render thread can read the cache
/// without a blocking JNI query.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chalharu_nerust_MainActivity_onSystemInsetsChanged(
    mut env: jni::EnvUnowned<'_>,
    _activity: jni::objects::JObject<'_>,
    insets: JIntArray<'_>,
) {
    match env
        .with_env(|env| -> jni::errors::Result<[i32; 4]> {
            let mut out = [0i32; 4];
            insets.get_region(env, 0, &mut out)?;
            Ok(out)
        })
        .into_outcome()
    {
        jni::Outcome::Ok(values) => push_system_insets(values),
        jni::Outcome::Err(error) => {
            log::warn!("failed to decode system insets push: {error:?}");
        }
        jni::Outcome::Panic(_) => {
            log::error!("system insets callback panicked");
        }
    }
}
