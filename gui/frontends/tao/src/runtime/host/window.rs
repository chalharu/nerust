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

use crate::ports::WindowOpener;

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
            self.session.window_title(),
            fullscreen,
        )) {
            Ok(window) => window,
            Err(detail) => {
                log::error!("failed to create window: {detail}");
                self.fatal_error = Some(detail);
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
        self.window = Some(window);
        self.sync_fullscreen_from_settings();
        self.sync_menu_state();
        if std::env::var("NERUST_NO_OSD").is_err() {
            self.ensure_osd_window(opener);
        }
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
            .is_some_and(|h| h.shell.window.id() == window_id)
    }

    pub(crate) fn window_surface_size(&self) -> Option<SurfaceSize> {
        self.surface_target().map(|(_, size, _)| size)
    }

    /// Native attach target for the game renderer: the content area
    /// below the menubar when available, else the toplevel. The xid
    /// tags the target so a target change recreates the surface.
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

    pub(crate) fn update_control_flow(&mut self, control_flow: &mut ControlFlow) {
        // Window creation failed: exit instead of running headless.
        if let Some(detail) = self.fatal_error.as_deref() {
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

        // Overlay tracks the main client area (see sync_osd_geometry).
        if self.osd_window.is_some() {
            self.sync_osd_geometry();
        }
        // OSD expiry wakes the loop for relabel: plain Wait would
        // leave a stale label up until the next input event.
        // Redraw while labels show so expiry applies even paused.
        if self.osd.has_visible()
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
        if let Some(deadline) = self.osd_wakeup() {
            let earlier = match *control_flow {
                ControlFlow::WaitUntil(at) => at.min(deadline),
                _ => deadline,
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
        if (self.focus.main_active() || live || running)
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

#[cfg(test)]
mod tests {
    use tao::event_loop::ControlFlow;

    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};

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
    fn armed_auto_pause_consumes_without_core() {
        use tao::event_loop::ControlFlow;
        let support = TestHost::new();
        let mut host = support.host;
        host.focus.leave_main();
        let mut flow = ControlFlow::Wait;
        host.update_control_flow(&mut flow);
        assert_eq!(flow, ControlFlow::Wait);
        // Consumed: arriving now reports no resume.
        assert!(!host.focus.arrive_main());
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
