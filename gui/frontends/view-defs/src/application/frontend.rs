//! Framework app implementation: [`FrontendApp`] over the owned session.
//!
//! Native frontends hold the app behind `Box<dyn FrontendApp>` (see
//! [`Application`](wakui::app::boundary::Application)) and never name
//! shell, runtime, or ViewModel types. All ViewModel contact lives here.

use std::{path::Path, time::Instant};

use wakui::{
    app::binding::PickerSink as _,
    chrome::viewport::ViewportRegion,
    flow::loop_driver::WakeNeed,
    scene::control::{ControlEffect, ControlEvent, ControlId, ControlScreen},
};
use wakui::{
    app::{
        boundary::{FrontendApp, WidgetCloseOutcome},
        roles::{
            AuxPorts, ChromePorts, ContentPorts, DialogPorts, FocusPorts, LifecyclePorts,
            PollPorts, ViewportPorts,
        },
    },
    chrome::dialog::{DialogTurn, ModalAdvance, ModalToken},
    input::session::SessionError,
};
use wakui::{
    chrome::aux::{AuxId, AuxRegistryPort, OpenRequest},
    chrome::mailbox::WindowMailbox,
    chrome::menu::{FileDialogTitles, MenuCapabilities, Menubar},
    chrome::viewport::{ViewportImage, ViewportPresent},
    chrome::window::{Window, WindowChromeSync, WindowEffects},
    input::focus::FocusEvent,
};

use super::ApplicationHost;
use crate::{
    debugger::{
        DebugBridge, DebugHandle, DebugInputs, DebugSnapshot, DebuggerDriver, DebuggerEffects,
        DebuggerEvent, DebuggerLiveEffects, DebuggerScope,
    },
    focus::AuxKind,
    widget_session::WidgetSession,
};

/// Owned debugger session: shared bridge plus its drain driver and open count.
/// The PPU viewer session lives here so debugger-local natives (gtk) can
/// project a stable generic ControlScreen without a global widget id.
pub(crate) struct DebugSession {
    pub(crate) bridge: DebugBridge,
    pub(crate) driver: DebuggerDriver,
    pub(crate) refs: usize,
}

impl ApplicationHost {
    /// Stored ROM loader for boundary loads. Set once by the bootstrap.
    pub fn set_rom_loader(&mut self, loader: Box<dyn nerust_gui_shell::load::RomLoader>) {
        self.loader = Some(loader);
    }

    fn load_rom_inner(&mut self, path: &Path) -> Result<(), String> {
        let Some(loader) = self.loader.as_mut() else {
            return Err("no ROM loader installed".to_string());
        };
        // Reborrow through a raw pointer: the loader lives in `self` while
        // the session borrow below also derives from `self`.
        let loader_ptr: *mut dyn nerust_gui_shell::load::RomLoader = loader.as_mut();
        // SAFETY: the pointee outlives this call and nothing else aliases it.
        let result = self.load_path(unsafe { &mut *loader_ptr }, path);
        result.map_err(|error| error.to_string())
    }

    fn debug_session(&self, handle: DebugHandle) -> Option<&DebugSession> {
        self.debug_sessions.get(&handle.0)
    }

    pub(crate) fn open_debug_inner(&mut self) -> DebugHandle {
        // Single bridge shared by every native viewer; count opens so the
        // last close drops the session.
        if let Some(id) = self.debug_sessions.iter_mut().next().map(|(id, session)| {
            session.refs += 1;
            *id
        }) {
            return DebugHandle(id);
        }
        let bridge = self.new_debug_bridge();
        bridge.bind_epoch(self.session.debug_epoch());
        let id = self.debug_next;
        self.debug_next = self
            .debug_next
            .checked_add(1)
            .expect("debug handle exhausted");
        self.debug_sessions.insert(
            id,
            DebugSession {
                bridge,
                driver: DebuggerDriver::new(),
                refs: 1,
            },
        );
        let handle = DebugHandle(id);
        // Initial Pause+Refresh so an empty viewer never shows stale chrome.
        self.debug_open_refresh_inner(handle);
        handle
    }

