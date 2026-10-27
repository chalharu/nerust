//! Native window cluster, surface targets, and loop control flow.

use super::HostState;
use super::{
    create_window_builder, default_fit_window_size, logical_size_from_remembered,
    logical_window_size, window_surface_size,
};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tao::{dpi::LogicalSize as TaoLogicalSize, event_loop::ControlFlow, window::WindowId};

use nerust_gui_shell::settings::scaling_factor;
use nerust_render_traits::{SurfaceSize, renderer::RenderResult};
use nerust_settings_core::model::app_state::RememberedWindowSize;
use raw_window_handle::RawWindowHandle;

use crate::ports::WindowOpener;

pub(crate) use nerust_frontend_host::loop_driver::WakeNeed;
use nerust_frontend_host::loop_driver::{LoopHost, TurnLiveness, drive_turn};
#[cfg(test)]
pub(crate) use nerust_frontend_host::loop_driver::{earliest_wake, flight_backstop, update_flow};

/// Native main-window handle and surface target.
/// ApplicationHost owns liveness, close, and fullscreen synchronization.
pub(crate) struct WindowCluster {
    window: Option<Arc<tao::window::Window>>,
    /// Linux render target below the muda menubar (`None` elsewhere).
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    content_area: Option<crate::gtk_embed::ContentArea>,
}

