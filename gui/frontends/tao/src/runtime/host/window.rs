//! Native window cluster, surface targets, and loop control flow.

use super::HostState;
use super::{
    create_window_builder, default_fit_window_size, logical_size_from_remembered,
    logical_window_size, window_surface_size,
};
use nerust_gui_runtime::shell::NativeShellState;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tao::{dpi::LogicalSize as TaoLogicalSize, event_loop::ControlFlow, window::WindowId};

use nerust_gui_shell::settings::scaling_factor;
use nerust_render_traits::{SurfaceSize, renderer::RenderResult};
use nerust_settings_core::model::app_state::RememberedWindowSize;
use raw_window_handle::RawWindowHandle;

use crate::ports::WindowOpener;

/// Main window cluster: handle, liveness, surface target, fullscreen sync.
/// All primary-window state lives here; `HostState` dispatches to it.
pub(crate) struct WindowCluster {
    window: Option<Arc<tao::window::Window>>,
    close: super::close::CloseGuard,
    pending_fullscreen_sync: Option<bool>,
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
            close: super::close::CloseGuard::new(),
            pending_fullscreen_sync: None,
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

    /// Record a window-creation failure and retire liveness.
    pub(super) fn fail(&mut self, detail: String) {
        self.close.fail(detail);
    }

    /// Creation failure, if any.
    pub(super) fn fatal_message(&self) -> Option<&str> {
        self.close.fatal_message()
    }

    /// Idempotent close entry: true exactly once.
    pub(super) fn begin_close(&mut self) -> bool {
        self.close.begin_close()
    }

    /// Close teardown already ran (test read of the guard).
    #[cfg(test)]
    pub(super) fn close_exited(&self) -> bool {
        self.close.exited()
    }

    /// Pending native fullscreen transition, if one is in flight.
    pub(super) fn pending_fullscreen_sync(&self) -> Option<bool> {
        self.pending_fullscreen_sync
    }

    pub(super) fn set_pending_fullscreen_sync(&mut self, pending: Option<bool>) {
        self.pending_fullscreen_sync = pending;
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
        if !self.close.is_usable() {
            return None;
        }
        self.window.as_ref().map(|w| f(w))
    }

    /// Drop the native handle after close teardown: post-loop code
    /// re-resolves through liveness, never a stale `Arc`.
    pub(super) fn close_main(&mut self) {
        self.window.take();
    }

    /// Shared attach-target resolution: content area below the menubar, else toplevel.
    fn attach_target(&self) -> Option<(RawWindowHandle, SurfaceSize, Option<u64>)> {
        if !self.close.is_usable() {
            return None;
        }
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

    pub(super) fn request_redraw(&mut self, shell: &mut NativeShellState) {
        // Liveness gates all dead-handle use: check first, so closed
        // or failed hosts never flag a redraw that cannot land.
        if !self.close.is_usable() {
            log::trace!("tao redraw dropped: no usable window");
            return;
        }
        shell.needs_redraw = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

/// Per-pump wake contribution. Each `tick_*` pump returns one;
/// `update_control_flow` folds them into a single [`WakeNeed`].
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WakeNeed {
    pub live_tick: Option<Instant>,
    pub flight_pending: bool,
    pub osd_deadline: Option<Instant>,
}

impl WakeNeed {
    pub(crate) fn fold(self, other: Self) -> Self {
        Self {
            live_tick: match (self.live_tick, other.live_tick) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, b) => b,
            },
            flight_pending: self.flight_pending || other.flight_pending,
            osd_deadline: match (self.osd_deadline, other.osd_deadline) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, b) => b,
            },
        }
    }
}

/// Poll latency for an in-flight drain/live fetch: the backstop that
/// bounds reply latency when the loop would otherwise sleep.
pub(crate) fn flight_backstop(now: Instant) -> Instant {
    now + std::time::Duration::from_millis(30)
}

/// Earliest deadline wins; no deadline sleeps. Pure (no tao import);
/// the runtime maps the result to `ControlFlow`.
pub(crate) fn earliest_wake(input: WakeNeed, now: Instant) -> Option<Instant> {
    [
        input.live_tick,
        input.flight_pending.then(|| flight_backstop(now)),
        input.osd_deadline,
    ]
    .into_iter()
    .flatten()
    .min()
}

/// Pure control-flow decision with injected clock: wake deadline plus repaint flag.
/// Repaint while focused, while the PPU follows a running core, or while the session runs.
pub(crate) fn update_flow(
    now: Instant,
    tick_loaded: bool,
    tick_paused: bool,
    main_active: bool,
    live_follow: bool,
    input: WakeNeed,
) -> (Option<Instant>, bool) {
    let running = tick_loaded && !tick_paused;
    (
        earliest_wake(input, now),
        main_active || live_follow || running,
    )
}

