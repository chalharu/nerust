//! AppKit-shaped host adapter: every responder-chain callback maps to one host call.
//!
//! `MacHost` owns the shared [`ApplicationHost`]; the methods below are the
//! complete selector surface a production port implements:
//! menu selectors → [`MacHost::on_menu`], `keyDown:`/`keyUp:` →
//! [`MacHost::on_key`], `NSApplicationDidBecomeActive`/
//! `NSApplicationDidResignActive` → [`MacHost::on_active`], display link →
//! [`MacHost::turn`], sheet lifetime → [`MacHost::open_settings`]/
//! [`MacHost::close_settings`]. The settings sheet uses the same ticket +
//! mailbox protocol as every other frontend; `NSAlert` sheets only change
//! where the dialog draws, never the staging.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use nerust_frontend_host::{
    application::ApplicationHost,
    focus::FocusEvent,
    frame::{FrameCache, FrameCtx},
    loop_driver::{LoopHost, TurnLiveness, TurnOutcome, WakeNeed, drive_turn},
    session::{SessionEffect, SessionEvent, SessionOutcome},
    settings::SettingsController,
};
use nerust_frontend_kernel::pause::PauseOwner;
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsSnapshot,
};
use nerust_gui_shell::{
    context::FrontendContext,
    session::{
        KeyboardShortcut, SessionError,
        access::{SessionControl, SettingsResult},
        commands::SessionCommand,
    },
};
use nerust_keyboard::Key;

use super::{
    keys::{mac_key_to_nerust, mac_menu_event},
    surface::MacSurface,
};

/// Narrow title-update interval, mirroring the gtk twin.
const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);

pub struct MacHost {
    pub(crate) host: ApplicationHost,
    ctx: FrontendContext,
    frames: FrameCache,
    fullscreen: bool,
    redraw_requests: u64,
    last_title_push: Instant,
}