impl WindowCluster {
    pub(super) fn new() -> Self {
        Self {
            window: None,
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

    pub(super) fn is_some(&self) -> bool {
        self.window.is_some()
    }

    pub(super) fn live(&self) -> Option<&Arc<tao::window::Window>> {
        self.window.as_ref()
    }

    pub(super) fn is_window(&self, window_id: WindowId) -> bool {
        self.window
            .as_ref()
            .is_some_and(|window| window.id() == window_id)
    }

    /// Seat a newly created window, wiring the menubar content area.
    pub(super) fn seat(&mut self, window: Arc<tao::window::Window>) {
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
    }

    /// Native attach target for the game renderer: content area below the menubar, else toplevel.
    /// The xid tags the target so a target change recreates the surface.
    pub(super) fn surface_target(&self) -> Option<(RawWindowHandle, SurfaceSize, Option<u64>)> {
        let (handle, size, xid) = self.attach_target()?;
        Some((handle, size, xid))
    }

    /// Client height in physical pixels for OSD strip math.
    pub(super) fn client_height(&self) -> u32 {
        self.window
            .as_ref()
            .map(|w| w.inner_size().height)
            .unwrap_or(0)
    }

    /// Xid the surface is currently attached to (`None` = toplevel fallback).
    pub(super) fn surface_xid(&self) -> Option<u64> {
        let (_, _, xid) = self.attach_target()?;
        xid
    }

    /// Run `f` on the usable native window, if any.
    pub(super) fn with_usable<R>(&self, f: impl FnOnce(&tao::window::Window) -> R) -> Option<R> {
        self.window.as_ref().map(|w| f(w))
    }

    /// Drop the native handle after close teardown: post-loop code
    /// re-resolves through liveness, never a stale `Arc`.
    pub(super) fn close_main(&mut self) {
        self.window.take();
    }

    /// Shared attach-target resolution: content area below the menubar, else toplevel.
    fn attach_target(&self) -> Option<(RawWindowHandle, SurfaceSize, Option<u64>)> {
        let window = self.window.as_ref()?;
        if let Some((handle, size, xid)) = self.content_target() {
            return Some((handle, size, Some(xid)));
        }
        // Native attach handle plus current surface size, if available.
        use raw_window_handle::HasWindowHandle;
        let raw = window.window_handle().ok()?.as_raw();
        Some((raw, window_surface_size(window.inner_size()), None))
    }

    /// Content-area target below the menubar (`None` where unsupported or not yet laid out).
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    fn content_target(&self) -> Option<(RawWindowHandle, SurfaceSize, u64)> {
        self.content_area.as_ref()?.target()
    }

    /// Content-area target below the menubar (`None` where unsupported or not yet laid out).
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    fn content_target(&self) -> Option<(RawWindowHandle, SurfaceSize, u64)> {
        None
    }

    pub(super) fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

/// Native Tao ports for the shared end-of-burst driver.
impl LoopHost for HostState {
    type Frame = super::FrameCtx;

    fn before_turn(&mut self) {
        self.on_focus_event(nerust_frontend_host::focus::FocusEvent::BurstEnd);
    }

    fn frame_ctx(&mut self) -> Self::Frame {
        self.core.frame_ctx()
    }

    fn chrome(&mut self, ctx: &Self::Frame) {
        self.tick_chrome_sync(ctx);
        self.sync_debug_chrome(ctx);
    }

    fn pump_debug(&mut self) -> WakeNeed {
        self.tick_debug_pump()
    }

    fn pump_osd(&mut self, ctx: &Self::Frame) -> WakeNeed {
        self.tick_osd_pump(ctx)
    }

    fn pump_aux(&mut self) -> WakeNeed {
        if self.core.take_render_retry(Instant::now()) {
            self.request_redraw();
        }
        WakeNeed {
            osd_deadline: self
                .aux
                .pump_deadlines(Instant::now())
                .into_iter()
                .chain(self.core.render_retry_deadline())
                .min(),
            ..WakeNeed::default()
        }
    }

    fn late_liveness(&mut self) -> TurnLiveness {
        let tick = self.core.tick();
        TurnLiveness {
            loaded: tick.loaded,
            paused: tick.paused,
            main_active: self.main_active(),
            live_follow: self.live_follow_active(),
        }
    }

    fn request_redraw(&mut self) {
        HostState::request_redraw(self);
    }
}

impl HostState {
    pub(crate) fn ensure_window(&mut self, opener: &dyn WindowOpener) {
        if self.windows.is_some() || !self.core.window_usable() {
            return;
        }

        let fullscreen = self
            .core
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        let tick = self.core.tick();
        let title = self.core.cached_title(&tick);
        let window = match opener.create_main(create_window_builder(
            self.startup_window_size(),
            title,
            fullscreen,
        )) {
            Ok(window) => window,
            Err(detail) => {
                log::error!("failed to create window: {detail}");
                self.core.fail_window(detail);
                return;
            }
        };
        self.app_menu.init_for_window(&window);
        self.windows.seat(window);
        self.sync_fullscreen_from_settings();
        let ctx = self.frame_ctx();
        self.sync_chrome(&ctx, true);
    }

    pub(crate) fn is_window(&self, window_id: WindowId) -> bool {
        self.windows.is_window(window_id)
    }

    pub(crate) fn window_surface_size(&self) -> Option<SurfaceSize> {
        self.surface_target().map(|(_, size, _)| size)
    }

    /// Native attach target for the game renderer: content area below the menubar, else toplevel.
    /// The xid tags the target so a target change recreates the surface.
    pub(crate) fn surface_target(&self) -> Option<(RawWindowHandle, SurfaceSize, Option<u64>)> {
        self.core
            .window_usable()
            .then(|| self.windows.surface_target())
            .flatten()
    }

    /// Xid the surface is currently attached to (`None` = toplevel fallback).
    pub(crate) fn surface_xid(&self) -> Option<u64> {
        self.core
            .window_usable()
            .then(|| self.windows.surface_xid())
            .flatten()
    }

    /// Load a ROM, syncing menu/title/redraw on success. Returns the failure detail on error.
    pub(crate) fn load_path(&mut self, path: &Path) -> Result<(), String> {
        match self.core.load_path(self.ctx.rom_loader.as_mut(), path) {
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

    pub(crate) fn update_control_flow(&mut self, control_flow: &mut ControlFlow) {
        // Window creation failed: exit instead of running headless.
        if let Some(detail) = self.core.fatal_message() {
            log::error!("exiting after window creation failure: {detail}");
            *control_flow = ControlFlow::Exit;
            return;
        }
        let outcome = drive_turn(self);
        *control_flow = outcome
            .deadline
            .map_or(ControlFlow::Wait, ControlFlow::WaitUntil);
    }

    pub(crate) fn on_render_result(&mut self, result: RenderResult, frame_counter: u64) {
        match result {
            RenderResult::Presented => {
                self.core.render_succeeded();
                self.core.frame_presented(frame_counter);
                let ctx = self.frame_ctx();
                self.maybe_refresh_window_title(&ctx, Instant::now());
            }
            RenderResult::Error => {
                self.render_failed("Rendering failed; retrying with backoff".into());
            }
            RenderResult::Skipped => {}
        }
    }

    pub(crate) fn clear_event_handler(&self) {
        self.app_menu.clear_event_handler();
    }

    pub(crate) fn request_redraw(&mut self) {
        if !self.core.render_ready() || !self.core.window_usable() {
            return;
        }
        self.core.note_redraw();
        self.windows.request_redraw();
    }
    /// Idempotent close teardown: the guard makes flush run exactly once.
    /// Handles drop here: post-loop code re-resolves through liveness.
    pub(crate) fn prepare_close(&mut self) {
        if !self.core.begin_close() {
            return;
        }
        self.remember_fit_window_size();
        // Renderer teardown precedes this at the runtime boundary. Retire
        // application OSD is retired by begin_close; drop aux before debug
        // flights, main window, and session flush.
        self.aux.close_all();
        self.core.close_debug_flights();
        self.windows.close_main();
        self.core.finish_close();
    }
    pub(super) fn startup_window_size(&self) -> TaoLogicalSize<f64> {
        match scaling_factor(self.core.settings_snapshot().local.video.window.scaling) {
            Some(scale) => logical_window_size(self.core.session().window_size(), Some(scale)),
            None => self
                .remembered_fit_window_size()
                .map(logical_size_from_remembered)
                .unwrap_or_else(default_fit_window_size),
        }
    }

    pub(super) fn update_window_size_for_scaling(&self) {
        let Some(window) = self.windows.live() else {
            return;
        };
        let Some(scale) = scaling_factor(self.core.settings_snapshot().local.video.window.scaling)
        else {
            return;
        };
        window.set_inner_size(logical_window_size(
            self.core.session().window_size(),
            Some(scale),
        ));
    }

    fn remembered_fit_window_size(&self) -> Option<RememberedWindowSize> {
        self.core.settings_snapshot().app_state.window_size("main")
    }

    pub(super) fn remember_fit_window_size(&mut self) {
        let Some(window) = self.windows.live() else {
            return;
        };
        if window.fullscreen().is_some()
            || scaling_factor(self.core.settings_snapshot().local.video.window.scaling).is_some()
        {
            return;
        }

        let logical_size = window.inner_size().to_logical::<f64>(window.scale_factor());
        let width = logical_size.width.round().max(1.0) as u32;
        let height = logical_size.height.round().max(1.0) as u32;

        if let Err(error) = self.core.update_window_size(width, height) {
            log::warn!("failed to remember tao window size: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use tao::event_loop::ControlFlow;

    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};

    #[test]
    fn with_usable_stays_total_without_window() {
        use super::WindowCluster;
        let cluster = WindowCluster::new();
        assert!(cluster.with_usable(|_| ()).is_none());
        assert_eq!(cluster.client_height(), 0);
    }

    #[test]
    fn prepare_close_is_idempotent_and_clears_state() {
        let support = TestHost::new();
        let mut host = support.host;
        host.prepare_close();
        assert!(host.close_exited());
        assert!(!host.settings_open());
        // Liveness gates all dead-handle use; the native handles stay
        // for natural post-loop teardown (no re-entrant destroy).
        assert!(host.surface_target().is_none());
        assert!(host.surface_xid().is_none());
        // Second delivery (Destroyed after CloseRequested) is a
        // no-op: re-armed state proves the flush path ran only once.
        host.prepare_close();
        assert!(!host.settings_open());
    }

    #[test]
    fn ensure_window_failure_exits_loop() {
        let support = TestHost::new();
        let mut host = support.host;
        let opener = FakeOpener::failing("no display");
        host.ensure_window(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Main]);
        assert!(host.window().is_none());
        let mut flow = ControlFlow::Wait;
        host.update_control_flow(&mut flow);
        assert_eq!(flow, ControlFlow::Exit);
    }

    #[test]
    fn clear_event_handler_reaches_menu() {
        let support = TestHost::new();
        support.host.clear_event_handler();
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn update_control_flow_syncs_chrome_once_then_holds() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut flow = ControlFlow::Wait;
        // Deferred focus effects land in the burst: the first pass
        // pushes through the empty gate, the repeat holds.
        host.update_control_flow(&mut flow);
        assert_eq!(flow, ControlFlow::Wait);
        assert_eq!(support.menu.updates().len(), 1);
        host.update_control_flow(&mut flow);
        assert_eq!(flow, ControlFlow::Wait);
        assert_eq!(support.menu.updates().len(), 1);
        // No window, no flights, no follows: nothing to present.
    }

    #[test]
    fn wake_policy_folds_to_earliest_deadline() {
        use super::{WakeNeed, earliest_wake, flight_backstop, update_flow};
        let now = Instant::now();
        let idle = WakeNeed::default();
        assert_eq!(earliest_wake(idle, now), None);
        // Flight poll alone wakes soon (30ms backstop).
        let flight = WakeNeed {
            flight_pending: true,
            ..WakeNeed::default()
        };
        assert_eq!(
            earliest_wake(flight, now).map(|t| t.duration_since(now)),
            Some(std::time::Duration::from_millis(30))
        );
        assert_eq!(
            flight_backstop(now),
            now + std::time::Duration::from_millis(30)
        );
        // Earliest wins across all three sources.
        let tick = now + std::time::Duration::from_millis(250);
        let soon_deadline = now + std::time::Duration::from_millis(10);
        let raced = WakeNeed {
            live_tick: Some(tick),
            flight_pending: true,
            osd_deadline: Some(soon_deadline),
        };
        assert_eq!(earliest_wake(raced, now), Some(soon_deadline));
        // Live tick earlier than the flight backstop survives.
        let early_tick = now + std::time::Duration::from_millis(5);
        let tick_wins = WakeNeed {
            live_tick: Some(early_tick),
            flight_pending: true,
            osd_deadline: None,
        };
        assert_eq!(earliest_wake(tick_wins, now), Some(early_tick));
        // Pure flow decision: any live source repaints; quiet sleeps.
        let (deadline, redraw) = update_flow(now, true, false, false, false, idle);
        assert_eq!(deadline, None);
        assert!(redraw);
        let (deadline, redraw) = update_flow(now, false, false, false, false, idle);
        assert_eq!(deadline, None);
        assert!(!redraw);
        let (deadline, _) = update_flow(now, false, false, false, false, flight);
        assert_eq!(
            deadline.map(|t| t.duration_since(now)),
            Some(std::time::Duration::from_millis(30))
        );
    }

    #[test]
    fn armed_auto_pause_consumes_without_core() {
        use tao::event_loop::ControlFlow;
        let support = TestHost::new();
        let mut host = support.host;
        host.note_main_departure();
        let mut flow = ControlFlow::Wait;
        host.update_control_flow(&mut flow);
        assert_eq!(flow, ControlFlow::Wait);
        // Consumed: arriving now reports no resume.
        assert!(!host.note_main_arrival());
    }

    #[test]
    fn window_helpers_stay_total_without_window() {
        use nerust_render_traits::renderer::RenderResult;
        let support = TestHost::new();
        let mut host = support.host;
        assert!(host.window_surface_size().is_none());
        assert!(host.surface_target().is_none());
        assert!(host.surface_xid().is_none());
        host.update_window_size_for_scaling();
        host.remember_fit_window_size();
        host.toggle_fullscreen();
        host.on_render_result(RenderResult::Presented, 0);
        host.on_render_result(RenderResult::Skipped, 0);
        host.on_render_result(RenderResult::Error, 0);
        assert!(host.startup_window_size().width > 0.0);
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn redraw_flag_sets_without_native_window() {
        use nerust_render_traits::renderer::RenderResult;
        let support = TestHost::new();
        let mut host = support.host;
        // Presented clears the flag; the headless redraw re-arms it.
        host.on_render_result(RenderResult::Presented, 0);
        assert!(!host.redraw_needed());
        host.request_redraw();
        assert!(host.redraw_needed());
    }

    #[test]
    fn flow_decision_repaints_on_any_live_source() {
        use super::{WakeNeed, update_flow};
        let now = std::time::Instant::now();
        let idle = WakeNeed::default();
        // Focused main window repaints even while paused.
        let (_, redraw) = update_flow(now, true, true, true, false, idle);
        assert!(redraw);
        // PPU follow repaints while running unfocused.
        let (_, redraw) = update_flow(now, true, false, false, true, idle);
        assert!(redraw);
        // Loaded paused unfocused sleeps.
        let (_, redraw) = update_flow(now, true, true, false, false, idle);
        assert!(!redraw);
    }

    #[test]
    fn wake_fold_keeps_earliest_per_source() {
        use super::WakeNeed;
        let now = std::time::Instant::now();
        let early = now + std::time::Duration::from_millis(5);
        let late = now + std::time::Duration::from_millis(50);
        let folded = WakeNeed {
            live_tick: Some(late),
            ..WakeNeed::default()
        }
        .fold(WakeNeed {
            live_tick: Some(early),
            osd_deadline: Some(late),
            ..WakeNeed::default()
        });
        assert_eq!(folded.live_tick, Some(early));
        assert_eq!(folded.osd_deadline, Some(late));
        assert!(!folded.flight_pending);
        let pending = WakeNeed {
            flight_pending: true,
            ..WakeNeed::default()
        }
        .fold(WakeNeed::default());
        assert!(pending.flight_pending);
    }
}
