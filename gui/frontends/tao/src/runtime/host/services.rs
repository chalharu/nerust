//! Narrow service ports behind the [`HostState`](super::HostState) composer.
//! Each domain owns its state; the host only routes through these traits.

use super::{FrameCtx, HostAction, HostState};
use crate::aux::focus::AuxKind;

/// Main-window service: native handle, liveness, and surface targets.
pub(crate) trait MainWindowService {
    /// Run `f` on the usable native window, if any.
    fn with_main_window<R>(&self, f: impl FnOnce(&tao::window::Window) -> R) -> Option<R>;
    /// Native attach target for the game renderer, if usable.
    fn surface_target(
        &self,
    ) -> Option<(
        raw_window_handle::RawWindowHandle,
        nerust_render_traits::SurfaceSize,
        Option<u64>,
    )>;
}

/// Chrome service: menu, title, and fullscreen presentation sync.
pub(crate) trait ChromeService {
    /// One choke point: menu, title, and optionally a frame.
    fn sync_chrome(&mut self, ctx: &FrameCtx, redraw: bool);
    /// Title-only gate over the cached session title.
    fn refresh_window_title(&mut self, tick: &nerust_gui_shell::session::osd::TickMetrics);
}

/// Debug orchestration service: aux windows plus in-flight async flights.
pub(crate) trait DebugOrchestratorService {
    /// Open the debugger (idempotent); the PPU viewer for toolbar focus.
    fn open_debug_windows(&mut self, opener: &dyn crate::ports::WindowOpener, focus_ppu: bool);
    /// Debugger batch take (`None` when the debugger is closed).
    fn take_debug_batch(&mut self) -> Option<(bool, Vec<nerust_gui_shell::debug::DebugAction>)>;
    /// Whether a kind of aux window is currently open.
    fn is_aux_open(&self, kind: AuxKind) -> bool;
}

/// OSD service: overlay layer sync plus modal and history queues.
pub(crate) trait OsdServicePort {
    /// Drain shell notifications into display state.
    fn poll_notifications(&mut self, ctx: &FrameCtx);
    /// Queued error batches awaiting a dialog.
    fn modal_pending(&self) -> usize;
}

/// Settings service: dialog lifecycle plus snapshot applies.
pub(crate) trait SettingsService {
    /// Whether the settings window is currently open.
    fn settings_open(&self) -> bool;
    /// Open the settings dialog (idempotent).
    fn open_settings(&mut self, opener: &dyn crate::ports::WindowOpener);
    /// Router result for one menu command.
    fn on_menu_command(
        &mut self,
        command: crate::event::MenuCommand,
        opener: &dyn crate::ports::WindowOpener,
    ) -> HostAction;
}

impl MainWindowService for HostState {
    fn with_main_window<R>(&self, f: impl FnOnce(&tao::window::Window) -> R) -> Option<R> {
        HostState::with_main_window(self, f)
    }

    fn surface_target(
        &self,
    ) -> Option<(
        raw_window_handle::RawWindowHandle,
        nerust_render_traits::SurfaceSize,
        Option<u64>,
    )> {
        HostState::surface_target(self)
    }
}

impl ChromeService for HostState {
    fn sync_chrome(&mut self, ctx: &FrameCtx, redraw: bool) {
        HostState::sync_chrome(self, ctx, redraw);
    }

    fn refresh_window_title(&mut self, tick: &nerust_gui_shell::session::osd::TickMetrics) {
        HostState::refresh_window_title(self, tick);
    }
}

impl DebugOrchestratorService for HostState {
    fn open_debug_windows(&mut self, opener: &dyn crate::ports::WindowOpener, focus_ppu: bool) {
        HostState::open_debug_windows(self, opener, focus_ppu);
    }

    fn take_debug_batch(&mut self) -> Option<(bool, Vec<nerust_gui_shell::debug::DebugAction>)> {
        self.aux.debug_take_requests()
    }

    fn is_aux_open(&self, kind: AuxKind) -> bool {
        self.aux.contains(kind)
    }
}

impl OsdServicePort for HostState {
    fn poll_notifications(&mut self, ctx: &FrameCtx) {
        HostState::poll_notifications(self, ctx);
    }

    fn modal_pending(&self) -> usize {
        self.osd.modal_queue_len()
    }
}

impl SettingsService for HostState {
    fn settings_open(&self) -> bool {
        HostState::settings_open(self)
    }

