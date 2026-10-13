mod debug_drain;
mod debug_windows;
mod host;

use std::path::Path;

use nerust_gui_shell::context::FrontendContext;
use nerust_render_traits::{
    SurfaceSize,
    renderer::{GpuRenderer, RendererConfig},
};
use raw_window_handle::HasDisplayHandle;
#[cfg(target_os = "macos")]
use tao::platform::macos::EventLoopExtMacOS;
use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
};

use self::host::{HostAction, HostState};
use crate::app_menu::{UserEvent, imp::AppMenu};

pub(crate) struct WindowRuntime {
    event_loop: Option<EventLoop<UserEvent>>,
    host: HostState,
    renderer: Option<Box<dyn GpuRenderer>>,
    /// Native target the surface is attached to (`None` = toplevel).
    /// The attach binds a drawable, so a target change needs a
    /// recreate, not a resize.
    surface_xid: Option<u64>,
}

impl WindowRuntime {
    pub(crate) fn new(ctx: FrontendContext) -> Self {
        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        #[cfg(target_os = "macos")]
        let event_loop = {
            let mut event_loop = event_loop;
            event_loop.set_activate_ignoring_other_apps(true);
            event_loop
        };
        let proxy = event_loop.create_proxy();
        let app_menu = AppMenu::new(proxy);

        let host = HostState::new(ctx, app_menu);

        Self {
            event_loop: Some(event_loop),
            host,
            renderer: None,
            surface_xid: None,
        }
    }

    fn build_renderer_and_surface(&mut self, window: &tao::window::Window) -> Option<()> {
        let (raw_window_handle, size, xid) = self.host.surface_target()?;
        let session = self.host.session();
        let vsync = session.settings_snapshot().local.video.presentation.vsync;
        let raw_display_handle = window
            .display_handle()
            .expect("failed to get display handle")
            .as_raw();
        let render_profile = session.render_profile().cloned()?;
        let config = RendererConfig {
            render_profile,
            vsync,
        };
        let mut renderer = self
            .host
            .gpu_factory()
            .create_renderer(&config, raw_display_handle)
            .expect("failed to create renderer");
        renderer
            .attach(
                raw_window_handle,
                raw_display_handle,
                SurfaceSize::new(size.width, size.height),
            )
            .expect("failed to attach");
        self.surface_xid = xid;
        self.renderer = Some(renderer);
        Some(())
    }

    pub(crate) fn load_path(&mut self, path: &Path) -> bool {
        match self.host.load_path(path) {
            Ok(()) => {
                self.recreate_renderer();
                true
            }
            Err(detail) => {
                // Startup runs before the main window exists, so the
                // standalone dialog is the only explanation for a bad
                // `--rom` path.
                host::show_rom_load_error(
                    self.host
                        .session()
                        .settings_snapshot()
                        .shared
                        .general
                        .language,
                    &detail,
                );
                false
            }
        }
    }

