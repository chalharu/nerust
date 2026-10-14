use nerust_gui_runtime::{
    settings::{
        BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities,
        SettingsSnapshot,
    },
    shell::NativeShellState,
};
use nerust_gui_shell::{
    context::FrontendContext,
    debug::{DrainFlight, LiveImagesFlight},
    session::{
        KeyboardShortcut, SessionError, SessionHandle,
        access::{FrontendSession, SettingsResult},
        commands::SessionCommand,
        lifecycle::WindowSize,
    },
    settings::scaling_factor,
};
use nerust_render_traits::{
    SurfaceSize,
    renderer::{GpuFactory, RenderResult},
};
use nerust_settings_core::i18n::{UiText, text};
use nerust_settings_core::model::app_state::RememberedWindowSize;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use rfd::FileDialog;
use std::{path::Path, rc::Rc, sync::Arc, time::Instant};
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::{ElementState, KeyEvent},
    event_loop::{ControlFlow, EventLoopWindowTarget},
    window::{Fullscreen, Window as TaoWindow, WindowBuilder, WindowId},
};

use crate::event::{MenuCommand, UserEvent};
use crate::menu::imp::AppMenu;

use super::debug::DebugWindows;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum HostAction {
    None,
    RomLoaded,
    Exit,
}

const DEFAULT_FIT_WINDOW_WIDTH: f64 = 960.0;
const DEFAULT_FIT_WINDOW_HEIGHT: f64 = 720.0;

pub(crate) struct HostState {
    window: Option<Arc<TaoWindow>>,
    pub(crate) session: SessionHandle,
    ctx: FrontendContext,
    app_menu: AppMenu,
    shell: NativeShellState,
    pub(crate) settings_window: Option<crate::aux::settings::window::SettingsWindowHandle>,
    settings_open: bool,
    resume_after_settings: bool,
    pending_fullscreen_sync: Option<bool>,
    pub(crate) active: bool,
    auto_paused: bool,
    /// Deferred auto-pause: armed on main blur, cancelled when our own
    /// window takes focus in the same burst, executed at
    /// `MainEventsCleared`.
    pending_auto_pause: bool,
    /// Pause state at the last completed debug drain, for focus
    /// coalescing below. `None` before the first drain.
    pub(crate) last_drain_paused: Option<bool>,
    /// In-flight async debug drain: `run_debug_drain` starts it, the
    /// event loop polls it (`poll_debug_flight`). A newer batch
    /// supersedes the stored one; orphaned replies are ignored.
    pub(crate) drain_flight: Option<DrainFlight>,
    /// In-flight async live-images fetch: the live tick starts it,
    /// the event loop polls it. At most one; a newer tick supersedes.
    pub(crate) live_flight: Option<LiveImagesFlight>,
    /// Aux-window state (debugger + PPU viewer). Orchestration lives
    /// here; ownership lives there.
    pub(crate) debug: DebugWindows,
    /// Linux render target below the muda menubar (`None` elsewhere).
    /// When it yields no native target (Wayland, pre-layout) the
    /// toplevel fallback applies and the menu stays occluded.
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    content_area: Option<crate::gtk_embed::ContentArea>,
}

