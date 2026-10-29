//! Owned application state. Native adapters own resources, not control truth.

mod auxiliary;
mod chrome;
mod debug;
mod frontend;
mod osd;
#[cfg(test)]
mod tests;
mod window;

pub use crate::menu::MenuSig;
pub use auxiliary::WidgetWindowClose;
use debug::DebugRuntime;
pub use wakui::menu::FileDialogTitles;
pub use window::MainState;

use std::{path::Path, time::Instant};

use crate::pause::{PauseOwner, PausePolicy};
use crate::session::pause_owners;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::{
    load::{RomLoader, RomLoaderError},
    session::{
        SessionError, SessionHandle,
        access::{SessionControl, SettingsResult},
        osd::TickMetrics,
    },
};
use nerust_gui_viewmodel::debugger::tile_hover_text;
use wakui::retry::RenderRetry;

use crate::focus::FocusTracker;
use wakui::focus::FocusEvent;
use wakui::menu::{MenuCapabilities, Menubar};
use wakui::session::{SessionEvent, SessionOutcome};
use wakui::window::Window;

use crate::{
    debugger::BridgeLock,
    frame::{FrameCache, FrameCtx},
    osd::OsdNotifications,
};
use wakui::dialog::{ModalAdvance, ModalToken};

/// One session and one source of truth for its application-level policies.
/// Native adapters should use policy-aware host methods for commands and pause
/// transitions. Raw session access remains available for shell integration, but
/// must not bypass pause ownership or debugger epoch guards.
pub struct ApplicationHost {
    pub(crate) session: SessionHandle,
    pause_policy: PausePolicy,
    focus: FocusTracker,
    frame_cache: FrameCache,
    render_retry: RenderRetry,
    debug: DebugRuntime,
    debug_sessions: std::collections::HashMap<u64, frontend::DebugSession>,
    debug_next: u64,
    widget_sessions:
        std::collections::HashMap<wakui::aux::AuxId, crate::widget_session::WidgetSession>,
    loader: Option<Box<dyn nerust_gui_shell::load::RomLoader>>,
    osd: osd::OsdRuntime,
    chrome: chrome::ChromeSync,
    menubar_chrome: chrome::ChromeSync,
    main_window: Window,
    menu_capabilities: Option<MenuCapabilities>,
    fullscreen: window::FullscreenSync,
    pub(crate) viewport: crate::viewport::RenderDriver,
    close: MainState,
    flushed: bool,
    shell: nerust_gui_runtime::shell::NativeShellState,
    history_key: wakui::cache::Versioned<(u64, nerust_settings_core::model::language::AppLanguage)>,
    menu_command_ids: wakui::control::IdRegistry<crate::commands::MenuCommandId>,
    menu_commands: crate::commands::CommandTable,
}

impl ApplicationHost {
    pub fn new(session: SessionHandle) -> Self {
        Self {
            session,
            pause_policy: Default::default(),
            focus: FocusTracker::new(),
            frame_cache: FrameCache::new(),
            render_retry: Default::default(),
            debug: DebugRuntime::new(),
            debug_sessions: std::collections::HashMap::new(),
            debug_next: 0,
            widget_sessions: std::collections::HashMap::new(),
            loader: None,
            osd: Default::default(),
            chrome: Default::default(),
            menubar_chrome: Default::default(),
            main_window: Window::main("nerust"),
            menu_capabilities: None,
            fullscreen: Default::default(),
            viewport: crate::viewport::RenderDriver::new(),
            close: MainState::new(),
            flushed: false,
            shell: Default::default(),
            history_key: Default::default(),
            menu_command_ids: Default::default(),
            menu_commands: Default::default(),
        }
    }

