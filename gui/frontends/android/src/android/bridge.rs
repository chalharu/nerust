use std::{collections::VecDeque, sync::Mutex};

use winit::event_loop::EventLoopProxy;

use super::{
    menu::MenuAction,
    picker::RomPickerResult,
    settings::{AndroidSettings, SettingsDialogResult},
};

pub(super) struct AndroidBridgeState {
    pub(super) event_loop_proxy: Option<EventLoopProxy<()>>,
    pub(super) picker_result: Option<RomPickerResult>,
    pub(super) picker_in_flight: bool,
    pub(super) menu_actions: VecDeque<MenuAction>,
    pub(super) settings_result: Option<SettingsDialogResult>,
    pub(super) settings_cache: Option<AndroidSettings>,
    pub(super) next_settings_request_id: u64,
    pub(super) pending_settings_request_id: Option<u64>,
}

impl AndroidBridgeState {
    const fn empty() -> Self {
        Self {
            event_loop_proxy: None,
            picker_result: None,
            picker_in_flight: false,
            menu_actions: VecDeque::new(),
            settings_result: None,
            settings_cache: None,
            next_settings_request_id: 1,
            pending_settings_request_id: None,
        }
    }
}

static ANDROID_BRIDGE: Mutex<AndroidBridgeState> = Mutex::new(AndroidBridgeState::empty());

pub(super) fn bind_event_loop(event_loop_proxy: EventLoopProxy<()>) {
    with_state(|state| {
        state.event_loop_proxy = Some(event_loop_proxy);
        state.picker_result = None;
        state.picker_in_flight = false;
        state.menu_actions.clear();
        state.settings_result = None;
        state.pending_settings_request_id = None;
    });
}

pub(super) fn reset_transient() {
    with_state(|state| {
        state.picker_result = None;
        state.picker_in_flight = false;
        state.menu_actions.clear();
        state.settings_result = None;
        state.pending_settings_request_id = None;
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