impl HostState {
    pub(crate) fn new(ctx: FrontendContext, app_menu: AppMenu) -> Self {
        let capabilities = HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: true,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: Some(BackendPresentationCapabilities {
                supports_vsync: true,
            }),
        };
        let session = SessionHandle::new(
            capabilities,
            ctx.registry.clone(),
            ctx.audio_registry.clone(),
        )
        .unwrap_or_else(|e| {
            log::error!("failed to create core: {e}");
            std::process::exit(1);
        });
        Self {
            window: None,
            session,
            ctx,
            app_menu,
            shell: NativeShellState::new(),
            settings_window: None,
            settings_open: false,
            resume_after_settings: false,
            pending_fullscreen_sync: None,
            active: true,
            auto_paused: false,
            pending_auto_pause: false,
            last_drain_paused: None,
            drain_flight: None,
            live_flight: None,
            debug: DebugWindows::new(),
            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            content_area: None,
        }
    }

    pub(crate) fn gpu_factory(&self) -> &Rc<dyn GpuFactory> {
        &self.ctx.gpu_factory
    }

    pub(crate) fn session(&self) -> &SessionHandle {
        &self.session
    }

    pub(crate) fn session_mut(&mut self) -> &mut SessionHandle {
        &mut self.session
    }

    pub(crate) fn window(&self) -> Option<&Arc<TaoWindow>> {
        self.window.as_ref()
    }

    pub(crate) fn resume_session(&mut self) {
        FrontendSession::resume(self);
    }

    pub(crate) fn pause_session(&mut self) {
        FrontendSession::pause(self);
    }

    pub(crate) fn auto_paused(&self) -> bool {
        self.auto_paused
    }

    pub(crate) fn set_auto_paused(&mut self) {
        self.auto_paused = true;
    }

    pub(crate) fn clear_auto_paused(&mut self) {
        self.auto_paused = false;
    }

    /// Arm a deferred auto-pause on focus loss. Execution waits for
    /// `commit_pending_auto_pause` so a same-burst focus arrival on
    /// another of our windows can cancel it first.
    pub(crate) fn arm_pending_auto_pause(&mut self) {
        self.pending_auto_pause = true;
    }

    /// Cancel a deferred auto-pause: focus arrived on another of our
    /// windows, so the session stays as the user left it.
    pub(crate) fn cancel_pending_auto_pause(&mut self) {
        self.pending_auto_pause = false;
    }

    /// Execute a still-armed auto-pause at the end of the event burst.
    /// Post-pause chrome sync keeps the menu/title truthful (the blur
    /// path below only synced pre-pause truth).
    pub(crate) fn commit_pending_auto_pause(&mut self) {
        if !std::mem::take(&mut self.pending_auto_pause) {
            return;
        }
        if self.session.can_pause() {
            self.pause_session();
            self.set_auto_paused();
            self.sync_debug_chrome();
        }
    }

    pub(crate) fn ensure_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.window.is_some() {
            return;
        }

        let fullscreen = self
            .session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        let window = Arc::new(
            create_window_builder(
                self.startup_window_size(),
                self.session.window_title(),
                fullscreen,
            )
            .build(event_loop)
            .unwrap(),
        );
        self.app_menu.init_for_window(&window);
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            use tao::platform::unix::WindowExtUnix;
            self.content_area = window
                .default_vbox()
                .map(|vbox| crate::gtk_embed::ContentArea::new(vbox, window.gtk_window(), &window));
        }
        self.window = Some(window);
        self.sync_fullscreen_from_settings();
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    pub(crate) fn is_window(&self, window_id: WindowId) -> bool {
        self.window
            .as_ref()
            .is_some_and(|window| window.id() == window_id)
    }

    pub(crate) fn is_settings_window(&self, window_id: WindowId) -> bool {
        self.settings_window
            .as_ref()
            .is_some_and(|h| h.window.id() == window_id)
    }

    pub(crate) fn window_surface_size(&self) -> Option<SurfaceSize> {
        self.surface_target().map(|(_, size, _)| size)
    }

    /// Native attach target for the game renderer: the content area
    /// below the menubar when it has a native window, else the
    /// toplevel (menu occluded, as before). The xid tags the target
    /// so the runtime recreates the surface across target changes
    /// instead of merely resizing into the wrong drawable.
    pub(crate) fn surface_target(&self) -> Option<(RawWindowHandle, SurfaceSize, Option<u64>)> {
        let window = self.window.as_ref()?;
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        if let Some(area) = self.content_area.as_ref()
            && let Some((handle, size, xid)) = area.target()
        {
            return Some((handle, size, Some(xid)));
        }
        let handle = window.window_handle().ok()?.as_raw();
        Some((handle, window_surface_size(window.inner_size()), None))
    }

    /// Xid the surface is currently attached to (`None` = toplevel
    /// fallback). Mirrors [`HostState::surface_target`] without
    /// rebuilding handles, for the runtime's recreate check.
    pub(crate) fn surface_xid(&self) -> Option<u64> {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        if let Some(area) = self.content_area.as_ref()
            && let Some((_, _, xid)) = area.target()
        {
            return Some(xid);
        }
        None
    }

    /// Load a ROM, syncing menu/title/redraw on success. Returns
    /// the failure detail on error; presenting it (dialog) is the
    /// caller's job so the file-dialog and startup paths can differ.
    pub(crate) fn load_path(&mut self, path: &Path) -> Result<(), String> {
        match self.ctx.rom_loader.load_rom(path, &mut self.session) {
            Ok(()) => {
                self.after_rom_load();
                Ok(())
            }
            Err(error) => {
                log::warn!("ROM load failed: {error}");
                Err(error.to_string())
            }
        }
    }

    pub(crate) fn on_menu_command(
        &mut self,
        command: MenuCommand,
        event_loop: &EventLoopWindowTarget<UserEvent>,
    ) -> HostAction {
        if self.settings_open {
            return match command {
                MenuCommand::Quit => {
                    if self.prepare_close() {
                        HostAction::Exit
                    } else {
                        HostAction::None
                    }
                }
                _ => HostAction::None,
            };
        }
        match command {
            MenuCommand::Open => {
                if self.open_rom_dialog() {
                    HostAction::RomLoaded
                } else {
                    HostAction::None
                }
            }
            MenuCommand::Close => {
                self.close_rom();
                HostAction::None
            }
            MenuCommand::Settings => {
                self.open_settings_window(event_loop);
                HostAction::None
            }
            MenuCommand::Debugger => {
                self.open_debug_windows(event_loop, false);
                HostAction::None
            }
            MenuCommand::Session(command) => {
                self.run_command(command);
                self.sync_menu_state();
                self.refresh_window_title();
                HostAction::None
            }
            MenuCommand::Quit => {
                if self.prepare_close() {
                    HostAction::Exit
                } else {
                    HostAction::None
                }
            }
        }
    }

    pub(crate) fn on_keyboard_input(&mut self, input: KeyEvent) {
        if self.settings_open {
            return;
        }
        if let Some(pressed) = element_state_to_pressed(input.state)
            && let Some(key) = crate::convert::keycode_to_key(input.physical_key)
            && let Some(shortcut) = self.session.handle_keyboard_key(key, pressed)
        {
            self.apply_keyboard_shortcut(shortcut);
        }
    }

    pub(crate) fn clear_keys(&mut self) {
        self.session.clear_input();
    }

    pub(crate) fn sync_fullscreen_default_from_window(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let fullscreen = window_is_fullscreen(window);
        let sync = derive_native_fullscreen_sync(
            self.pending_fullscreen_sync,
            fullscreen,
            self.session
                .settings_snapshot()
                .local
                .video
                .window
                .fullscreen_default,
        );
        self.pending_fullscreen_sync = sync.pending_target;
        if !sync.persist_setting {
            return;
        }
        match self.session.set_fullscreen_default(fullscreen) {
            Ok(plan) if plan.fullscreen_default_changed => {
                self.sync_menu_state();
                self.refresh_window_title();
                self.request_redraw();
            }
            Ok(_) => (),
            Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
        }
    }

    pub(crate) fn update_control_flow(&mut self, control_flow: &mut ControlFlow) {
        // End-of-burst auto-pause first: pause state settles before
        // the title/live/drain logic below reads it.
        self.commit_pending_auto_pause();
        self.sync_fullscreen_default_from_window();
        self.maybe_refresh_window_title(Instant::now());
        self.maybe_live_ppu();
        // Poll a pending drain flight (non-blocking): the reply lands
        // on a normal event without a dedicated wake in the common
        // case, and the backstop below bounds the latency when the
        // loop would otherwise sleep.
        let drain_pending = self.poll_debug_flight();
        *control_flow = ControlFlow::Wait;
        // While the PPU follows a running core, wake for the next
        // tick: plain Wait sleeps until the next input event, which
        // would collapse the live cadence to zero whenever the aux
        // windows (not the main one) hold focus.
        if self.live_follow_active()
            && let Some(next) = self.debug.next_live_tick()
        {
            *control_flow = ControlFlow::WaitUntil(next);
        }
        // While a drain or live flight is pending, wake soon to poll
        // it: the reply applies within ~30ms instead of waiting for
        // the next input event.
        if drain_pending || self.live_flight.is_some() {
            let soon = Instant::now() + std::time::Duration::from_millis(30);
            let earlier = match *control_flow {
                ControlFlow::WaitUntil(at) => at.min(soon),
                _ => soon,
            };
            *control_flow = ControlFlow::WaitUntil(earlier);
        }

        // Inactive loop sleeps; redraw while focused, while the PPU
        // follows a running core, or while the session runs (a bare
        // WaitUntil alone would not reliably wake the loop, at the
        // cost of one running frame render).
        let live = self.live_follow_active();
        // Repaint a running session even unfocused (e.g. operating the
        // debugger while the game runs): frames keep coming and must
        // show. Paused, unfocused, and unfollowed sleeps.
        let running = self.session.loaded() && !self.session.paused();
        if (self.active || live || running)
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }

    pub(crate) fn on_render_result(&mut self, result: RenderResult) {
        match result {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.metrics().frame_counter);
                self.maybe_refresh_window_title(Instant::now());
            }
            RenderResult::Skipped | RenderResult::Error => {}
        }
    }

    pub(crate) fn clear_event_handler(&self) {
        self.app_menu.clear_event_handler();
    }

    pub(crate) fn request_redraw(&mut self) {
        self.shell.needs_redraw = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    pub(crate) fn prepare_close(&mut self) -> bool {
        self.remember_fit_window_size();
        self.settings_open = false;
        self.resume_after_settings = false;
        self.settings_window.take();
        self.debug.close_all();
        self.session.flush_before_exit();
        true
    }

    pub(crate) fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let result = FrontendSession::apply_settings(self, settings)?;
        if result.fullscreen_default_changed {
            self.sync_fullscreen_from_settings();
        }
        if result.scaling_changed {
            self.update_window_size_for_scaling();
        }
        self.sync_menu_state();
        self.refresh_window_title();
        self.request_redraw();
        Ok(result)
    }

    pub(crate) fn on_settings_closed(&mut self) {
        self.settings_window = None;
        if !self.settings_open {
            return;
        }
        self.settings_open = false;
        let should_resume = std::mem::take(&mut self.resume_after_settings);
        if should_resume {
            self.resume();
        } else {
            self.sync_menu_state();
            self.refresh_window_title();
        }
    }

    fn open_rom_dialog(&mut self) -> bool {
        let mut dialog = FileDialog::new().set_title(text(
            self.session.settings_snapshot().shared.general.language,
            UiText::Open,
        ));
        for factory in self.ctx.registry.all() {
            let extensions = factory.supported_extensions();
            if !extensions.is_empty() {
                dialog = dialog.add_filter(format!("{} ROM", factory.display_name()), extensions);
            }
        }
        let Some(path) = dialog.pick_file() else {
            return false;
        };
        if let Err(detail) = self.load_path(&path) {
            show_rom_load_error(
                self.session.settings_snapshot().shared.general.language,
                &detail,
            );
            return false;
        }
        true
    }

    fn after_rom_load(&mut self) {
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    fn close_rom(&mut self) {
        FrontendSession::unload(self);
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    pub(crate) fn sync_menu_state(&mut self) {
        let snapshot = self.session.settings_snapshot();
        let slots = self.session.slots();
        self.app_menu.update(crate::menu::MenuSync {
            loaded: self.session.loaded(),
            paused: self.session.paused(),
            slots,
            active_slot: self.session.active_slot_id(),
            settings_open: self.settings_open,
            language: snapshot.shared.general.language,
            shortcuts: &snapshot.shared.input.shortcuts.keyboard,
        });
    }

    pub(crate) fn refresh_window_title(&mut self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&self.session.window_title());
        }
    }

    fn maybe_refresh_window_title(&mut self, now: Instant) {
        if self.shell.should_refresh_title(now) {
            self.refresh_window_title();
        }
    }

    fn apply_keyboard_shortcut(&mut self, shortcut: KeyboardShortcut) {
        use nerust_gui_shell::session::access::dispatch_session_action;
        match shortcut {
            // The debugger open rides the same schema as every other
            // shortcut (default F4, rebindable in Settings): it only
            // sets the flag here because opening needs the event loop,
            // which the runtime arm owns.
            KeyboardShortcut::OpenDebugger => {
                self.debug.request_debug_open();
                return;
            }
            KeyboardShortcut::Session(action) => dispatch_session_action(self, action),
            KeyboardShortcut::ToggleFullscreen => self.toggle_fullscreen(),
        }
        self.sync_menu_state();
        self.refresh_window_title();
    }

    fn toggle_fullscreen(&mut self) {
        let fullscreen = if let Some(window) = self.window.as_ref() {
            !window_is_fullscreen(window)
        } else {
            return;
        };
        if let Err(error) = self.persist_fullscreen_default(fullscreen) {
            log::warn!("failed to persist fullscreen setting: {error}");
            if let Some(window) = self.window.as_ref() {
                set_window_fullscreen(window, fullscreen);
            }
            self.request_redraw();
        }
    }

    fn open_settings_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.settings_open {
            return;
        }
        self.settings_open = true;
        self.resume_after_settings = self.session.loaded() && !self.session.paused();
        if self.resume_after_settings {
            self.pause();
        } else {
            self.sync_menu_state();
        }

        match crate::aux::settings::window::SettingsWindowHandle::new(
            self.session.settings_snapshot().clone(),
            self.ctx.registry.clone(),
            self.ctx.audio_registry.clone(),
            event_loop,
        ) {
            Some(handle) => self.settings_window = Some(handle),
            None => {
                log::error!("failed to open settings window");
                self.settings_open = false;
                if self.resume_after_settings {
                    self.resume();
                } else {
                    self.sync_menu_state();
                }
            }
        }
    }

    pub(crate) fn close_settings_window(
        &mut self,
        mut handle: crate::aux::settings::window::SettingsWindowHandle,
    ) -> Option<SettingsResult> {
        let pending = handle.take_pending_apply();
        drop(handle);
        self.on_settings_closed();
        if let Some(snapshot) = pending {
            match self.apply_settings(snapshot) {
                Ok(plan) => return Some(plan),
                Err(error) => {
                    log::warn!("settings apply failed after close: {error}");
                }
            }
        }
        None
    }

    fn startup_window_size(&self) -> TaoLogicalSize<f64> {
        match scaling_factor(self.session.settings_snapshot().local.video.window.scaling) {
            Some(scale) => logical_window_size(self.session.window_size(), Some(scale)),
            None => self
                .remembered_fit_window_size()
                .map(logical_size_from_remembered)
                .unwrap_or_else(default_fit_window_size),
        }
    }

    fn update_window_size_for_scaling(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(scale) =
            scaling_factor(self.session.settings_snapshot().local.video.window.scaling)
        else {
            return;
        };
        window.set_inner_size(logical_window_size(self.session.window_size(), Some(scale)));
    }

    fn sync_fullscreen_from_settings(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let fullscreen = self
            .session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        if window_is_fullscreen(window) == fullscreen {
            self.pending_fullscreen_sync = None;
            return;
        }
        self.pending_fullscreen_sync = Some(fullscreen);
        set_window_fullscreen(window, fullscreen);
    }

    fn persist_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        self.sync_fullscreen_from_settings();
        self.sync_menu_state();
        self.refresh_window_title();
        self.request_redraw();
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }

    fn remembered_fit_window_size(&self) -> Option<RememberedWindowSize> {
        self.session
            .settings_snapshot()
            .app_state
            .window_size("main")
    }

    fn remember_fit_window_size(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window_is_fullscreen(window)
            || scaling_factor(self.session.settings_snapshot().local.video.window.scaling).is_some()
        {
            return;
        }

        let logical_size = window.inner_size().to_logical::<f64>(window.scale_factor());
        let width = logical_size.width.round().max(1.0) as u32;
        let height = logical_size.height.round().max(1.0) as u32;

        if let Err(error) = self
            .session
            .settings_manager()
            .update_window_size(width, height)
        {
            log::warn!("failed to remember tao window size: {error}");
        }
    }
}

