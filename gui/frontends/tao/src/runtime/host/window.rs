//! Native window lifecycle, surface targets, and loop control flow.

use super::HostState;
use super::{
    create_window_builder, default_fit_window_size, logical_size_from_remembered,
    logical_window_size, window_is_fullscreen, window_surface_size,
};
use std::path::Path;
use std::time::Instant;
use tao::{dpi::LogicalSize as TaoLogicalSize, event_loop::ControlFlow, window::WindowId};

use nerust_gui_shell::settings::scaling_factor;
use nerust_render_traits::{SurfaceSize, renderer::RenderResult};
use nerust_settings_core::model::app_state::RememberedWindowSize;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::aux::focus::AuxWindow;
use crate::ports::WindowOpener;

/// Wake inputs for one control-flow pass. Plain data, no tao import:
/// the host assembles it, [`earliest_wake`] folds it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WakeInput {
    pub live_tick: Option<Instant>,
    pub flight_pending: bool,
    pub osd_deadline: Option<Instant>,
}

/// Poll latency for an in-flight drain/live fetch: the backstop that
/// bounds reply latency when the loop would otherwise sleep.
pub(crate) fn flight_backstop(now: Instant) -> Instant {
    now + std::time::Duration::from_millis(30)
}

/// Earliest deadline wins; no deadline sleeps. Pure (no tao import);
/// the runtime maps the result to `ControlFlow`.
pub(crate) fn earliest_wake(input: WakeInput, now: Instant) -> Option<Instant> {
    [
        input.live_tick,
        input.flight_pending.then(|| flight_backstop(now)),
        input.osd_deadline,
    ]
    .into_iter()
    .flatten()
    .min()
}

/// Pure control-flow decision with injected clock: returns the wake
/// deadline plus whether the loop must repaint. The runtime maps the
/// deadline to `ControlFlow::{Wait, WaitUntil}`.
///
/// Repaint while focused, while the PPU follows a running core, or
/// while the session runs (a bare WaitUntil alone would not reliably
/// wake the loop, at the cost of one running frame render).
pub(crate) fn update_flow(
    now: Instant,
    tick_loaded: bool,
    tick_paused: bool,
    main_active: bool,
    live_follow: bool,
    input: WakeInput,
) -> (Option<Instant>, bool) {
    let running = tick_loaded && !tick_paused;
    (
        earliest_wake(input, now),
        main_active || live_follow || running,
    )
}

impl HostState {
    pub(crate) fn ensure_window(&mut self, opener: &dyn WindowOpener) {
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
        let window = match opener.create_main(create_window_builder(
            self.startup_window_size(),
            self.session.window_title(&self.session.tick_metrics()),
            fullscreen,
        )) {
            Ok(window) => window,
            Err(detail) => {
                log::error!("failed to create window: {detail}");
                self.close.fail(detail);
                return;
            }
        };
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
        self.window = Some(super::close::LiveWindow::new(window));
        self.sync_fullscreen_from_settings();
        self.sync_chrome(true);
    }

    pub(crate) fn is_window(&self, window_id: WindowId) -> bool {
        self.window
            .as_ref()
            .is_some_and(|window| window.id() == window_id)
    }

    pub(crate) fn is_settings_window(&self, window_id: WindowId) -> bool {
        self.settings
            .window
            .as_ref()
            .is_some_and(|h| h.tao_window_id() == window_id)
    }

    pub(crate) fn window_surface_size(&self) -> Option<SurfaceSize> {
        self.surface_target().map(|(_, size, _)| size)
    }

    /// Native attach target for the game renderer: the content area
    /// below the menubar when available, else the toplevel. The xid
    /// tags the target so a target change recreates the surface.
    /// Single helper: the content-area target or the toplevel
    /// fallback, so `surface_target` and `surface_xid` cannot drift.
    pub(crate) fn surface_target(&self) -> Option<(RawWindowHandle, SurfaceSize, Option<u64>)> {
        let (handle, size, xid) = self.attach_target()?;
        Some((handle, size, xid))
    }

    /// Xid the surface is currently attached to (`None` = toplevel
    /// fallback). Mirrors [`HostState::surface_target`] without
    /// rebuilding handles, for the runtime's recreate check.
    pub(crate) fn surface_xid(&self) -> Option<u64> {
        let (_, _, xid) = self.attach_target()?;
        xid
    }

    /// Shared attach-target resolution: content area below the menubar
    /// when it yields a native target (Wayland/pre-layout yields
    /// nothing), else the toplevel window with no xid.
    fn attach_target(&self) -> Option<(RawWindowHandle, SurfaceSize, Option<u64>)> {
        if !self.close.is_usable() {
            return None;
        }
        let window = self.window.as_ref()?;
        if let Some((handle, size, xid)) = self.content_target() {
            return Some((handle, size, Some(xid)));
        }
        let handle = window.raw().window_handle().ok()?.as_raw();
        Some((handle, window_surface_size(window.inner_size()), None))
    }

    /// Content-area target below the menubar (`None` where unsupported
    /// or not yet laid out). Linux/BSD only; elsewhere always `None`.
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

    /// Content-area target below the menubar (`None` where unsupported
    /// or not yet laid out). Linux/BSD only; elsewhere always `None`.
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