impl MacHost {
    pub fn new(ctx: FrontendContext) -> Result<Self, SessionError> {
        let capabilities = HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: true,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        };
        let session = nerust_gui_shell::session::SessionHandle::new(
            capabilities,
            ctx.registry.clone(),
            ctx.audio_registry.clone(),
        )
        .map_err(|error| {
            log::error!("failed to create core: {error}");
            error
        })?;
        Ok(Self {
            host: ApplicationHost::new(session),
            ctx,
            frames: FrameCache::new(),
            fullscreen: false,
            redraw_requests: 0,
            last_title_push: Instant::now() - TITLE_UPDATE_INTERVAL,
        })
    }

    /// Test-only construction: settings persist under a unique temp dir.
    #[cfg(test)]
    pub fn new_for_tests(ctx: FrontendContext) -> Result<Self, SessionError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TEST_SETTINGS_SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nerust-mac-test-{}-{}",
            std::process::id(),
            TEST_SETTINGS_SEQ.fetch_add(1, Ordering::SeqCst)
        ));
        let paths = nerust_gui_runtime::settings::SettingsPaths {
            settings_file: dir.join("settings.toml"),
            central_storage_root: dir.join("storage"),
        };
        let capabilities = HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        };
        let session = nerust_gui_shell::session::SessionHandle::new_with_settings_paths(
            capabilities,
            ctx.registry.clone(),
            ctx.audio_registry.clone(),
            paths,
        )?;
        Ok(Self {
            host: ApplicationHost::new(session),
            ctx,
            frames: FrameCache::new(),
            fullscreen: false,
            redraw_requests: 0,
            last_title_push: Instant::now() - TITLE_UPDATE_INTERVAL,
        })
    }

    /// Menu selector: complete host event, unless Settings owns the UI.
    /// Returns whether the selector dispatched.
    pub fn on_menu(&mut self, selector: &str, surface: &mut impl MacSurface) -> bool {
        let Some(event) = mac_menu_event(selector) else {
            return false;
        };
        // Same gate as every twin: a sheet-modal settings dialog swallows menu.
        if self.host.owns_pause(PauseOwner::Settings) {
            return false;
        }
        match self.host.dispatch(event) {
            Ok(outcome) => {
                self.apply_outcome(outcome, surface);
                true
            }
            Err(error) => {
                log::warn!("menu dispatch failed: {error}");
                false
            }
        }
    }

    /// `keyDown:`/`keyUp:`: key code → shared key → host.
    pub fn on_key(&mut self, code: u16, pressed: bool) -> bool {
        let Some(key) = mac_key_to_nerust(code) else {
            return false;
        };
        self.dispatch_key_event(key, pressed)
    }

    /// Raw session command for chrome that owns no menu id (debugger buttons).
    pub fn dispatch_cmd(&mut self, command: SessionCommand) -> bool {
        self.host.dispatch(SessionEvent::Command(command)).is_ok()
    }

    fn dispatch_key_event(&mut self, key: Key, pressed: bool) -> bool {
        self.host
            .dispatch(SessionEvent::Keyboard { key, pressed })
            .is_ok()
    }

    /// Cmd-shortcut from the responder chain: schema-driven like all twins.
    pub fn on_shortcut(
        &mut self,
        shortcut: KeyboardShortcut,
        surface: &mut impl MacSurface,
    ) -> bool {
        match self.host.dispatch(SessionEvent::Shortcut(shortcut)) {
            Ok(outcome) => {
                self.apply_outcome(outcome, surface);
                true
            }
            Err(error) => {
                log::warn!("shortcut dispatch failed: {error}");
                false
            }
        }
    }

    fn apply_outcome(&mut self, outcome: SessionOutcome, surface: &mut impl MacSurface) {
        match outcome.effect {
            Some(SessionEffect::NativeOpenDebugger) => surface.request_debugger(),
            Some(SessionEffect::ToggleFullscreen) => {
                self.fullscreen = !self.fullscreen;
                surface.set_fullscreen(self.fullscreen);
            }
            None => {}
        }
    }

    /// `NSApplicationDidBecomeActive`/`DidResignActive`: arm now, commit
    /// on the next [`MacHost::turn`].
    pub fn on_active(&mut self, active: bool) {
        self.host.focus_event(FocusEvent::Main(active));
    }

    /// Commit deferred focus effects outside the turn. Ports whose loop
    /// cannot run the shared turn still honor the protocol through this.
    pub fn tick_focus_sync(&mut self) {
        self.host.tick_focus_sync();
    }

    /// Display link: commit deferred focus effects, then run the shared turn.
    pub fn turn(&mut self) -> TurnOutcome {
        drive_turn(&mut MacTurn { host: self })
    }

    /// Explicit title push for the `TITLE_UPDATE_INTERVAL` timer.
    pub fn push_title(&mut self, surface: &mut impl MacSurface) {
        if self.last_title_push.elapsed() < TITLE_UPDATE_INTERVAL {
            return;
        }
        self.last_title_push = Instant::now();
        let tick = self.host.tick();
        surface.set_title(&self.host.cached_title(&tick));
    }

    /// Settings sheet attach: acquire the Settings ticket, then hand out
    /// a controller staging into its own mailbox.
    pub fn open_settings(&mut self) -> SettingsController {
        let _ = self.host.dispatch(SessionEvent::SettingsOpened);
        let snapshot = self.host.settings_snapshot().clone();
        SettingsController::new(
            &snapshot,
            Arc::clone(&self.ctx.registry),
            Arc::clone(&self.ctx.audio_registry),
        )
    }

    /// Settings sheet detach: apply the staged snapshot, release the ticket.
    /// Returns whether a snapshot was applied.
    pub fn close_settings(
        &mut self,
        controller: &SettingsController,
    ) -> Result<bool, SessionError> {
        let applied = if let Some(snapshot) = controller.take_pending_apply() {
            self.host.apply_settings(snapshot)?;
            true
        } else {
            false
        };
        let _ = self.host.dispatch(SessionEvent::SettingsClosed);
        Ok(applied)
    }

    /// Shared session read for native pumps (frame capture, OSD text).
    pub fn session(&self) -> &nerust_gui_shell::session::SessionHandle {
        self.host.session()
    }

    pub fn session_mut(&mut self) -> &mut nerust_gui_shell::session::SessionHandle {
        self.host.session_mut()
    }

    /// Present the latest core frame. Call once per frame before reading it.
    pub fn swap_frame_buffer(&mut self) {
        self.host.swap_frame_buffer();
    }

    /// Pause ownership query for native gates (menus, dialogs).
    pub fn owns_pause(&self, owner: nerust_frontend_kernel::pause::PauseOwner) -> bool {
        self.host.owns_pause(owner)
    }

    /// Drop the notification history (History window Clear button).
    pub fn clear_history(&mut self) {
        self.host.clear_history();
    }

    /// Latest tick for native chrome (fps line, pause badge).
    pub fn tick(&self) -> nerust_gui_shell::session::osd::TickMetrics {
        self.host.tick()
    }

    pub fn load_path(&mut self, path: &Path) -> Result<(), String> {
        let Self { host, ctx, .. } = self;
        host.load_path(&mut *ctx.rom_loader, path).map_err(|error| {
            log::warn!("ROM load failed: {error}");
            error.to_string()
        })
    }
}