    /// Shared shell access, including existing internally mutable debug probes.
    /// Application-level commands should use this host's policy-aware methods.
    pub fn session(&self) -> &SessionHandle {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut SessionHandle {
        &mut self.session
    }

    /// Factories backing the owned session. Native adapters pass these by
    /// inference where a host entry point still needs them; the shell and
    /// audio types never cross the boundary by name.
    pub fn system_registry(&self) -> std::sync::Arc<nerust_gui_shell::registry::SystemRegistry> {
        self.session.system_registry().clone()
    }

    /// Audio backends backing the owned session, exposed for the same reason.
    pub fn audio_registry(
        &self,
    ) -> std::sync::Arc<nerust_core_traits::audio::AudioBackendRegistry> {
        self.session.audio_registry().clone()
    }

    pub fn dispatch(
        &mut self,
        event: SessionEvent,
    ) -> Result<SessionOutcome, wakui::session::SessionError> {
        crate::session::dispatch(
            &mut self.session,
            &mut self.pause_policy,
            &self.menu_commands,
            event,
        )
    }

    /// Typed command entry for menu-resolved domain commands.
    pub fn dispatch_session_command(
        &mut self,
        command: nerust_gui_shell::session::commands::SessionCommand,
    ) -> Result<SessionOutcome, wakui::session::SessionError> {
        crate::session::dispatch_command(&mut self.session, &mut self.pause_policy, command)
    }

    /// Latest menu resolution table for opaque native activations.
    pub fn menu_commands(&self) -> &crate::commands::CommandTable {
        &self.menu_commands
    }

    pub fn frame_ctx(&mut self) -> FrameCtx {
        self.frame_cache.frame_ctx(&self.session)
    }
    pub fn tick(&self) -> TickMetrics {
        self.session.tick_metrics()
    }
    pub fn settings_snapshot(&self) -> &SettingsSnapshot {
        self.session.settings_snapshot()
    }

    /// Pre-localized dialog titles for native file/error dialogs.
    pub fn dialog_titles(&self) -> FileDialogTitles {
        crate::menu::dialog_titles(self.settings_snapshot().shared.general.language)
    }

    /// Pre-localized debugger window title. Frontends render it verbatim
    /// instead of calling into the i18n catalog themselves.
    pub fn debugger_window_title(&self) -> String {
        self.debugger_chrome().debugger
    }

    /// Pre-localized debugger chrome for the debugger window.
    pub fn debugger_chrome(&self) -> crate::debugger::DebuggerChrome {
        crate::debugger::debugger_chrome(self.settings_snapshot().shared.general.language)
    }

    /// New debug bridge with the session language baked in. Frontends
    /// supply spaces and the anchor; the language never crosses the boundary.
    /// New debug bridge with session inputs baked in: spaces and the anchor
    /// come from the live session, the display starts empty, and the language
    /// comes from the snapshot. Frontends name no view-data types to open one.
    /// Mirrors `DebuggerModel::from_session` for the split-lock bridge.
    pub fn new_debug_bridge(&self) -> crate::debugger::DebugBridge {
        let spaces: Vec<_> = self
            .session
            .debug_spaces()
            .iter()
            .map(|info| (info.id, info.name.to_string(), *info.range.start()))
            .collect();
        if spaces.is_empty() {
            log::warn!("debugger: no spaces (no core)");
        }
        let mem_addr = spaces.first().map(|(_, _, start)| *start).unwrap_or(0);
        crate::debugger::DebugBridge::new(
            Default::default(),
            spaces,
            mem_addr,
            self.settings_snapshot().shared.general.language,
        )
    }

    /// Localized PPU tile hover line for raw view coordinates.
    /// Geometry stays view-side; the language comes from the snapshot.
    pub fn ppu_hover_text(&self, label: &str, width: u32, height: u32, x: f64, y: f64) -> String {
        tile_hover_text(
            self.settings_snapshot().shared.general.language,
            label,
            width,
            height,
            x,
            y,
        )
    }
    pub fn render_profile(&self) -> Option<nerust_render_traits::VideoRenderProfile> {
        self.session.render_profile().cloned()
    }
    pub fn debug_epoch(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.session.debug_epoch()
    }
    pub fn swap_frame_buffer(&mut self) {
        self.session.swap_frame_buffer();
    }
    pub fn frame_buffer(&self) -> Option<&nerust_render_traits::FrameBuffer> {
        self.session.frame_buffer()
    }
    pub fn report_host_error(&mut self, detail: String) {
        self.session.report_external_error(
            nerust_gui_shell::session::notification::FailureOrigin::Host,
            detail,
        );
    }
    pub fn clear_history(&mut self) {
        self.session.clear_notification_history();
    }
    pub fn cached_title(&mut self, tick: &TickMetrics) -> String {
        self.session.window_title_cached(tick).to_owned()
    }
    pub fn clear_keys(&mut self) {
        let _ = self.dispatch(SessionEvent::ClearInput);
    }

    pub fn load_path(
        &mut self,
        loader: &mut dyn RomLoader,
        path: &Path,
    ) -> Result<(), RomLoaderError> {
        self.reconcile_pause();
        let result = loader.load_rom(path, &mut self.session);
        self.reconcile_pause();
        result
    }

    pub fn set_pause_owner(&mut self, owner: PauseOwner, active: bool) {
        crate::session::set_pause_owner(&mut self.session, &mut self.pause_policy, owner, active);
    }
    pub fn owns_pause(&self, owner: PauseOwner) -> bool {
        self.pause_policy.owns(owner)
    }

    /// Whether a modal dialog currently gates chrome and commands.
    /// Flavors never query kinds; they forward events and the view side
    /// enforces this guard in `handle_event` and menu builds.
    pub fn modal_guard_active(&self) -> bool {
        self.owns_pause(pause_owners::SETTINGS)
    }

    /// View-side driver entry for the [`wakui::event::EventHandler`] impl:
    /// opaque inputs in, effects and settings transitions out.
    pub(crate) fn dispatch_driver_event(
        &mut self,
        event: SessionEvent,
    ) -> Result<crate::session::DriverOutcome, wakui::session::SessionError> {
        crate::session::dispatch_driver(
            &mut self.session,
            &mut self.pause_policy,
            &self.menu_commands,
            event,
        )
    }
    pub fn pause_held(&self) -> bool {
        self.pause_policy.paused()
    }
    pub fn reconcile_pause(&mut self) {
        crate::session::reconcile_pause(&mut self.session, &self.pause_policy);
    }

    pub fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<nerust_gui_shell::session::access::SettingsResult, wakui::session::SessionError>
    {
        crate::session::apply_settings(&mut self.session, &self.pause_policy, settings)
            .map_err(|error| wakui::session::SessionError(error.to_string()))
    }
    pub fn release_focus_hold(&mut self) {
        self.set_pause_owner(pause_owners::FOCUS, false);
    }
    pub fn resume_session(&mut self) {
        use nerust_gui_shell::session::access::PauseControl;
        self.resume();
    }

    /// Focus notification entry: departures arm, arrivals cancel and release
    /// the focus hold when held, burst end commits. Resume reads the hold
    /// itself, never a framework back-channel: the framework pauses nothing,
    /// suspension is content behavior owned here.
    pub fn focus_event(&mut self, event: FocusEvent) {
        match event {
            FocusEvent::Main(true) => {
                self.focus.main(true);
                self.release_focus_hold();
            }
            FocusEvent::Main(false) => self.focus.main(false),
            FocusEvent::Aux { resume, focused } => {
                self.focus.aux(focused);
                if focused && resume {
                    self.release_focus_hold();
                }
            }
            FocusEvent::BurstEnd => {
                if self.focus.take_armed() && self.tick().loaded {
                    self.set_pause_owner(pause_owners::FOCUS, true);
                }
            }
        }
    }
    pub fn drive_aux_focus(&mut self, id: wakui::aux::AuxId, focused: bool) -> bool {
        let resume = self
            .widget_sessions
            .get(&id)
            .map(|session| session.kind().resume_on_arrival())
            .unwrap_or(false);
        self.focus_event(FocusEvent::Aux { resume, focused });
        focused
    }
    pub fn tick_focus_sync(&mut self) {
        self.focus_event(FocusEvent::BurstEnd);
    }
    pub fn main_active(&self) -> bool {
        self.focus.main_active()
    }
    pub fn note_main_arrival(&mut self) -> bool {
        let held = self.owns_pause(pause_owners::FOCUS);
        self.focus_event(FocusEvent::Main(true));
        held
    }
    pub fn note_main_departure(&mut self) {
        self.focus_event(FocusEvent::Main(false));
    }

    pub fn render_ready(&self) -> bool {
        self.render_retry.ready(Instant::now())
    }
    pub fn render_failed(&mut self, detail: String) {
        if self.render_retry.failed(Instant::now()) {
            self.report_host_error(detail);
        }
    }
    pub fn render_succeeded(&mut self) {
        self.render_retry.succeeded();
    }
    pub fn take_render_retry(&mut self, now: Instant) -> bool {
        self.render_retry.take_due(now)
    }
    pub fn render_retry_deadline(&self) -> Option<Instant> {
        self.render_retry.deadline()
    }

    pub fn pump_notifications(
        &mut self,
        notifications: &mut OsdNotifications,
        ctx: &FrameCtx,
        now: Instant,
    ) -> nerust_gui_shell::session::osd::PumpReport {
        if self.close_exited() {
            return nerust_gui_shell::session::osd::PumpReport {
                changed: false,
                modal_queued: false,
            };
        }
        let report = notifications.pump(&mut self.session, ctx, now);
        let pending = notifications.modal_pending();
        if self.owns_pause(pause_owners::MODAL) != pending {
            self.set_pause_owner(pause_owners::MODAL, pending);
        }
        report
    }

    /// Validate callback identity and commit pause changes before native presentation.
    pub fn acknowledge_modal(
        &mut self,
        notifications: &mut OsdNotifications,
        token: ModalToken,
    ) -> ModalAdvance {
        if self.close_exited() {
            return ModalAdvance::Ignored;
        }
        let advance = notifications.acknowledge_modal(token);
        self.commit_modal_advance(&advance);
        advance
    }

    /// Thread ACK/disconnection uses the same policy path as widget callbacks.
    pub fn poll_modal_ack(&mut self, notifications: &mut OsdNotifications) -> Option<ModalAdvance> {
        if self.close_exited() {
            return None;
        }
        let advance = notifications.poll_modal_ack()?;
        self.commit_modal_advance(&advance);
        Some(advance)
    }

    fn commit_modal_advance(&mut self, advance: &ModalAdvance) {
        if matches!(advance, ModalAdvance::Drained) {
            self.set_pause_owner(pause_owners::MODAL, false);
            self.note_redraw();
        }
    }

    pub fn history_update(
        &mut self,
        language: nerust_settings_core::model::language::AppLanguage,
    ) -> Option<std::sync::Arc<[nerust_gui_shell::session::osd::HistoryLine]>> {
        let key = (self.session.notification_history_seq(), language);
        self.history_key.check(&key).then(|| {
            self.session
                .notification_history_lines_cached(language)
                .into()
        })
    }
    pub fn invalidate_history(&mut self) {
        self.history_key.clear();
    }
    pub fn history_key(&self) -> Option<(u64, nerust_settings_core::model::language::AppLanguage)> {
        self.history_key.get().copied()
    }
    pub fn note_redraw(&mut self) {
        self.shell.needs_redraw = true;
    }
    pub fn redraw_needed(&self) -> bool {
        self.shell.needs_redraw
    }
    pub fn frame_presented(&mut self, counter: u64) {
        self.shell.on_frame_presented(counter);
    }
    pub fn title_refresh_due(&mut self, now: Instant) -> bool {
        self.shell.should_refresh_title(now)
    }
    pub fn update_window_size(&mut self, width: u32, height: u32) -> Result<(), SessionError> {
        Ok(self.session.update_window_size(width, height)?)
    }
    /// Live media size in device pixels as a plain pair. Frontends scale
    /// and convert to toolkit sizes themselves; the shell struct never
    /// crosses the boundary.
    pub fn window_size(&self) -> (f32, f32) {
        let size = self.session.window_size();
        (size.width, size.height)
    }
    /// Remembered main-window size as a plain tuple. Frontends convert
    /// to their own logical-size types; the settings-core struct never
    /// crosses the boundary.
    pub fn remembered_window_size(&self) -> Option<(u32, u32)> {
        self.settings_snapshot()
            .app_state
            .window_size("main")
            .map(|size| (size.width, size.height))
    }
    /// Fixed-scale factor for the main window, if any. `None` means
    /// free-sizing: frontends restore the remembered size instead.
    /// Frontends apply the factor verbatim and never name `ScalingMode`
    /// or the shell helper themselves.
    pub fn window_scale_factor(&self) -> Option<u32> {
        nerust_gui_shell::settings::scaling_factor(
            self.settings_snapshot().local.video.window.scaling,
        )
    }
    /// ROM-picker filters: unified "All ROMs" first, then per-core entries.
    /// Frontends build native filter objects verbatim from these pairs and
    /// never name the shell registry themselves.
    pub fn rom_dialog_filters(&self) -> Vec<(String, Vec<String>)> {
        nerust_gui_shell::registry::rom_picker_filters(self.session.factories())
    }
    /// Persist the fullscreen default; Ok(true) when the default changed.
    /// Errors surface as strings for native logging; the shell result
    /// struct never crosses the boundary.
    pub fn persist_fullscreen_default(&mut self, fullscreen: bool) -> Result<bool, String> {
        self.set_fullscreen_default(fullscreen)
            .map(|plan| plan.fullscreen_default_changed)
            .map_err(|error| error.to_string())
    }

    pub fn enqueue_debug(
        &mut self,
        requests: &[nerust_gui_shell::debug::DebugAction],
        snapshot: Option<crate::debugger::DrainSnapshot>,
    ) {
        self.debug.enqueue(&self.session, requests, snapshot);
    }
    /// Take one debugger batch from the bridge and enqueue it, returning
    /// whether the PPU viewer was requested. Empty batches skip the drain;
    /// native code polls the flight and opens aux windows itself.
    pub fn take_debug_requests(
        &mut self,
        bridge: &crate::debugger::DebugBridge,
        snapshot: Option<crate::debugger::DrainSnapshot>,
    ) -> bool {
        bridge.bind_epoch(self.session.debug_epoch());
        let (open_ppu, requests) = bridge.take_debug_batch();
        if !requests.is_empty() {
            self.enqueue_debug(&requests, snapshot);
        }
        open_ppu
    }
    /// Enqueue the fixed open-debugger batch (Pause + Refresh).
    pub fn open_debug_drain(
        &mut self,
        bridge: &crate::debugger::DebugBridge,
        snapshot: Option<crate::debugger::DrainSnapshot>,
    ) {
        bridge.bind_epoch(self.session.debug_epoch());
        self.enqueue_debug(
            &[
                nerust_gui_shell::debug::DebugAction::Pause,
                nerust_gui_shell::debug::DebugAction::Refresh,
            ],
            snapshot,
        );
    }
    /// Enqueue an imageless refresh for the open viewer. Bridgeless
    /// windows still enqueue so the chrome gate observes the drain.
    pub fn refresh_debug_drain(
        &mut self,
        bridge: Option<&crate::debugger::DebugBridge>,
        snapshot: Option<crate::debugger::DrainSnapshot>,
    ) {
        if let Some(bridge) = bridge {
            bridge.bind_epoch(self.session.debug_epoch());
        }
        self.enqueue_debug(&[], snapshot);
    }
    pub fn poll_debug(&mut self, bridge: &crate::debugger::DebugBridge) -> bool {
        self.debug
            .poll(&mut self.session, &mut self.pause_policy, bridge)
    }
    pub fn pump_live_debug(
        &mut self,
        debug_open: bool,
        ppu_open: bool,
        bridge: &crate::debugger::DebugBridge,
    ) {
        self.debug
            .pump_live(&self.session, debug_open, ppu_open, bridge, Instant::now());
    }
    /// One debug turn. Wake interest observes pause changes from the drain,
    /// not the frame captured before it. Native ports only publish outputs.
    /// Test-only driver pump: tests own their drain queue and bridge.
    #[cfg(test)]
    pub(crate) fn poll_driver_with(
        &mut self,
        driver: &mut crate::debugger::DebuggerDriver,
        bridge: &crate::debugger::DebugBridge,
    ) -> bool {
        driver
            .poll(&mut self.session, &mut self.pause_policy, bridge)
            .pending
    }

    pub fn pump_debug(
        &mut self,
        debug_open: bool,
        ppu_open: bool,
        bridge: &crate::debugger::DebugBridge,
    ) -> wakui::loop_driver::WakeNeed {
        self.pump_live_debug(debug_open, ppu_open, bridge);
        let pending = self.poll_debug(bridge);
        self.debug_wake(debug_open, ppu_open, pending)
    }
    pub fn live_follow_active(&self, debug_open: bool, ppu_open: bool) -> bool {
        DebugRuntime::live_follow_active(&self.session, debug_open, ppu_open)
    }
    pub fn debug_wake(
        &mut self,
        debug_open: bool,
        ppu_open: bool,
        drain_pending: bool,
    ) -> wakui::loop_driver::WakeNeed {
        self.debug
            .wake(&self.session, debug_open, ppu_open, drain_pending)
    }
    pub fn chrome_drain_due(&self, debug_open: bool) -> bool {
        self.debug.chrome_drain_due(debug_open, self.tick().paused)
    }
    pub fn debug_pending(&self) -> bool {
        self.debug.pending()
    }
    pub fn close_debug_flights(&mut self) {
        self.debug.close();
    }

    pub fn menu_update(&mut self, ctx: &FrameCtx, settings_open: bool) -> Option<MenuSig> {
        self.chrome
            .menu_update(ctx, settings_open, self.session.active_slot_id())
    }

    /// Independent of the legacy signature gate: calling menu_update cannot
    /// consume this update. Unchanged captures skip building the tree entirely.
    /// Changed signatures are delivered even if the resulting tree is equal;
    /// MenuPresenter then skips native work, not the signature notification.
    pub fn menubar_update(
        &mut self,
        ctx: &FrameCtx,
        capabilities: MenuCapabilities,
    ) -> Option<MenubarUpdate> {
        // The modal guard is view-side state; flavors never pass it in.
        let settings_open = self.owns_pause(pause_owners::SETTINGS);
        let signature =
            match self
                .menubar_chrome
                .menu_update(ctx, settings_open, self.session.active_slot_id())
            {
                Some(signature) => signature,
                None if self.menu_capabilities == Some(capabilities) => return None,
                None => self
                    .menubar_chrome
                    .menu_signature()
                    .expect("menubar signature initialized")
                    .clone(),
            };
        let (menubar, table) =
            crate::menu::build_menubar(&mut self.menu_command_ids, &signature, capabilities);
        self.menu_commands = table;
        self.menu_capabilities = Some(capabilities);
        self.main_window.menu = Some(menubar.clone());
        Some(MenubarUpdate { signature, menubar })
    }

    /// View-side settings protocol for native open/close paths.
    /// Returns whether the Settings hold changed.
    pub fn note_settings_opened(&mut self) -> bool {
        crate::session::SessionDriver::new(
            &mut self.session,
            &mut self.pause_policy,
            &self.menu_commands,
        )
        .note_settings_opened()
        .settings_changed
    }

    /// View-side settings protocol for native close and failed-open paths.
    /// Returns whether the Settings hold changed.
    pub fn note_settings_dismissed(&mut self) -> bool {
        crate::session::SessionDriver::new(
            &mut self.session,
            &mut self.pause_policy,
            &self.menu_commands,
        )
        .note_settings_dismissed()
        .settings_changed
    }

    pub fn main_window(&self) -> &Window {
        &self.main_window
    }

    pub fn menubar(&self) -> Option<&Menubar> {
        self.main_window.menu.as_ref()
    }
    pub fn title_update(&mut self, tick: &TickMetrics) -> Option<String> {
        let title = self.cached_title(tick);
        self.main_window.title.clone_from(&title);
        self.chrome.title_update(title)
    }
    /// Title refresh without shell vocabulary: reads the live tick
    /// internally and publishes a new main-window title when changed.
    /// Frontends apply the returned title to the native window verbatim.
    pub fn refresh_window_title(&mut self) -> Option<String> {
        let tick = self.session.tick_metrics();
        self.title_update(&tick)
    }
    pub fn title_same(&self, title: &str) -> bool {
        self.chrome.title_same(title)
    }

    /// Return a native request only when the persisted target differs.
    pub fn fullscreen_request(&mut self, native: bool) -> Option<bool> {
        self.fullscreen.request(
            native,
            self.settings_snapshot()
                .local
                .video
                .window
                .fullscreen_default,
        )
    }
    /// Ignore intermediate native observations while awaiting the target.
    pub fn observe_fullscreen(&mut self, native: bool) -> bool {
        self.fullscreen.observe(
            native,
            self.settings_snapshot()
                .local
                .video
                .window
                .fullscreen_default,
        )
    }
    /// One-shot window chrome sync: the fullscreen request plus the
    /// remembered size to restore when the window ends up windowed.
    /// Size restore applies in free-sizing mode only; fixed-scale modes
    /// keep their exact pixel size. Frontends apply both verbatim and
    /// never name `ScalingMode` themselves.
    pub fn window_chrome_sync(
        &mut self,
        native_fullscreen: bool,
    ) -> wakui::window::WindowChromeSync {
        use nerust_settings_core::model::local::ScalingMode;
        let request = self.fullscreen_request(native_fullscreen);
        let ends_windowed = !request.unwrap_or(native_fullscreen);
        let snapshot = self.settings_snapshot();
        let restore_size =
            if ends_windowed && snapshot.local.video.window.scaling == ScalingMode::FitToWindow {
                snapshot
                    .app_state
                    .window_size("main")
                    .map(|size| (size.width as i32, size.height as i32))
            } else {
                None
            };
        wakui::window::WindowChromeSync {
            restore_size,
            set_fullscreen: request,
        }
    }
    pub fn pending_fullscreen(&self) -> Option<bool> {
        self.fullscreen.pending()
    }
    pub fn fail_window(&mut self, detail: String) {
        self.close.fail(detail);
    }
    pub fn fatal_message(&self) -> Option<&str> {
        self.close.fatal_message()
    }
    pub fn window_usable(&self) -> bool {
        self.close.is_usable()
    }
    pub fn close_exited(&self) -> bool {
        self.close.exited()
    }
    /// Retire liveness first. Native teardown follows: aux -> flights -> window -> flush.
    pub fn begin_close(&mut self) -> bool {
        if !self.close.begin_close() {
            return false;
        }
        self.osd.close();
        self.pause_policy = Default::default();
        self.invalidate_history();
        true
    }
    pub fn finish_close(&mut self) {
        self.finish_close_with(SessionHandle::flush_before_exit);
    }
    fn finish_close_with(&mut self, flush: impl FnOnce(&mut SessionHandle)) {
        if self.close.exited() && !self.flushed {
            self.flushed = true;
            flush(&mut self.session);
        }
    }
}

crate::impl_session_controls!(ApplicationHost);

#[derive(Debug, Clone)]
pub struct MenubarUpdate {
    pub signature: MenuSig,
    pub menubar: Menubar,
}

// Scheduling already exposes meaningful request/consume/cadence operations;
// reuse that facade rather than growing a second forest of forwarding methods.
impl std::ops::Deref for ApplicationHost {
    type Target = crate::aux_schedule::AuxSchedule<crate::focus::AuxKind>;
    fn deref(&self) -> &Self::Target {
        self.debug.schedule()
    }
}
impl std::ops::DerefMut for ApplicationHost {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.debug.schedule_mut()
    }
}

impl SessionControl for ApplicationHost {
    fn unload(&mut self) {
        if let Err(error) = self.session.unload() {
            log::warn!("ROM unload failed: {error}");
        }
    }
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        crate::session::apply_settings(&mut self.session, &self.pause_policy, settings)
    }
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }
}