    fn open_settings(&mut self, opener: &dyn crate::ports::WindowOpener) {
        HostState::open_settings_window(self, opener);
    }

    fn on_menu_command(
        &mut self,
        command: crate::event::MenuCommand,
        opener: &dyn crate::ports::WindowOpener,
    ) -> HostAction {
        HostState::on_menu_command(self, command, opener)
    }
}

#[cfg(test)]
mod tests {
    use super::super::TestHost;
    use super::*;
    use crate::test_opener::{FakeOpener, OpenerCall};

    #[test]
    fn god_split_delegation_routes_per_service() {
        let support = TestHost::new();
        let mut host = support.host;
        // Main window: no window headless, so the port stays total.
        assert!(MainWindowService::with_main_window(&host, |_| ()).is_none());
        assert!(MainWindowService::surface_target(&host).is_none());
        // Chrome: menu sync lands through the chrome service.
        let ctx = host.frame_ctx();
        ChromeService::sync_chrome(&mut host, &ctx, false);
        assert_eq!(support.menu.updates().len(), 1);
        // Second pass with the same capture is a no-op (lazy gates hold).
        let ctx = host.frame_ctx();
        assert!(std::sync::Arc::ptr_eq(&ctx, &host.frame_ctx()));
        ChromeService::sync_chrome(&mut host, &ctx, false);
        assert_eq!(support.menu.updates().len(), 1);
        // Debug: nothing open headless; batch take stays empty.
        assert!(!DebugOrchestratorService::is_aux_open(
            &host,
            AuxKind::Debug
        ));
        assert!(DebugOrchestratorService::take_debug_batch(&mut host).is_none());
        // OSD: idle poll touches nothing.
        let ctx = host.frame_ctx();
        OsdServicePort::poll_notifications(&mut host, &ctx);
        assert_eq!(OsdServicePort::modal_pending(&host), 0);
        // Settings: closed headless.
        assert!(!SettingsService::settings_open(&host));
        let opener = FakeOpener::failing("no display");
        SettingsService::open_settings(&mut host, &opener);
        assert_eq!(opener.calls(), [OpenerCall::Settings]);
        assert!(!SettingsService::settings_open(&host));
    }

    #[test]
    fn frame_ctx_versioning_reuses_arc_on_hot_path() {
        let support = TestHost::new();
        let mut host = support.host;
        let first = host.frame_ctx();
        let second = host.frame_ctx();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(first.generation, second.generation);
    }

    #[test]
    fn frame_ctx_version_bumps_on_settings_change() {
        let support = TestHost::new();
        let mut host = support.host;
        let first = host.frame_ctx();
        let mut snapshot = host.session().settings_snapshot().clone();
        snapshot.shared.general.language =
            nerust_settings_core::model::language::AppLanguage::Japanese;
        host.apply_settings(snapshot)
            .expect("headless settings apply");
        let second = host.frame_ctx();
        assert!(!std::sync::Arc::ptr_eq(&first, &second));
        assert!(second.generation > first.generation);
        assert_eq!(
            second.settings.shared.general.language,
            nerust_settings_core::model::language::AppLanguage::Japanese
        );
    }

    #[test]
    fn title_service_gates_repeats() {
        let support = TestHost::new();
        let mut host = support.host;
        let ctx = host.frame_ctx();
        let title = host.session_mut().window_title_cached(&ctx.tick).to_owned();
        assert!(!host.chrome.title_same(&title));
        ChromeService::refresh_window_title(&mut host, &ctx.tick);
        assert!(host.chrome.title_same(&title));
        ChromeService::refresh_window_title(&mut host, &ctx.tick);
        assert!(host.chrome.title_same(&title));
    }

    #[test]
    fn menu_command_service_delegates() {
        use crate::event::MenuCommand;
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        assert_eq!(
            SettingsService::on_menu_command(&mut host, MenuCommand::Quit, &opener),
            crate::runtime::host::HostAction::Exit
        );
        assert!(host.close_exited());
    }

    #[test]
    fn debug_service_open_fails_closed_headless() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        DebugOrchestratorService::open_debug_windows(&mut host, &opener, false);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
        assert!(!DebugOrchestratorService::is_aux_open(
            &host,
            AuxKind::Debug
        ));
        assert!(DebugOrchestratorService::take_debug_batch(&mut host).is_none());
    }
}