    pub(crate) fn close_debug_inner(&mut self, handle: DebugHandle) {
        let drop_now = self
            .debug_sessions
            .get_mut(&handle.0)
            .map(|session| {
                session.refs = session.refs.saturating_sub(1);
                session.refs == 0
            })
            .unwrap_or(false);
        if drop_now {
            self.debug_sessions.remove(&handle.0);
            self.debug.close();
        }
    }

    fn rebuild_debug_bridge(&mut self, handle: DebugHandle) {
        if !self.debug_sessions.contains_key(&handle.0) {
            return;
        }
        let bridge = self.new_debug_bridge();
        bridge.bind_epoch(self.session.debug_epoch());
        if let Some(session) = self.debug_sessions.get_mut(&handle.0) {
            session.bridge = bridge;
        }
    }

    pub(crate) fn debug_snapshot_inner(&self, handle: DebugHandle) -> Option<DebugSnapshot> {
        let session = self.debug_session(handle)?;
        let chrome = self.debugger_chrome();
        let rows = session.bridge.debugger_rows();
        let view = session.bridge.view_snapshot();
        let nav = view.nav;
        Some(DebugSnapshot {
            chrome,
            rows,
            inputs: DebugInputs {
                mem_input: nav.mem_input().to_string(),
                dis_input: nav.dis_input().to_string(),
                follow_pc: nav.follow_pc(),
                back_len: nav.back_len(),
                dis_addr: nav.dis_addr(),
                mem_addr: Some(nav.mem_addr()),
                bm_input: view.bm_input,
                bookmarks: view.bookmarks,
                freeze_input: view.freeze_input,
                edit_input: view.edit_input,
                reg_input: view.reg_input,
                watch: view.watch,
                watch_value: view.display.watch_value,
                freeze: view.freeze,
            },
            selected: session.bridge.selected(),
            reg_target: session.bridge.reg_target(),
            space_name: session.bridge.selected_name().unwrap_or_default(),
            has_spaces: !session.bridge.spaces().is_empty(),
        })
    }

    fn debug_gesture_inner(
        &mut self,
        handle: DebugHandle,
        inputs: Vec<DebuggerEvent>,
        event: DebuggerEvent,
    ) -> DebuggerEffects {
        let need_images = self.images_needed();
        if self.debug_reset_if_replaced_inner(handle) {
            return DebuggerEffects {
                epoch_reset: true,
                ..Default::default()
            };
        }
        let Some(session) = self.debug_sessions.get_mut(&handle.0) else {
            return DebuggerEffects::default();
        };
        session
            .driver
            .dispatch(&self.session, &session.bridge, inputs, event, need_images)
    }

    fn debug_refresh_inner(&mut self, handle: DebugHandle, scope: DebuggerScope) {
        let need_images = self.images_needed();
        if self.debug_reset_if_replaced_inner(handle) {
            return;
        }
        let Some(session) = self.debug_sessions.get_mut(&handle.0) else {
            return;
        };
        session
            .driver
            .refresh(&self.session, &session.bridge, scope, need_images);
    }

    fn debug_open_refresh_inner(&mut self, handle: DebugHandle) {
        let need_images = self.images_needed();
        if self.debug_reset_if_replaced_inner(handle) {
            return;
        }
        let Some(session) = self.debug_sessions.get_mut(&handle.0) else {
            return;
        };
        session
            .driver
            .open_refresh(&self.session, &session.bridge, need_images);
    }

    fn debug_poll_inner(&mut self, handle: DebugHandle) -> WakeNeed {
        let ppu_open = self.images_needed();
        if !self.debug_sessions.contains_key(&handle.0) {
            return WakeNeed::default();
        }
        // Disjoint field borrows: the bridge is read through an immutable
        // borrow while the runtime, session, and policy borrow mutably.
        let pending = {
            let sessions = &mut self.debug_sessions;
            let debug = &mut self.debug;
            let session = &mut self.session;
            let pause = &mut self.pause_policy;
            let Some(entry) = sessions.get_mut(&handle.0) else {
                return WakeNeed::default();
            };
            debug.poll(session, pause, &entry.bridge)
        };
        self.debug_wake(true, ppu_open, pending)
    }

