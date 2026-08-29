use std::{collections::VecDeque, sync::Mutex};

use nerust_core_traits::peripheral::{AccelerationSample, AccelerometerInputHandle};
use winit::event_loop::EventLoopProxy;

use super::messages::MenuAction;

pub(super) struct AndroidBridgeState {
    pub(super) event_loop_proxy: Option<EventLoopProxy<()>>,
    pub(super) menu_actions: VecDeque<MenuAction>,
    accelerometer: Option<AccelerometerInputHandle>,
    exit_requested: bool,
}

impl AndroidBridgeState {
    const fn empty() -> Self {
        Self {
            event_loop_proxy: None,
            menu_actions: VecDeque::new(),
            accelerometer: None,
            exit_requested: false,
        }
    }
}

static ANDROID_BRIDGE: Mutex<AndroidBridgeState> = Mutex::new(AndroidBridgeState::empty());

pub(super) fn bind_event_loop(event_loop_proxy: EventLoopProxy<()>) {
    with_state(|state| {
        state.event_loop_proxy = Some(event_loop_proxy);
        state.menu_actions.clear();
        state.exit_requested = false;
    });
}

pub(super) fn reset_transient() {
    with_state(|state| {
        state.menu_actions.clear();
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
    operation(
        &mut ANDROID_BRIDGE
            .lock()
            .expect("Android bridge mutex poisoned"),
    )
}

pub(super) fn wake() {
    let event_loop_proxy = with_state(|state| state.event_loop_proxy.clone());
    if let Some(event_loop_proxy) = event_loop_proxy {
        let _ = event_loop_proxy.send_event(());
    }
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