impl HostState {
    pub(crate) fn ensure_window(&mut self, opener: &dyn WindowOpener) {
        if self.windows.is_some() {
            return;
        }

        let fullscreen = self
            .session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        let tick = self.session.tick_metrics();
        let title = self.session.window_title_cached(&tick).to_owned();
        let window = match opener.create_main(create_window_builder(
            self.startup_window_size(),
            title,
            fullscreen,
        )) {
            Ok(window) => window,
            Err(detail) => {
                log::error!("failed to create window: {detail}");
                self.windows.fail(detail);
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
        self.windows.surface_target().map(|(_, size, _)| size)
    }

    /// Native attach target for the game renderer: content area below the menubar, else toplevel.
    /// The xid tags the target so a target change recreates the surface.
    pub(crate) fn surface_target(&self) -> Option<(RawWindowHandle, SurfaceSize, Option<u64>)> {
        self.windows.surface_target()
    }

    /// Xid the surface is currently attached to (`None` = toplevel fallback).
    pub(crate) fn surface_xid(&self) -> Option<u64> {
        self.windows.surface_xid()
    }

    /// Load a ROM, syncing menu/title/redraw on success. Returns the failure detail on error.
    pub(crate) fn load_path(&mut self, path: &Path) -> Result<(), String> {
        let mut target =
            nerust_gui_shell::session::load_adapter::SessionLoadAdapter::new(&mut self.session);
        match self.ctx.rom_loader.load_rom(path, &mut target) {
            Ok(()) => {
                let ctx = self.frame_ctx();
                self.after_rom_load(&ctx);
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
        if let Some(detail) = self.windows.fatal_message() {
            log::error!("exiting after window creation failure: {detail}");
            *control_flow = ControlFlow::Exit;
            return;
        }
        // End-of-burst auto-pause first: pause state settles before
        // the title/live/drain logic below reads it.
        self.tick_focus_sync();
        // One capture for the whole pass: chrome, debug, and OSD pumps share it.
        let ctx = self.frame_ctx();
        self.tick_chrome_sync(&ctx);
        // Deferred focus effects land here: one gated sync covers menu,
        // title, and drain-due instead of per-event round-trips.
        self.sync_debug_chrome(&ctx);
        let wake = self.tick_debug_pump().fold(self.tick_osd_pump(&ctx));
        let tick = ctx.tick();
        let now = Instant::now();
        let (deadline, wants_redraw) = update_flow(
            now,
            tick.loaded,
            tick.paused,
            self.main_active(),
            self.live_follow_active(),
            wake,
        );
        *control_flow = deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil);

        // Inactive loop sleeps; a running session repaints even unfocused.
        if wants_redraw {
            self.request_redraw();
        }
    }

    pub(crate) fn on_render_result(&mut self, result: RenderResult) {
        match result {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.tick_metrics().frame_counter);
                let ctx = self.frame_ctx();
                self.maybe_refresh_window_title(&ctx, Instant::now());
            }
            RenderResult::Skipped | RenderResult::Error => {}
        }
    }

    pub(crate) fn clear_event_handler(&self) {
        self.app_menu.clear_event_handler();
    }

    pub(crate) fn request_redraw(&mut self) {
        self.windows.request_redraw(&mut self.shell);
    }
    /// Idempotent close teardown: the guard makes flush run exactly once.
    /// Handles drop here: post-loop code re-resolves through liveness.
    pub(crate) fn prepare_close(&mut self) {
        if !self.windows.begin_close() {
            return;
        }
        self.remember_fit_window_size();
        // Every cluster drops its windows, sync memos, flights, and
        // resume flags: no history/modal/resume leaks past close.
        self.settings.close_all();
        self.osd.close_all();
        self.aux.close_all();
        self.debug.close_flights();
        self.windows.close_main();
        self.session.flush_before_exit();
    }
    pub(super) fn startup_window_size(&self) -> TaoLogicalSize<f64> {
        match scaling_factor(self.session.settings_snapshot().local.video.window.scaling) {
            Some(scale) => logical_window_size(self.session.window_size(), Some(scale)),
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
        let Some(scale) =
            scaling_factor(self.session.settings_snapshot().local.video.window.scaling)
        else {
            return;
        };
        window.set_inner_size(logical_window_size(self.session.window_size(), Some(scale)));
    }

    fn remembered_fit_window_size(&self) -> Option<RememberedWindowSize> {
        self.session
            .settings_snapshot()
            .app_state
            .window_size("main")
    }

    pub(super) fn remember_fit_window_size(&self) {
        let Some(window) = self.windows.live() else {
            return;
        };
        if window.fullscreen().is_some()
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

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use tao::event_loop::ControlFlow;

    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};

    #[test]
    fn with_usable_stays_total_without_window() {
        use super::WindowCluster;
        let mut cluster = WindowCluster::new();
        assert!(cluster.with_usable(|_| ()).is_none());
        assert_eq!(cluster.client_height(), 0);
        cluster.fail("no display".to_string());
        assert!(cluster.with_usable(|_| ()).is_none());
        assert!(cluster.fatal_message().is_some());
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
        host.on_render_result(RenderResult::Presented);
        host.on_render_result(RenderResult::Skipped);
        host.on_render_result(RenderResult::Error);
        assert!(host.startup_window_size().width > 0.0);
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn fullscreen_sync_target_round_trips() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(host.windows.pending_fullscreen_sync().is_none());
        host.windows.set_pending_fullscreen_sync(Some(true));
        assert_eq!(host.windows.pending_fullscreen_sync(), Some(true));
        host.windows.set_pending_fullscreen_sync(None);
        assert!(host.windows.pending_fullscreen_sync().is_none());
        host.windows.close_main();
        assert!(!host.windows.is_some());
    }

    #[test]
    fn cluster_close_arms_exactly_once() {
        let support = TestHost::new();
        let mut host = support.host;
        assert!(host.windows.begin_close());
        assert!(!host.windows.begin_close());
        assert!(host.windows.close_exited());
    }

    #[test]
    fn redraw_flag_sets_without_native_window() {
        use nerust_render_traits::renderer::RenderResult;
        let support = TestHost::new();
        let mut host = support.host;
        // Presented clears the flag; the headless redraw re-arms it.
        host.on_render_result(RenderResult::Presented);
        assert!(!host.shell.needs_redraw);
        host.request_redraw();
        assert!(host.shell.needs_redraw);
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
