mod host;

use std::path::{Path, PathBuf};

#[cfg(feature = "opengl")]
use nerust_backend_opengl::GlFactory as Factory;
#[cfg(feature = "wgpu")]
use nerust_backend_wgpu::WgpuFactory as Factory;
use nerust_gui_shell::load::LoadRequest;
use nerust_screen_video::{GpuFactory, GpuRenderer, RendererConfig, SurfaceSize};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
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
}

impl WindowRuntime {
    pub(crate) fn new() -> Self {
        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        #[cfg(target_os = "macos")]
        let event_loop = {
            let mut event_loop = event_loop;
            event_loop.set_activate_ignoring_other_apps(true);
            event_loop
        };
        let proxy = event_loop.create_proxy();

        Self {
            event_loop: Some(event_loop),
            host: HostState::new(AppMenu::new(proxy)),
            renderer: None,
        }
    }

    fn build_renderer_and_surface(&mut self, window: &tao::window::Window) -> Option<()> {
        let size = window.inner_size();
        let session = self.host.session();
        let vsync = session.settings_snapshot().local.video.presentation.vsync;
        let raw_window_handle = window
            .window_handle()
            .expect("failed to get window handle")
            .as_raw();
        let raw_display_handle = window
            .display_handle()
            .expect("failed to get display handle")
            .as_raw();
        let config = RendererConfig {
            initial_size: SurfaceSize::new(size.width, size.height),
            render_profile: session.render_profile().clone(),
            vsync,
        };
        let mut renderer = Factory
            .create_renderer(&config, raw_display_handle)
            .expect("failed to create renderer");
        renderer
            .attach(
                raw_window_handle,
                raw_display_handle,
                SurfaceSize::new(size.width, size.height),
            )
            .expect("failed to attach");
        self.renderer = Some(renderer);
        Some(())
    }

    pub(crate) fn load(&mut self, data: Vec<u8>) {
        if self.host.load(data) {
            self.recreate_renderer();
        }
    }

    pub(crate) fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        request: LoadRequest,
    ) {
        if self.host.load_with_options(rom_path, data, request) {
            self.recreate_renderer();
        }
    }

    pub(crate) fn load_path(&mut self, path: &Path) -> bool {
        let loaded = self.host.load_path(path);
        if loaded {
            self.recreate_renderer();
        }
        loaded
    }

    pub(crate) fn run(mut self) {
        self.host.resume_session();
        let event_loop = self.event_loop.take().unwrap();

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
                }
                WindowEvent::Focused(false) => {
                    self.host.active = false;
                    if self.host.session().can_pause() {
                        self.host.pause_session();
                        self.host.set_auto_paused();
                    }
                    self.host.clear_keys();
                }
                WindowEvent::Resized(_) => {
                    self.host.sync_fullscreen_default_from_window();
                    self.host.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. } => self.host.on_keyboard_input(event),
                _ => (),
            },
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.is_settings_window(window_id) => {
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
                    if plan.is_some_and(|p| p.renderer_rebuild_required) {
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
            Event::MainEventsCleared => self.host.update_control_flow(control_flow),
            Event::UserEvent(command) => match command {
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

        // Keep the output sized to the current window.
        if renderer.size() != window_size {
            renderer.resize(window_size);
        }

        self.host.session_mut().swap_frame_buffer();
        let result = renderer.render(self.host.session_mut().frame_buffer());
        self.host.on_render_result(result);
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
