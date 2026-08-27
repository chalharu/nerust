use std::{collections::VecDeque, sync::Mutex};

use winit::event_loop::EventLoopProxy;

use super::messages::MenuAction;

pub(super) struct AndroidBridgeState {
    pub(super) event_loop_proxy: Option<EventLoopProxy<()>>,
    pub(super) menu_actions: VecDeque<MenuAction>,
}

impl AndroidBridgeState {
    const fn empty() -> Self {
        Self {
            event_loop_proxy: None,
            menu_actions: VecDeque::new(),
        }
    }
}

static ANDROID_BRIDGE: Mutex<AndroidBridgeState> = Mutex::new(AndroidBridgeState::empty());

pub(super) fn bind_event_loop(event_loop_proxy: EventLoopProxy<()>) {
    with_state(|state| {
        state.event_loop_proxy = Some(event_loop_proxy);
        state.menu_actions.clear();
    });
}

pub(super) fn reset_transient() {
    with_state(|state| {
        state.menu_actions.clear();
    });
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
