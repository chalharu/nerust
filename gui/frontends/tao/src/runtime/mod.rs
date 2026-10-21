mod debug_windows;
mod dispatch;
mod drain;
pub(crate) mod host;

use std::path::Path;

use nerust_gui_shell::context::FrontendContext;
use nerust_render_traits::{
    SurfaceSize,
    renderer::{RenderPipeline, RendererConfig},
};
use raw_window_handle::HasDisplayHandle;
#[cfg(target_os = "macos")]
use tao::platform::macos::EventLoopExtMacOS;
use tao::{
    event::{Event, StartCause},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
};

use self::host::{HostAction, HostState};
use crate::event::UserEvent;
use crate::menu::imp::AppMenu;
use crate::ports::{NativeDialogs, TaoWindowOpener};

pub(crate) struct WindowRuntime {
    event_loop: Option<EventLoop<UserEvent>>,
    host: HostState,
    renderer: Option<Box<dyn RenderPipeline>>,
    /// Native target the surface is attached to (`None` = toplevel).
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

    /// Live-follow pulse: parked with no wakeups unless a viewer
    /// wants live ticks; arming wakes it, disarming re-parks it.
    fn spawn_tick_thread(
        tick_proxy: tao::event_loop::EventLoopProxy<UserEvent>,
        tick_armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Option<std::thread::Thread> {
        let (thread_tx, thread_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = thread_tx.send(std::thread::current());
            use std::sync::atomic::Ordering;
            loop {
                if tick_armed.load(Ordering::Acquire) {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    if tick_proxy.send_event(UserEvent::LiveTick).is_err() {
                        break;
                    }
                } else {
                    std::thread::park();
                }
            }
        });
        thread_rx.recv().ok()
    }

    fn build_renderer_and_surface(
        &mut self,
        raw_display_handle: raw_window_handle::RawDisplayHandle,
    ) -> Option<()> {
        let (raw_window_handle, size, xid) = self.host.surface_target()?;
        let session = self.host.session();
        let vsync = session.settings_snapshot().local.video.presentation.vsync;
        let render_profile = session.render_profile().cloned()?;
        let config = RendererConfig {
            render_profile,
            vsync,
            max_osd_upload_retries: nerust_render_traits::renderer::DEFAULT_OSD_UPLOAD_RETRIES,
        };
        // Drop the old renderer before the new attach makes its own context current.
        self.renderer = None;
        let mut renderer = match self
            .host
            .gpu_factory()
            .create_pipeline(&config, raw_display_handle)
        {
            Ok(renderer) => renderer,
            Err(e) => {
                log::error!("failed to create renderer: {e}");
                self.host
                    .render_failed(format!("Failed to create renderer: {e}"));
                return None;
            }
        };
        if let Err(e) = renderer.attach(
            raw_window_handle,
            raw_display_handle,
            SurfaceSize::new(size.width, size.height),
        ) {
            log::error!("failed to attach renderer: {e}");
            self.host
                .render_failed(format!("Failed to attach renderer: {e}"));
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
                // Startup runs before the main window exists: standalone dialog only.
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

        if let Some(thread) =
            Self::spawn_tick_thread(event_loop.create_proxy(), self.host.live_tick_armed())
        {
            self.host.register_tick_thread(thread);
        }

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
            } => {
                if let Some(kind) = self.host.aux_kind_of(window_id) {
                    let opener = TaoWindowOpener::new(event_loop);
                    self.on_aux_event(kind, event, &opener);
                }
            }
            Event::RedrawRequested(window_id) if self.host.is_window(window_id) => self.on_update(),
            Event::RedrawRequested(window_id) => {
                // Debugger redraws also drain; the rest only render.
                if let Some(kind) = self.host.aux_kind_of(window_id) {
                    if kind == crate::aux::focus::AuxKind::Debug {
                        let opener = TaoWindowOpener::new(event_loop);
                        self.on_debug_redrawn(&opener);
                    } else {
                        self.on_aux_redrawn(kind);
                    }
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
        if !self.host.render_ready() {
            return;
        }
        if self.renderer.is_none() {
            self.recreate_renderer();
        }
        let Some(window_size) = self.host.window_surface_size() else {
            return;
        };
        if window_size.width == 0 || window_size.height == 0 {
            return;
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };

        // Keep the output sized to the current target: target change needs a full recreate.
        let current_xid = self.host.surface_xid();
        if self.surface_xid != current_xid {
            self.recreate_renderer();
            return;
        }
        if renderer.size() != window_size {
            renderer.resize(window_size);
        }

        // No notification drain here: `update_control_flow` drains every pass.
        // One capture for the whole redraw: strips and layer sync share the frame.
        let ctx = self.host.frame_ctx();
        self.host
            .render_osd_frame(renderer.as_mut(), window_size, &ctx);

        self.host.swap_frame_buffer();
        if let Some(fb) = self.host.frame_buffer() {
            let frame_counter = ctx.tick().frame_counter;
            let result = renderer.render(fb);
            self.host.on_render_result(result, frame_counter);
            if matches!(result, nerust_render_traits::renderer::RenderResult::Error) {
                self.renderer = None;
            }
        }
    }

    fn recreate_renderer(&mut self) {
        self.renderer = None;
        if !self.host.render_ready() {
            return;
        }
        // Owned display handle out; the attach below only borrows the host.
        let display = self
            .host
            .with_main_window(|window| window.display_handle().ok().map(|handle| handle.as_raw()));
        if let Some(Some(raw_display)) = display {
            self.build_renderer_and_surface(raw_display);
        } else if display.is_some() {
            log::error!("failed to get display handle");
            self.host
                .render_failed("Failed to get display handle".into());
        }
    }
}

impl Drop for WindowRuntime {
    fn drop(&mut self) {
        self.renderer = None;
    }
}