    /// Load a ROM, syncing menu/title/redraw on success. Returns
    /// the failure detail on error; presenting it (dialog) is the
    /// caller's job so the file-dialog and startup paths can differ.
    pub(crate) fn load_path(&mut self, path: &Path) -> Result<(), String> {
        let mut target =
            nerust_gui_shell::session::load_adapter::SessionLoadAdapter::new(&mut self.session);
        match self.ctx.rom_loader.load_rom(path, &mut target) {
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
        if let Some(detail) = self.close.fatal_message() {
            log::error!("exiting after window creation failure: {detail}");
            *control_flow = ControlFlow::Exit;
            return;
        }
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
        // While the PPU follows a running core, wake for the next
        // tick: plain Wait sleeps until the next input event, which
        // would collapse the live cadence to zero whenever the aux
        // windows (not the main one) hold focus.
        let live_follow = self.live_follow_active();
        let live_tick = if live_follow {
            self.next_live_tick()
        } else {
            None
        };
        // While a drain or live flight is pending, wake soon to poll
        // it: the reply applies within ~30ms instead of waiting for
        // the next input event.
        let flight_pending = drain_pending || self.live_flight_pending();

        // Drain notifications here, not only in on_update: a paused
        // unfocused loop schedules no redraws, which is exactly when
        // pause/error notices must still land and expire. One capture
        // for the whole pass, threaded through.
        let frame = self.frame_snapshot();
        self.poll_notifications(&frame);
        // Post-pump the deadline implies a showing label (promotion
        // is eager), so one call drives the expiry redraw and the
        // wait: plain Wait would leave a stale label up otherwise.
        let osd_deadline = self.osd_wakeup();
        if osd_deadline.is_some() {
            // Redraw while labels show so expiry applies even paused.
            self.request_redraw();
        }
        let tick = self.session.tick_metrics();
        let now = Instant::now();
        let (deadline, wants_redraw) = update_flow(
            now,
            tick.loaded,
            tick.paused,
            self.main_active(),
            live_follow,
            WakeInput {
                live_tick,
                flight_pending,
                osd_deadline,
            },
        );
        *control_flow = deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil);

        // Inactive loop sleeps; repaint a running session even
        // unfocused (e.g. operating the debugger while the game runs):
        // frames keep coming and must show. Paused, unfocused, and
        // unfollowed sleeps.
        if wants_redraw {
            self.request_redraw();
        }
    }

    pub(crate) fn on_render_result(&mut self, result: RenderResult) {
        match result {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.tick_metrics().frame_counter);
                self.maybe_refresh_window_title(Instant::now());
            }
            RenderResult::Skipped | RenderResult::Error => {}
        }
    }

    pub(crate) fn clear_event_handler(&self) {
        self.app_menu.clear_event_handler();
    }

    pub(crate) fn request_redraw(&mut self) {
        // Liveness gates all dead-handle use: check first, so closed
        // or failed hosts never flag a redraw that cannot land.
        if !self.close.is_usable() {
            log::trace!("tao redraw dropped: no usable window");
            return;
        }
        self.shell.needs_redraw = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
    /// Idempotent close teardown: the guard makes flush run exactly
    /// once across CloseRequested + Destroyed. Handles stay alive:
    /// dropping tao/GTK objects inside a destroy handler re-enters
    /// the dying native window (BadWindow, exit 1); natural teardown
    /// after loop exit already reclaims them (exit 0). Liveness is
    /// tracked separately so no dead handle is ever touched.
    pub(crate) fn prepare_close(&mut self) {
        if !self.close.begin_close() {
            return;
        }
        self.remember_fit_window_size();
        // Every cluster drops its windows, sync memos, flights, and
        // resume flags: no history/modal/resume leaks past close.
        self.settings.close_all();
        self.osd.close_all();
        self.debug.close_all();
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

    fn remembered_fit_window_size(&self) -> Option<RememberedWindowSize> {
        self.session
            .settings_snapshot()
            .app_state
            .window_size("main")
    }

    pub(super) fn remember_fit_window_size(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window_is_fullscreen(window.raw())
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
    fn prepare_close_is_idempotent_and_clears_state() {
        let support = TestHost::new();
        let mut host = support.host;
        host.prepare_close();
        assert!(host.close_exited());
        assert!(!host.settings_open());
        // Liveness gates all dead-handle use; the native handles stay
        // for natural post-loop teardown (no re-entrant destroy).
        assert!(!host.close.is_usable());
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
    fn update_control_flow_waits_quietly_without_windows() {
        let support = TestHost::new();
        let mut host = support.host;
        let mut flow = ControlFlow::Wait;
        host.update_control_flow(&mut flow);
        assert_eq!(flow, ControlFlow::Wait);
        // No window, no flights, no follows: nothing to present.
        assert!(support.menu.updates().is_empty());
    }

    #[test]
    fn wake_policy_folds_to_earliest_deadline() {
        use super::{WakeInput, earliest_wake, flight_backstop, update_flow};
        let now = Instant::now();
        let idle = WakeInput::default();
        assert_eq!(earliest_wake(idle, now), None);
        // Flight poll alone wakes soon (30ms backstop).
        let flight = WakeInput {
            flight_pending: true,
            ..WakeInput::default()
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
        let raced = WakeInput {
            live_tick: Some(tick),
            flight_pending: true,
            osd_deadline: Some(soon_deadline),
        };
        assert_eq!(earliest_wake(raced, now), Some(soon_deadline));
        // Live tick earlier than the flight backstop survives.
        let early_tick = now + std::time::Duration::from_millis(5);
        let tick_wins = WakeInput {
            live_tick: Some(early_tick),
            flight_pending: true,
            osd_deadline: None,
        };
        assert_eq!(earliest_wake(tick_wins, now), Some(early_tick));
        // Pure flow decision with injected clock: any live source
        // repaints; quiet sleeps. The runtime maps the deadline to
        // ControlFlow.
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
}