    fn debug_poll_live_inner(&mut self, handle: DebugHandle, visible: bool) -> DebuggerLiveEffects {
        let need_images = self.images_needed();
        let Some(session) = self.debug_sessions.get_mut(&handle.0) else {
            return DebuggerLiveEffects::default();
        };
        session
            .driver
            .live_tick(&self.session, &session.bridge, visible, need_images)
    }

    fn debug_live_tick_inner(&mut self, handle: DebugHandle) {
        let ppu_open = self.images_needed();
        let Some(entry) = self.debug_sessions.get(&handle.0) else {
            return;
        };
        let bridge = &entry.bridge;
        let now = std::time::Instant::now();
        self.debug
            .pump_live(&self.session, true, ppu_open, bridge, now);
    }

    fn debug_reset_if_replaced_inner(&mut self, handle: DebugHandle) -> bool {
        let replaced = self
            .debug_sessions
            .get_mut(&handle.0)
            .is_some_and(|session| session.driver.reset_replaced_epoch(&self.session));
        if replaced {
            self.rebuild_debug_bridge(handle);
        }
        replaced
    }

    /// View-side debugger gesture entry (tests, examples). Native frontends
    /// route `widget_dispatch` instead and never name these types.
    pub fn debug_gesture(
        &mut self,
        handle: DebugHandle,
        inputs: Vec<DebuggerEvent>,
        event: DebuggerEvent,
    ) -> DebuggerEffects {
        self.debug_gesture_inner(handle, inputs, event)
    }

    /// View-side drain check (tests, examples).
    pub fn debug_drain_pending(&self, handle: DebugHandle) -> bool {
        self.debug_session(handle)
            .is_some_and(|session| session.driver.pending())
    }

    /// View-side handle lookup (tests, examples). Native frontends use
    /// the generic `polled_widget`/`poll_widget` ports instead.
    pub fn debug_handle_for(&self, id: AuxId) -> Option<DebugHandle> {
        self.widget_session(id)
            .and_then(|session| session.debug_handle())
    }

    /// Widget session owned by id, if a prepared window holds one.
    fn widget_session(&self, id: AuxId) -> Option<&WidgetSession> {
        self.widget_sessions.get(&id)
    }

    /// Whether any prepared widget currently needs debug images (PPU open).
    fn images_needed(&self) -> bool {
        self.widget_sessions
            .values()
            .any(|session| session.kind() == crate::focus::AuxKind::Ppu)
    }

    fn widget_session_mut(&mut self, id: AuxId) -> Option<&mut WidgetSession> {
        self.widget_sessions.get_mut(&id)
    }

    fn consume_close(
        &mut self,
        id: AuxId,
        pending: Option<crate::widget_session::WindowSubmission>,
    ) -> WindowEffects {
        // Mirror the application close path without naming native registries.
        let kind = self
            .widget_session(id)
            .map(|session| session.kind())
            .unwrap_or(crate::focus::AuxKind::History);
        let _ = kind;
        if let Some(submission) = pending {
            self.consume_submission(submission).unwrap_or_default()
        } else {
            WindowEffects::default()
        }
    }

    fn consume_submission(
        &mut self,
        submission: crate::widget_session::WindowSubmission,
    ) -> Result<WindowEffects, SessionError> {
        self.apply_window_submission_inner(submission)
    }

