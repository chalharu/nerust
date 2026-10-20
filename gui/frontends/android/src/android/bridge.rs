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

static ANDROID_BRIDGE: Mutex<AndroidBridgeState> = Mutex::new(AndroidBridgeState::empty());

/// Accelerometer handle on its own lock: per-sample sensor pushes
/// never contend with the event-queue mutex, which stays queue-only.
static ACCEL_HANDLE: Mutex<Option<AccelerometerInputHandle>> = Mutex::new(None);

fn lock_accel() -> std::sync::MutexGuard<'static, Option<AccelerometerInputHandle>> {
    ACCEL_HANDLE.lock().unwrap_or_else(|poison| {
        log::error!("accelerometer handle mutex poisoned; recovering state");
        poison.into_inner()
    })
}

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
    });
    *lock_accel() = None;
}

pub(super) fn bind_accelerometer(handle: Option<AccelerometerInputHandle>) {
    *lock_accel() = handle;
}

pub(super) fn request_exit() {
    with_state(|state| state.exit_requested = true);
    wake();
}

pub(super) fn take_exit_request() -> bool {
    with_state(|state| std::mem::take(&mut state.exit_requested))
}

pub(super) fn with_state<T>(operation: impl FnOnce(&mut AndroidBridgeState) -> T) -> T {
    // Lock order for the three bridge mutexes: never nest `with_state`;
    // wake the looper after releasing. Poison recovers state, never wedges.
    let _guard = lock_reentrancy_guard();
    let mut guard = ANDROID_BRIDGE.lock().unwrap_or_else(|poison| {
        log::error!("Android bridge mutex poisoned; recovering state");
        poison.into_inner()
    });
    operation(&mut guard)
}

/// Re-entrancy guard for the three bridge mutexes.
/// `with_state` must never nest; every profile panics, never silently nests.
pub(super) use crate::reentrancy::lock_reentrancy_guard;

pub(super) fn wake() {
    let event_loop_proxy = with_state(|state| state.event_loop_proxy.clone());
    if let Some(event_loop_proxy) = event_loop_proxy {
        let _ = event_loop_proxy.send_event(());
    }
}

/// Batch publish: one wake per JNI batch, not per event.
/// Dialog dismissals take the priority lane; everything else keeps arrival order.
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

/// Drain into a caller-owned buffer: priority lane first, then the normal lane.
/// Idle ticks reuse capacity instead of allocating per pass.
pub(super) fn take_events_into(out: &mut Vec<FrontendEvent>) {
    with_state(|state| {
        out.extend(state.priority_events.drain(..));
        out.extend(state.normal_events.drain(..));
    });
}

/// UI-thread insets push from the Kotlin layout listener.
/// The render thread reads the cached value without another JNI round-trip.
pub(super) fn push_system_insets(values: [i32; 4]) {
    with_state(|state| {
        state.insets_cache = Some(values);
    });
}

pub(super) fn cached_system_insets() -> Option<[i32; 4]> {
    with_state(|state| state.insets_cache)
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

/// Marshal to the Java main thread, run the operation, log failures with the label.
/// JNI failures are reported to the session by the caller.
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

/// Fallible variant: `on_failure` runs on the Java main thread on JNI failure.
/// Logging matches `call_on_activity`.
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
    let handle = lock_accel().clone();
    if let Some(handle) = handle {
        handle.publish(AccelerationSample::new(x_g, y_g));
    }
}

/// UI-thread push for the insets cache, called on every layout pass.
/// The render thread reads the cache without a blocking JNI query.
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