    pub(crate) fn run(mut self) {
        self.host.resume_session();
        let event_loop = self.event_loop.take().unwrap();

        // Live-follow pulse: a helper thread nudges the loop every
        // interval so the PPU viewer tracks a running core even when
        // no input or redraw events arrive (a sleeping `Wait` loop
        // would otherwise collapse the cadence to zero). The pulse
        // handler is throttle-guarded and no-ops when inactive; the
        // thread exits once the loop is gone.
        let tick_proxy = event_loop.create_proxy();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(250));
                if tick_proxy.send_event(UserEvent::LiveTick).is_err() {
                    break;
                }
            }
        });

        event_loop.run(move |event, event_loop, control_flow| match event {
            Event::NewEvents(StartCause::Init) => {
                self.host.ensure_window(event_loop);
                self.recreate_renderer();
                *control_flow = ControlFlow::Wait;
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.is_window(window_id) => match event {
                WindowEvent::CloseRequested if self.host.prepare_close() => {
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::Focused(true) => {
                    self.host.active = true;
                    if self.host.auto_paused() {
                        self.host.resume_session();
                        self.host.clear_auto_paused();
                    }
                    self.host.request_redraw();
                    // Keep the debugger toolbar truthful without
                    // re-hitting the emu thread when nothing moved.
                    self.host.sync_debug_chrome();
                }
                WindowEvent::Focused(false) => {
                    self.host.active = false;
                    if self.host.session().can_pause() {
                        self.host.pause_session();
                        self.host.set_auto_paused();
                    }
                    self.host.clear_keys();
                    // Same agreement in the other direction, still
                    // without redundant round-trips.
                    self.host.sync_debug_chrome();
                }
                WindowEvent::Resized(_) => {
                    self.host.sync_fullscreen_default_from_window();
                    self.host.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    self.host.on_keyboard_input(event);
                    if self.host.take_debug_open_request() {
                        self.host.open_debug_windows(event_loop, false);
                    }
                }
                _ => (),
            },
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.is_settings_window(window_id) => {
                // External destroy without CloseRequested: the window
                // is already gone, so skip event/render and run the
                // normal close path (pending apply still commits).
                if matches!(event, WindowEvent::Destroyed) {
                    if let Some(handle) = self.host.settings_window.take() {
                        let plan = self.host.close_settings_window(handle);
                        if plan.is_some_and(|p| p.renderer_needs_rebuild) {
                            self.recreate_renderer();
                        }
                        self.host.request_redraw();
                    }
                    return;
                }
                let Some(handle) = self.host.settings_window.as_mut() else {
                    return;
                };
                match &event {
                    WindowEvent::Resized(size) => handle.resize(size.width, size.height),
                    WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        handle.set_scale_factor(*scale_factor as f32);
                    }
                    WindowEvent::ModifiersChanged(state) => handle.set_modifiers(*state),
                    WindowEvent::CloseRequested => {
                        handle
                            .should_close
                            .store(true, std::sync::atomic::Ordering::Release);
                    }
                    _ => {}
                }
                handle.update_modifiers_from_tao_event(&event);
                handle.handle_tao_event(event);
                handle.render();
                if handle
                    .should_close
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    let handle = self.host.settings_window.take().unwrap();
                    let plan = self.host.close_settings_window(handle);
                    if plan.is_some_and(|p| p.renderer_needs_rebuild) {
                        self.recreate_renderer();
                    }
                    self.host.request_redraw();
                }
            }
            Event::RedrawRequested(window_id) if self.host.is_window(window_id) => self.on_update(),
            Event::RedrawRequested(window_id) if self.host.is_settings_window(window_id) => {
                if let Some(handle) = self.host.settings_window.as_mut() {
                    handle.render();
                }
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.debug.is_debug_window(window_id) => {
                if let WindowEvent::Moved(position) = &event {
                    self.host
                        .debug
                        .remember_debug_window_pos(position.x, position.y);
                }
                // Focus re-reads, never force-pauses: the main window's
                // auto-pause already paused when the debugger took
                // focus, and re-pausing here would surprise a running
                // session (and overwrite a meaningful status like
                // "stepped N" with a bare "paused"). The empty batch
                // still refreshes the toolbar label so it agrees with
                // the menu.
                // External destroy without CloseRequested: drop the
                // stale handle so a later F4 reopens instead of
                // silently no-op'ing on a dead window.
                let refocused = matches!(event, WindowEvent::Focused(true));
                if matches!(event, WindowEvent::Destroyed) {
                    self.host.debug.debug_window.take();
                    self.host.request_redraw();
                    return;
                }
                {
                    let Some(handle) = self.host.debug.debug_window.as_mut() else {
                        return;
                    };
                    handle.shell.handle_common_event(&event);
                    handle.handle_tao_event(event);
                }
                if refocused {
                    self.host.sync_debug_chrome();
                }
                self.host.drain_debug_requests();
                if self.host.take_ppu_open_request() {
                    self.host.open_debug_windows(event_loop, true);
                    self.host.run_debug_drain(&[]);
                }
                // Closing never resumes: resume is always explicit.
                if let Some(handle) = self.host.debug.debug_window.as_mut() {
                    handle.render();
                    if handle
                        .shell
                        .should_close
                        .load(std::sync::atomic::Ordering::Acquire)
                    {
                        self.host.debug.debug_window.take();
                        self.host.request_redraw();
                    }
                }
            }
            Event::RedrawRequested(window_id) if self.host.debug.is_debug_window(window_id) => {
                self.host.drain_debug_requests();
                if self.host.take_ppu_open_request() {
                    self.host.open_debug_windows(event_loop, true);
                    self.host.run_debug_drain(&[]);
                }
                if let Some(handle) = self.host.debug.debug_window.as_mut() {
                    handle.render();
                }
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.debug.is_ppu_window(window_id) => {
                // Same external-destroy guard as the debugger: a dead
                // handle must not linger and block reopening.
                if matches!(event, WindowEvent::Destroyed) {
                    self.host.debug.ppu_window.take();
                    self.host.request_redraw();
                    return;
                }
                if let WindowEvent::Moved(position) = &event {
                    self.host
                        .debug
                        .remember_ppu_window_pos(position.x, position.y);
                }
                let Some(handle) = self.host.debug.ppu_window.as_mut() else {
                    return;
                };
                handle.shell.handle_common_event(&event);
                handle.handle_tao_event(event);
                handle.render();
                if handle
                    .shell
                    .should_close
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    self.host.debug.ppu_window.take();
                    self.host.request_redraw();
                }
            }
            Event::RedrawRequested(window_id) if self.host.debug.is_ppu_window(window_id) => {
                if let Some(handle) = self.host.debug.ppu_window.as_mut() {
                    handle.render();
                }
            }
            Event::MainEventsCleared => self.host.update_control_flow(control_flow),
            Event::UserEvent(command) => match command {
                UserEvent::LiveTick => self.host.live_tick(),
                UserEvent::Menu(command) => {
                    let action = self.host.on_menu_command(command, event_loop);
                    match action {
                        HostAction::None => (),
                        HostAction::RomLoaded => self.recreate_renderer(),
                        HostAction::Exit => *control_flow = ControlFlow::Exit,
                    }
                }
            },
            Event::LoopDestroyed => self.host.clear_event_handler(),
            _ => (),
        });
    }

    fn on_update(&mut self) {
        let Some(window_size) = self.host.window_surface_size() else {
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        // Keep the output sized to the current target. A size change
        // resizes in place, but a target change (toplevel fallback to
        // content area once laid out) rebinds a drawable and needs a
        // full recreate.
        let current_xid = self.host.surface_xid();
        if self.surface_xid != current_xid {
            if let Some(window) = self.host.window().cloned() {
                self.build_renderer_and_surface(&window);
            }
            return;
        }
        if renderer.size() != window_size {
            renderer.resize(window_size);
        }

        self.host.session_mut().swap_frame_buffer();
        if let Some(fb) = self.host.session_mut().frame_buffer() {
            let result = renderer.render(fb);
            self.host.on_render_result(result);
        }
    }

    fn recreate_renderer(&mut self) {
        self.renderer = None;
        if let Some(window) = self.host.window().cloned() {
            self.build_renderer_and_surface(&window);
        }
    }
}

impl Drop for WindowRuntime {
    fn drop(&mut self) {
        self.renderer = None;
    }
}