    fn apply_window_submission_inner(
        &mut self,
        submission: crate::widget_session::WindowSubmission,
    ) -> Result<WindowEffects, SessionError> {
        use crate::widget_session::Pending;
        match submission.take() {
            Some(Pending::Settings(snapshot)) => match self.apply_settings(*snapshot) {
                Ok(result) => {
                    let fullscreen = result.fullscreen_default_changed.then_some(
                        self.settings_snapshot()
                            .local
                            .video
                            .window
                            .fullscreen_default,
                    );
                    let resize = if result.scaling_changed {
                        nerust_gui_shell::settings::scaling_factor(
                            self.settings_snapshot().local.video.window.scaling,
                        )
                        .map(|scale| {
                            let size = self.session.window_size();
                            (
                                (size.width * scale as f32).round().max(1.0) as u32,
                                (size.height * scale as f32).round().max(1.0) as u32,
                            )
                        })
                    } else {
                        None
                    };
                    self.note_redraw();
                    Ok(WindowEffects {
                        rebuild_renderer: result.renderer_needs_rebuild,
                        fullscreen,
                        resize,
                        redraw: true,
                        refresh_chrome: true,
                    })
                }
                Err(error) => {
                    submission.mailbox.apply_failed(error.to_string());
                    Err(error)
                }
            },
            Some(Pending::ClearHistory) => {
                self.clear_history();
                self.invalidate_history();
                submission.mailbox.invalidate();
                self.note_redraw();
                Ok(WindowEffects {
                    redraw: true,
                    ..Default::default()
                })
            }
            None => Ok(WindowEffects::default()),
        }
    }
}

impl LifecyclePorts for ApplicationHost {
    fn load_file(&mut self, path: &Path) -> Result<(), String> {
        let result = self.load_rom_inner(path);
        if result.is_ok() {
            ApplicationHost::resume_session(self);
            // Reassert suspension holds after replacement: a focus or modal
            // hold active across the load suspends the new content again.
            // Flavors never touch this; the policy lives view-side.
            ApplicationHost::reconcile_pause(self);
        }
        result
    }

    fn unload(&mut self) {
        let _ = self.session.unload();
    }

    fn begin_close(&mut self) -> bool {
        ApplicationHost::begin_close(self)
    }

    fn finish_close(&mut self) {
        ApplicationHost::finish_close(self);
    }

    fn close_exited(&self) -> bool {
        ApplicationHost::close_exited(self)
    }

    fn fatal_message(&self) -> Option<String> {
        ApplicationHost::fatal_message(self).map(str::to_string)
    }

    fn fail_window(&mut self, detail: String) {
        ApplicationHost::fail_window(self, detail);
    }

    fn resume_content(&mut self) {
        ApplicationHost::resume_session(self);
    }
}

impl ContentPorts for ApplicationHost {
    fn content_loaded(&self) -> bool {
        self.session.tick_metrics().loaded
    }

    fn content_suspended(&self) -> bool {
        self.session.tick_metrics().paused
    }

    fn clear_input(&mut self) {
        ApplicationHost::clear_keys(self);
    }

    fn update_window_size(&mut self, width: u32, height: u32) -> Result<(), String> {
        ApplicationHost::update_window_size(self, width, height).map_err(|error| error.to_string())
    }

    fn dispatch(
        &mut self,
        event: wakui::input::session::SessionEvent,
    ) -> Result<wakui::input::session::SessionOutcome, SessionError> {
        crate::session::dispatch(
            &mut self.session,
            &mut self.pause_policy,
            &self.menu_commands,
            event,
        )
    }
}

impl ChromePorts for ApplicationHost {
    fn dialog_titles(&self) -> FileDialogTitles {
        ApplicationHost::dialog_titles(self)
    }

    fn file_dialog_filters(&self) -> Vec<(String, Vec<String>)> {
        ApplicationHost::rom_dialog_filters(self)
    }

    fn menubar_update(&mut self, capabilities: MenuCapabilities) -> Option<Menubar> {
        let ctx = self.frame_cache.frame_ctx(&self.session);
        ApplicationHost::menubar_update(self, &ctx, capabilities).map(|update| update.menubar)
    }

    fn main_window(&self) -> &Window {
        ApplicationHost::main_window(self)
    }

    fn window_usable(&self) -> bool {
        ApplicationHost::window_usable(self)
    }

    fn title_update(&mut self) -> Option<String> {
        let tick = self.session.tick_metrics();
        ApplicationHost::title_update(self, &tick)
    }