impl FrontendSession for HostState {
    fn pause(&mut self) {
        let _ = self.session.run_command(SessionCommand::Pause);
    }
    fn resume(&mut self) {
        let _ = self.session.run_command(SessionCommand::Resume);
    }
    fn toggle_pause(&mut self) {
        let _ = self.session.run_command(SessionCommand::TogglePause);
    }
    fn save_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::SaveActiveSlotOrNew)
            .unwrap_or_default()
            .executed
    }
    fn load_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::LoadActiveSlot)
            .unwrap_or_default()
            .executed
    }
    fn select_next_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectNextSlot);
    }
    fn select_previous_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectPreviousSlot);
    }
    fn load_slot(&mut self, slot_id: u64) -> bool {
        self.session
            .run_command(SessionCommand::LoadSlot(slot_id))
            .unwrap_or_default()
            .executed
    }
    fn save_slot(&mut self, slot_id: u64) {
        let _ = self.session.run_command(SessionCommand::SaveSlot(slot_id));
    }
    fn delete_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::DeleteSlot(slot_id));
    }
    fn select_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::SelectActiveSlot(slot_id));
    }
    fn create_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::CreateSlot);
    }
    fn reset(&mut self) {
        let _ = self.session.run_command(SessionCommand::Reset);
    }
    fn unload(&mut self) {
        if let Err(error) = self.session.unload() {
            log::warn!("ROM unload failed: {error}");
        }
    }
    fn run_command(&mut self, command: SessionCommand) {
        let _ = self.session.run_command(command);
    }
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.apply_settings(settings)?;
        Ok(SettingsResult {
            // The renderer follows presentation, not the session: a
            // live filter swap rebuilds no core but still needs the new
            // GPU assets (same rule as the android twin).
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: plan.scaling_changed,
        })
    }
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required
                || plan.session_rebuild_required
                || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }
}

