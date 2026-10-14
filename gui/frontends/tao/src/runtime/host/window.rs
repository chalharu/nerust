//! Native window lifecycle, surface targets, and loop control flow.

use super::HostState;
use super::*;

impl HostState {
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
        let window = match create_window_builder(
            self.startup_window_size(),
            self.session.window_title(),
            fullscreen,
        )
        .build(event_loop)
        {
            Ok(window) => Arc::new(window),
            Err(e) => {
                log::error!("failed to create window: {e}");
                self.fatal_error = Some(e.to_string());
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
}