    fn observe_fullscreen(&mut self, native: bool) -> bool {
        ApplicationHost::observe_fullscreen(self, native)
    }

    fn fullscreen_request(&mut self, native: bool) -> Option<bool> {
        ApplicationHost::fullscreen_request(self, native)
    }

    fn persist_fullscreen_default(&mut self, fullscreen: bool) -> Result<bool, String> {
        ApplicationHost::persist_fullscreen_default(self, fullscreen)
    }

    fn window_size(&self) -> (f32, f32) {
        ApplicationHost::window_size(self)
    }

    fn window_scale_factor(&self) -> Option<u32> {
        ApplicationHost::window_scale_factor(self)
    }

    fn window_chrome_sync(&mut self, native_fullscreen: bool) -> WindowChromeSync {
        ApplicationHost::window_chrome_sync(self, native_fullscreen)
    }

    fn remembered_window_size(&self) -> Option<(u32, u32)> {
        ApplicationHost::remembered_window_size(self)
    }
}

impl FocusPorts for ApplicationHost {
    fn focus_event(&mut self, event: FocusEvent) {
        ApplicationHost::focus_event(self, event)
    }

    fn main_active(&self) -> bool {
        ApplicationHost::main_active(self)
    }

    fn note_main_arrival(&mut self) -> bool {
        ApplicationHost::note_main_arrival(self)
    }

    fn note_main_departure(&mut self) {
        ApplicationHost::note_main_departure(self);
    }

    fn drive_aux_focus(&mut self, id: AuxId, focused: bool) -> bool {
        ApplicationHost::drive_aux_focus(self, id, focused)
    }
}