impl SessionControl for MacHost {
    fn unload(&mut self) {
        if let Err(error) = self.host.session_mut().unload() {
            log::warn!("ROM unload failed: {error}");
        }
    }

    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        self.host.apply_settings(settings)
    }

    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError> {
        let plan = self.host.session_mut().set_fullscreen_default(fullscreen)?;
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.renderer_rebuild_required,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }
}

/// Per-turn driver: the shared ordering, with native pumps defaulted.
struct MacTurn<'a> {
    host: &'a mut MacHost,
}

impl LoopHost for MacTurn<'_> {
    type Frame = FrameCtx;

    fn before_turn(&mut self) {
        self.host.host.tick_focus_sync();
    }

    fn frame_ctx(&mut self) -> Self::Frame {
        self.host.frames.frame_ctx(self.host.host.session())
    }

    fn chrome(&mut self, _ctx: &Self::Frame) {
        // Title/menu sync stays on the native timer via `push_title`.
    }

    fn pump_debug(&mut self) -> WakeNeed {
        WakeNeed::default()
    }

    fn pump_osd(&mut self, _ctx: &Self::Frame) -> WakeNeed {
        WakeNeed::default()
    }

    fn pump_aux(&mut self) -> WakeNeed {
        WakeNeed::default()
    }

    fn request_redraw(&mut self) {
        self.host.redraw_requests += 1;
    }

    fn late_liveness(&mut self) -> TurnLiveness {
        let tick = self.host.host.tick();
        TurnLiveness {
            loaded: tick.loaded,
            paused: tick.paused,
            main_active: self.host.host.main_active(),
            live_follow: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    use nerust_core_traits::audio::AudioBackendRegistry;
    use nerust_gui_shell::registry::SystemRegistry;

    use crate::surface::HeadlessSurface;

    fn test_host() -> (MacHost, HeadlessSurface) {
        let registry = Arc::new(SystemRegistry::new(Vec::new()));
        let audio = Arc::new(AudioBackendRegistry::new());
        let ctx = FrontendContext {
            gpu_factory: Rc::new(TestPipelineFactory),
            registry,
            rom_loader: Box::new(TestRomLoader),
            audio_registry: audio,
        };
        let host = MacHost::new_for_tests(ctx).expect("test host builds");
        (host, HeadlessSurface::default())
    }

    #[derive(Debug)]
    struct TestPipelineFactory;

    impl nerust_render_traits::renderer::PipelineFactory for TestPipelineFactory {
        fn create_pipeline(
            &self,
            _config: &nerust_render_traits::renderer::RendererConfig,
            _display: raw_window_handle::RawDisplayHandle,
        ) -> Result<
            Box<dyn nerust_render_traits::renderer::RenderPipeline>,
            nerust_render_traits::renderer::RendererError,
        > {
            Err(nerust_render_traits::renderer::RendererError::new(
                "test",
                Box::new(std::io::Error::other("no renderer in tests")),
            ))
        }
    }

    #[derive(Debug)]
    struct TestRomLoader;

    impl nerust_gui_shell::load::RomLoader for TestRomLoader {
        fn load_rom(
            &mut self,
            _path: &std::path::Path,
            _target: &mut dyn nerust_gui_shell::load::RomLoadTarget,
        ) -> Result<(), nerust_gui_shell::load::RomLoaderError> {
            Err(nerust_gui_shell::load::RomLoaderError::from_session(
                SessionError::NoCore,
            ))
        }
    }

    #[test]
    fn menu_pause_and_resume_toggle_user_owner() {
        use crate::keys::selector;
        let (mut host, mut surface) = test_host();
        assert!(host.on_menu(selector::PAUSE, &mut surface));
        assert!(host.host.owns_pause(PauseOwner::User));
        assert!(host.on_menu(selector::RESUME, &mut surface));
        assert!(!host.host.owns_pause(PauseOwner::User));
    }

    #[test]
    fn unknown_selector_is_ignored() {
        let (mut host, mut surface) = test_host();
        assert!(!host.on_menu("unknown:", &mut surface));
    }

    #[test]
    fn settings_sheet_gate_blocks_menu() {
        use crate::keys::selector;
        let (mut host, mut surface) = test_host();
        let _controller = host.open_settings();
        assert!(host.host.owns_pause(PauseOwner::Settings));
        assert!(!host.on_menu(selector::PAUSE, &mut surface));
        assert!(!host.host.owns_pause(PauseOwner::User));
    }

    #[test]
    fn settings_submit_stages_and_applies() {
        use nerust_frontend_host::settings::SettingsEvent;
        let (mut host, _) = test_host();
        let mut controller = host.open_settings();
        let _ = controller.update(SettingsEvent::ToggleFullscreenDefault(true));
        let _ = controller.update(SettingsEvent::Submit);
        let applied = host.close_settings(&controller).expect("close applies");
        assert!(applied);
        assert!(
            host.host
                .settings_snapshot()
                .local
                .video
                .window
                .fullscreen_default
        );
        assert!(!host.host.owns_pause(PauseOwner::Settings));
    }

    #[test]
    fn settings_cancel_applies_nothing() {
        let (mut host, _) = test_host();
        let controller = host.open_settings();
        let applied = host.close_settings(&controller).expect("close succeeds");
        assert!(!applied);
        assert!(!host.host.owns_pause(PauseOwner::Settings));
    }

    #[test]
    fn resign_active_arms_without_core_crash() {
        let (mut host, _) = test_host();
        host.on_active(false);
        host.turn();
        // Coreless: the protocol runs, but no pause lands without a core.
        assert!(!host.host.owns_pause(PauseOwner::Focus));
        host.on_active(true);
        host.turn();
        assert!(!host.host.owns_pause(PauseOwner::Focus));
    }

    #[test]
    fn unknown_key_is_ignored() {
        let (mut host, _) = test_host();
        assert!(!host.on_key(999, true));
    }

    #[test]
    fn shortcut_debugger_reaches_surface() {
        let (mut host, mut surface) = test_host();
        assert!(host.on_shortcut(KeyboardShortcut::OpenDebugger, &mut surface));
        assert_eq!(surface.debugger_requests, 1);
    }

    #[test]
    fn title_push_throttles_to_interval() {
        let (mut host, mut surface) = test_host();
        host.push_title(&mut surface);
        host.push_title(&mut surface);
        assert_eq!(surface.titles.len(), 1);
    }

    #[test]
    fn open_missing_rom_reports_error() {
        let (mut host, _) = test_host();
        assert!(host.load_path(Path::new("/nonexistent/test.nes")).is_err());
    }

    #[test]
    fn turn_runs_shared_ordering() {
        let (mut host, _) = test_host();
        host.on_active(false);
        let _outcome = host.turn();
        // Coreless commit lands nothing, but the turn completes.
        assert!(!host.host.owns_pause(PauseOwner::Focus));
    }
}