fn create_window_builder(
    size: TaoLogicalSize<f64>,
    title: String,
    fullscreen: bool,
) -> WindowBuilder {
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(size)
        .with_fullscreen(fullscreen_mode(fullscreen))
}

/// Surface ROM load failures in a modal dialog: the previous
/// warn-only path left the empty window with no explanation.
pub(super) fn show_rom_load_error(
    language: nerust_settings_core::model::language::AppLanguage,
    detail: &str,
) {
    let title = text(language, UiText::RomLoadFailed);
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(detail)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn window_is_fullscreen(window: &TaoWindow) -> bool {
    window.fullscreen().is_some()
}

fn set_window_fullscreen(window: &TaoWindow, fullscreen: bool) {
    window.set_fullscreen(fullscreen_mode(fullscreen));
}

fn fullscreen_mode(fullscreen: bool) -> Option<Fullscreen> {
    fullscreen.then_some(Fullscreen::Borderless(None))
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct NativeFullscreenSync {
    pending_target: Option<bool>,
    persist_setting: bool,
}

fn derive_native_fullscreen_sync(
    pending_target: Option<bool>,
    fullscreen: bool,
    persisted_fullscreen: bool,
) -> NativeFullscreenSync {
    if let Some(expected) = pending_target
        && fullscreen != expected
    {
        return NativeFullscreenSync {
            pending_target: Some(expected),
            persist_setting: false,
        };
    }

    NativeFullscreenSync {
        pending_target: None,
        persist_setting: fullscreen != persisted_fullscreen,
    }
}

fn window_surface_size(size: TaoPhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize::new(size.width, size.height)
}

fn logical_window_size(size: WindowSize, scaling: Option<u32>) -> TaoLogicalSize<f64> {
    let (width, height) = scaling
        .map(|scale| {
            (
                f64::from(size.width) * f64::from(scale),
                f64::from(size.height) * f64::from(scale),
            )
        })
        .unwrap_or((f64::from(size.width), f64::from(size.height)));
    TaoLogicalSize::new(width, height)
}

fn default_fit_window_size() -> TaoLogicalSize<f64> {
    TaoLogicalSize::new(DEFAULT_FIT_WINDOW_WIDTH, DEFAULT_FIT_WINDOW_HEIGHT)
}

fn logical_size_from_remembered(size: RememberedWindowSize) -> TaoLogicalSize<f64> {
    TaoLogicalSize::new(f64::from(size.width), f64::from(size.height))
}

fn element_state_to_pressed(state: ElementState) -> Option<bool> {
    Some(match state {
        ElementState::Pressed => true,
        ElementState::Released => false,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use tao::{dpi::LogicalSize as TaoLogicalSize, window::Fullscreen};

    use super::{NativeFullscreenSync, create_window_builder, derive_native_fullscreen_sync};

    #[test]
    fn window_builder_requests_initial_fullscreen_when_enabled() {
        let builder =
            create_window_builder(TaoLogicalSize::new(960.0, 720.0), "nerust".into(), true);

        assert_eq!(
            builder.window.fullscreen,
            Some(Fullscreen::Borderless(None))
        );
    }

    #[test]
    fn window_builder_skips_initial_fullscreen_when_disabled() {
        let builder =
            create_window_builder(TaoLogicalSize::new(960.0, 720.0), "nerust".into(), false);

        assert_eq!(builder.window.fullscreen, None);
    }

    #[test]
    fn fullscreen_sync_ignores_transient_state_while_transition_is_pending() {
        assert_eq!(
            derive_native_fullscreen_sync(Some(true), false, true),
            NativeFullscreenSync {
                pending_target: Some(true),
                persist_setting: false,
            }
        );
    }

    #[test]
    fn fullscreen_sync_clears_pending_transition_once_window_matches_target() {
        assert_eq!(
            derive_native_fullscreen_sync(Some(true), true, true),
            NativeFullscreenSync {
                pending_target: None,
                persist_setting: false,
            }
        );
    }

    #[test]
    fn fullscreen_sync_persists_native_changes_once_no_transition_is_pending() {
        assert_eq!(
            derive_native_fullscreen_sync(None, false, true),
            NativeFullscreenSync {
                pending_target: None,
                persist_setting: true,
            }
        );
    }
}