impl AuxPorts<AuxKind> for ApplicationHost {
    fn reserve_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<AuxKind>,
        kind: AuxKind,
    ) -> Result<Option<OpenRequest>, String> {
        if self.close_exited() {
            return Ok(None);
        }
        let request = registry.request_open(kind, kind.title());
        if kind == AuxKind::Settings
            && let OpenRequest::Create(id) = request
            && !ApplicationHost::note_settings_opened(self)
        {
            registry.cancel_open(id);
            return Ok(None);
        }
        Ok(Some(request))
    }

    fn cancel_widget(&mut self, registry: &mut dyn AuxRegistryPort<AuxKind>, id: AuxId) -> bool {
        let kind = registry.kind_of_id(id);
        if !registry.cancel_open(id) {
            return false;
        }
        if kind == Some(AuxKind::Settings) {
            ApplicationHost::note_settings_dismissed(self);
        }
        true
    }

    fn prepare_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<AuxKind>,
        id: AuxId,
    ) -> Result<bool, String> {
        use crate::widget_session::WidgetSession as ViewSession;
        if self.close_exited() || registry.mailbox(id).is_some() {
            return Ok(false);
        }
        let session = match registry.kind_of_id(id) {
            Some(AuxKind::Settings) => {
                let registry_snapshot = self.system_registry();
                if let Some(duplicate) =
                    crate::settings::find_duplicate_system_id(&registry_snapshot)
                {
                    self.cancel_widget(registry, id);
                    return Err(format!("duplicate SystemId '{duplicate}'"));
                }
                let audio = self.audio_registry();
                let controller = crate::settings::SettingsController::new(
                    self.settings_snapshot(),
                    registry_snapshot,
                    audio,
                );
                ViewSession::from_settings(controller)
            }
            Some(AuxKind::History) if self.widget_session(id).is_none() => {
                let language = self.settings_snapshot().shared.general.language;
                let lines = self
                    .session
                    .notification_history_lines_cached(language)
                    .into();
                ViewSession::history(lines, language)
            }
            Some(AuxKind::Ppu) => {
                let language = self.settings_snapshot().shared.general.language;
                let chrome = crate::ppu::ppu_chrome(language);
                let (pictures, panels, generation) = self.ppu_contents();
                ViewSession::ppu(chrome, pictures, panels, generation, language)
            }
            Some(AuxKind::Debug) => {
                let handle = self.open_debug_inner();
                // Initial Pause+Refresh so an empty viewer never shows stale chrome.
                self.debug_open_refresh_inner(handle);
                let snapshot = self.debug_snapshot_inner(handle);
                let mut session = ViewSession::debug(handle);
                session.refresh_debug(snapshot);
                session
            }
            _ => return Ok(false),
        };
        if let Some(slot) = registry.window_mut(id) {
            *slot = session.window(id);
        }
        let mailbox = session.mailbox();
        self.widget_sessions.insert(id, session);
        registry.set_mailbox(id, mailbox);
        if registry.kind_of_id(id) == Some(AuxKind::History) {
            self.invalidate_history();
        }
        Ok(true)
    }

    fn widget_screen(&self, id: AuxId) -> Option<ControlScreen> {
        self.widget_session(id).map(|session| session.screen())
    }

    fn widget_window(&self, id: AuxId) -> Option<Window> {
        self.widget_session(id).map(|session| session.window(id))
    }

    fn widget_mailbox(&self, id: AuxId) -> Option<WindowMailbox> {
        self.widget_session(id).map(|session| session.mailbox())
    }

    fn widget_dispatch(&mut self, id: AuxId, event: ControlEvent) -> Vec<ControlEffect> {
        // Debugger gestures need the bridge handle: map first, then run the
        // existing debug driver per gesture (PPU opens route through it too).
        if self
            .widget_session(id)
            .is_some_and(|session| session.kind() == AuxKind::Debug)
        {
            let gestures = self
                .widget_session(id)
                .and_then(|session| session.debug_events(&event))
                .unwrap_or_default();
            // Screen gating mirrors WidgetSession::dispatch (accepts check).
            let accepted = self
                .widget_session(id)
                .and_then(|session| {
                    let screen = session.screen();
                    let target = event.id.or(screen.capture.target)?;
                    screen
                        .control(target)
                        .is_some_and(|control| control.accepts(&event.input))
                        .then_some(())
                })
                .is_some();
            if accepted {
                let handle = self
                    .widget_session(id)
                    .and_then(|session| session.debug_handle());
                if let Some(handle) = handle {
                    for gesture in gestures {
                        let effects = self.debug_gesture(handle, vec![], gesture);
                        if effects.open_ppu {
                            self.request_open(AuxKind::Ppu);
                        }
                    }
                    self.widget_refresh(id);
                }
            }
            return Vec::new();
        }
        self.widget_session_mut(id)
            .map(|session| session.dispatch(event))
            .unwrap_or_default()
    }

    fn widget_submit_picker(
        &mut self,
        id: AuxId,
        target: ControlId,
        request: wakui::scene::control::Id,
        payload: Option<std::path::PathBuf>,
    ) {
        if let Some(session) = self.widget_session_mut(id) {
            session.submit(target, request, payload);
        }
    }

    fn widget_refresh(&mut self, id: AuxId) {
        let kind = self.widget_session(id).map(|session| session.kind());
        match kind {
            Some(AuxKind::History) => {
                let language = self.settings_snapshot().shared.general.language;
                let lines = self
                    .session
                    .notification_history_lines_cached(language)
                    .into();
                if let Some(session) = self.widget_session_mut(id) {
                    session.refresh_history(lines, language);
                }
            }
            Some(AuxKind::Ppu) => {
                let language = self.settings_snapshot().shared.general.language;
                let chrome = crate::ppu::ppu_chrome(language);
                let (pictures, panels, generation) = self.ppu_contents();
                if let Some(session) = self.widget_session_mut(id) {
                    session.refresh_ppu(chrome, pictures, panels, generation, language);
                }
            }
            Some(AuxKind::Debug) => {
                let handle = self
                    .widget_session(id)
                    .and_then(|session| session.debug_handle());
                if let Some(handle) = handle {
                    let snapshot = self.debug_snapshot_inner(handle);
                    if let Some(session) = self.widget_session_mut(id) {
                        session.refresh_debug(snapshot);
                    }
                }
            }
            _ => {}
        }
    }

    fn close_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<AuxKind>,
        id: AuxId,
        close: bool,
    ) -> Option<WidgetCloseOutcome> {
        let kind = registry.kind_of_id(id)?;
        let pending = registry
            .mailbox(id)
            .and_then(|mailbox| crate::widget_session::take_submission(&mailbox));
        let has_pending = pending.is_some();
        if let Some(submission) = &pending
            && !registry
                .mailbox(id)
                .is_some_and(|mailbox| mailbox.same(&submission.mailbox))
        {
            return None;
        }
        let native = if close {
            let debug = self
                .widget_sessions
                .get(&id)
                .and_then(|session| session.debug_handle());
            self.widget_sessions.remove(&id);
            if let Some(handle) = debug {
                self.close_debug_inner(handle);
            }
            registry.remove_native(id)
        } else {
            None
        };
        let effects = if self.close_exited() {
            WindowEffects::default()
        } else if close {
            self.consume_close(id, pending)
        } else if let Some(submission) = pending {
            self.consume_submission(submission).unwrap_or_default()
        } else {
            WindowEffects::default()
        };
        if close {
            if kind == AuxKind::Settings {
                self.settings_closed();
            }
            if kind == AuxKind::History {
                self.invalidate_history();
            }
        }
        let changed = close || has_pending;
        Some(WidgetCloseOutcome {
            native,
            effects: WindowEffects {
                refresh_chrome: changed,
                ..effects
            },
        })
    }

    fn report_host_error(&mut self, detail: String) {
        ApplicationHost::report_host_error(self, detail);
    }

    fn modal_guard_active(&self) -> bool {
        ApplicationHost::modal_guard_active(self)
    }

    fn take_open_request(&mut self, kind: AuxKind) -> bool {
        self.debug.schedule_mut().take_open_request(kind)
    }

    fn drain_open_requests(&mut self) -> Vec<AuxKind> {
        [
            AuxKind::Debug,
            AuxKind::Ppu,
            AuxKind::History,
            AuxKind::Settings,
        ]
        .into_iter()
        .filter(|kind| self.debug.schedule_mut().take_open_request(*kind))
        .collect()
    }

    fn request_open(&mut self, kind: AuxKind) {
        self.debug.schedule_mut().request_open(kind);
    }

    fn window_pos(&self, kind: AuxKind) -> Option<(i32, i32)> {
        self.debug.schedule().window_pos(kind)
    }

    fn remember_window_pos(&mut self, kind: AuxKind, x: i32, y: i32) {
        self.debug.schedule_mut().remember_window_pos(kind, x, y);
    }
}

