mod debug_windows;
mod dispatch;
mod drain;
pub(crate) mod host;

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
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
};

use self::host::{HostAction, HostState};
use crate::aux::focus::WindowHandler;
use crate::event::UserEvent;
use crate::menu::imp::AppMenu;
use crate::ports::{NativeDialogs, TaoWindowOpener};

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
    pub(crate) fn new(ctx: FrontendContext) -> Result<Self, String> {
        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        #[cfg(target_os = "macos")]
        let event_loop = {
            let mut event_loop = event_loop;
            event_loop.set_activate_ignoring_other_apps(true);
            event_loop
        };
        let proxy = event_loop.create_proxy();
        let app_menu = AppMenu::new(proxy);

        let host = HostState::new(ctx, Box::new(app_menu), std::sync::Arc::new(NativeDialogs))?;

        Ok(Self {
            event_loop: Some(event_loop),
            host,
            renderer: None,
            surface_xid: None,
        })
    }

    fn build_renderer_and_surface(&mut self, window: &tao::window::Window) -> Option<()> {
        let (raw_window_handle, size, xid) = self.host.surface_target()?;
        let session = self.host.session();
        let vsync = session.settings_snapshot().local.video.presentation.vsync;
        let raw_display_handle = match window.display_handle() {
            Ok(handle) => handle.as_raw(),
            Err(e) => {
                log::error!("failed to get display handle: {e}");
                return None;
            }
        };
        let render_profile = session.render_profile().cloned()?;
        let config = RendererConfig {
            render_profile,
            vsync,
            max_osd_upload_retries: nerust_render_traits::renderer::DEFAULT_OSD_UPLOAD_RETRIES,
        };
        // Drop the old renderer before the new attach makes its own
        // context current: GL object names die with their context, and
        // a fresh context reuses the same ids.
        self.renderer = None;
        let mut renderer = match self
            .host
            .gpu_factory()
            .create_renderer(&config, raw_display_handle)
        {
            Ok(renderer) => renderer,
            Err(e) => {
                log::error!("failed to create renderer: {e}");
                return None;
            }
        };
        if let Err(e) = renderer.attach(
            raw_window_handle,
            raw_display_handle,
            SurfaceSize::new(size.width, size.height),
        ) {
            log::error!("failed to attach renderer: {e}");
            return None;
        }
        self.surface_xid = xid;
        self.renderer = Some(renderer);
        // Fresh view holds no layer; re-push on the next sync
        // instead of skipping on a stale fingerprint.
        self.host.note_renderer_replaced();
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
                crate::ports::show_rom_load_error(
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
        let event_loop = self
            .event_loop
            .take()
            .expect("WindowRuntime::run() called more than once");

        // Live-follow pulse: nudges the loop each interval so the PPU
        // viewer tracks a running core with no input events. Throttled;
        // exits when the loop is gone.
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
                let opener = TaoWindowOpener::new(event_loop);
                self.host.ensure_window(&opener);
                self.recreate_renderer();
                *control_flow = ControlFlow::Wait;
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.is_window(window_id) => {
                let opener = TaoWindowOpener::new(event_loop);
                self.on_main_event(&opener, event, control_flow);
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.is_settings_window(window_id) => {
                self.on_settings_event(event);
            }
            Event::RedrawRequested(window_id) if self.host.is_window(window_id) => self.on_update(),
            Event::RedrawRequested(window_id) if self.host.is_settings_window(window_id) => {
                if let Some(handle) = self.host.settings_window_mut() {
                    handle.render();
                }
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.debug.is_debug_window(window_id) => {
                let opener = TaoWindowOpener::new(event_loop);
                self.on_debug_event(&opener, event);
            }
            Event::RedrawRequested(window_id) if self.host.debug.is_debug_window(window_id) => {
                let opener = TaoWindowOpener::new(event_loop);
                self.on_debug_redrawn(&opener);
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.debug.is_ppu_window(window_id) => {
                self.on_ppu_event(event);
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.is_history_window(window_id) => {
                self.host.on_history_event(event);
            }
            Event::RedrawRequested(window_id) if self.host.is_history_window(window_id) => {
                if let Some(handle) = self.host.history_window_mut() {
                    handle.render();
                }
            }
            Event::RedrawRequested(window_id) if self.host.debug.is_ppu_window(window_id) => {
                if let Some(handle) = self.host.debug.ppu_window_mut() {
                    handle.render();
                }
            }
            Event::MainEventsCleared => self.host.update_control_flow(control_flow),
            Event::UserEvent(command) => match command {
                UserEvent::LiveTick => self.host.live_tick(),
                UserEvent::Menu(command) => {
                    let opener = TaoWindowOpener::new(event_loop);
                    let action = self.host.on_menu_command(command, &opener);
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

        // No notification drain here: `update_control_flow` drains
        // every pass (also paused/unfocused ones with no redraw).
        self.host.render_osd_frame(renderer.as_mut(), window_size);

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