impl PollPorts for ApplicationHost {
    fn close_background_flights(&mut self) {
        ApplicationHost::close_debug_flights(self);
    }

    fn polled_widget(&self) -> Option<AuxId> {
        self.widget_sessions
            .iter()
            .find_map(|(id, session)| (session.kind() == AuxKind::Debug).then_some(*id))
    }

    fn poll_widget(&mut self, id: AuxId) -> WakeNeed {
        let Some(handle) = self
            .widget_session(id)
            .and_then(|session| session.debug_handle())
        else {
            return WakeNeed::default();
        };
        if self.debug_reset_if_replaced_inner(handle) {
            return self.debug_poll_inner(handle);
        }
        let wake = self.debug_poll_inner(handle);
        if ApplicationHost::chrome_drain_due(self, true) {
            self.debug_refresh_inner(handle, DebuggerScope::All);
            let second = self.debug_poll_inner(handle);
            self.widget_refresh(id);
            return wake.fold(second);
        }
        self.widget_refresh(id);
        wake
    }

    fn tick_widget(&mut self, id: AuxId) {
        if !(self.content_loaded() && !self.content_suspended()) {
            return;
        }
        let Some(handle) = self
            .widget_session(id)
            .and_then(|session| session.debug_handle())
        else {
            return;
        };
        self.debug_live_tick_inner(handle);
        let _ = self.debug_poll_live_inner(handle, true);
        self.widget_refresh(id);
    }

    fn live_armed(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.debug.schedule().live_armed()
    }

    fn register_tick_thread(&mut self, thread: std::thread::Thread) {
        self.debug.schedule().register_tick_thread(thread);
    }

    fn next_live_tick(&self) -> Option<std::time::Instant> {
        self.debug.schedule().next_live_tick()
    }

    fn live_due(&mut self, now: std::time::Instant) -> bool {
        self.debug.schedule_mut().live_due(now)
    }
}

impl DialogPorts for ApplicationHost {
    fn pump_dialog(&mut self, now: Instant) -> DialogTurn {
        let ctx = self.frame_cache.frame_ctx(&self.session);
        let turn = ApplicationHost::pump_osd(self, &ctx, now);
        DialogTurn {
            modal: turn.modal,
            wake: turn.wake,
        }
    }

    fn attach_dialog_receiver(
        &mut self,
        token: ModalToken,
        receiver: std::sync::mpsc::Receiver<()>,
    ) -> bool {
        ApplicationHost::attach_osd_modal_receiver(self, token, receiver)
    }

    fn acknowledge_dialog(&mut self, token: ModalToken) -> ModalAdvance {
        ApplicationHost::acknowledge_osd_modal(self, token)
    }

    fn dialog_pending(&self) -> bool {
        ApplicationHost::osd_modal_pending(self)
    }

    fn dialog_flight_pending(&self) -> bool {
        ApplicationHost::osd_flight_pending(self)
    }

    fn dialog_queue_len(&self) -> usize {
        ApplicationHost::osd_pending(self)
    }

    fn dialog_wakeup(&self, now: Instant) -> Option<Instant> {
        ApplicationHost::osd_wakeup(self, now)
    }

    fn note_redraw(&mut self) {
        ApplicationHost::note_redraw(self);
    }

    fn redraw_needed(&self) -> bool {
        ApplicationHost::redraw_needed(self)
    }

    fn invalidate_overlay(&mut self) {
        ApplicationHost::invalidate_osd(self);
    }
}

impl ViewportPorts for ApplicationHost {
    fn swap_frame_buffer(&mut self) {
        ApplicationHost::swap_frame_buffer(self);
    }

    fn viewport_sync(&mut self, region: ViewportRegion, force: bool) -> Result<bool, String> {
        ApplicationHost::viewport_sync(self, region, force)
    }

    fn viewport_present(&mut self, width: u32, height: u32, client_height: u32) -> ViewportPresent {
        ApplicationHost::viewport_present(self, width, height, client_height)
    }

    fn viewport_capture(&mut self, width: u32, height: u32) -> Option<ViewportImage> {
        ApplicationHost::viewport_capture(self, width, height)
    }

    fn sync_overlay_viewport(&mut self, width: u32, height: u32, client_height: u32) {
        let ctx = self.frame_cache.frame_ctx(&self.session);
        let update = ApplicationHost::prepare_osd_viewport(
            self,
            width,
            height,
            client_height,
            &ctx,
            Instant::now(),
        );
        let generation = ApplicationHost::viewport_generation(self);
        ApplicationHost::viewport_apply_osd(self, update, generation);
    }

    fn viewport_drop(&mut self) {
        ApplicationHost::viewport_drop(self);
    }

    fn viewport_attached(&self) -> bool {
        ApplicationHost::viewport_attached(self)
    }

    fn viewport_generation(&self) -> u64 {
        ApplicationHost::viewport_generation(self)
    }

    fn render_ready(&self) -> bool {
        ApplicationHost::render_ready(self)
    }

    fn render_failed(&mut self, detail: String) {
        ApplicationHost::render_failed(self, detail);
    }

    fn render_retry_deadline(&self) -> Option<Instant> {
        ApplicationHost::render_retry_deadline(self)
    }

    fn take_render_retry(&mut self, now: Instant) -> bool {
        ApplicationHost::take_render_retry(self, now)
    }

    fn render_succeeded(&mut self) {
        ApplicationHost::render_succeeded(self);
    }
}

impl FrontendApp<AuxKind> for ApplicationHost {}
