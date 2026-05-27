mod renderer;
mod surface;

use self::renderer::WgpuRenderer;
use nerust_backend_wgpu::RenderResult;
use nerust_gui_runtime::settings::HostBackendIdentity;
use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_session::commands::SessionCommand;
use nerust_gui_shell::session::SessionHandle;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::android::activity::AndroidApp;
use winit::platform::android::EventLoopBuilderExtAndroid;
use winit::window::{Window, WindowId};

pub(crate) fn run(app: AndroidApp) -> Result<(), String> {
    let mut builder = EventLoop::<()>::with_user_event();
    builder.with_android_app(app);
    builder.handle_volume_keys();
    let event_loop = builder
        .build()
        .map_err(|error| format!("failed to build Android event loop: {error}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let mut state = AndroidFrontend::default();
    event_loop
        .run_app(&mut state)
        .map_err(|error| format!("Android event loop failed: {error}"))
}

struct AndroidFrontend {
    session: SessionHandle,
    shell: NativeShellState,
    window: Option<Arc<Window>>,
    window_id: Option<WindowId>,
    renderer: Option<WgpuRenderer>,
}

impl Default for AndroidFrontend {
    fn default() -> Self {
        Self {
            session: SessionHandle::new_for_host(HostBackendIdentity::android_wgpu()),
            shell: NativeShellState::new(),
            window: None,
            window_id: None,
            renderer: None,
        }
    }
}

impl AndroidFrontend {
    fn ensure_window(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if self.window.is_some() {
            return Ok(());
        }

        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(self.session.window_title())
                        .with_resizable(false)
                        .with_inner_size(LogicalSize::new(360.0, 640.0)),
                )
                .map_err(|error| format!("failed to create Android window: {error}"))?,
        );
        self.window_id = Some(window.id());
        self.renderer = Some(WgpuRenderer::new(window.clone(), &self.session));
        self.window = Some(window);
        Ok(())
    }

    fn request_redraw(&mut self) {
        self.shell.needs_redraw = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn render(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let size = window.inner_size();
        match renderer.render(
            &self.session,
            nerust_screen_wgpu::surface::SurfaceSize::new(size.width, size.height),
        ) {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.metrics().frame_counter);
            }
            RenderResult::Skipped | RenderResult::Error => {
                self.shell.needs_redraw = true;
            }
        }
    }

    fn maybe_refresh_title(&mut self, now: Instant) {
        if self.shell.should_refresh_title(now) && let Some(window) = self.window.as_ref() {
            window.set_title(&self.session.window_title());
        }
    }
}

impl ApplicationHandler for AndroidFrontend {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.ensure_window(event_loop) {
            log::error!("{error}");
            event_loop.exit();
            return;
        }
        let _ = self.session.run_command(SessionCommand::Resume);
        self.request_redraw();
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.session.clear_input();
        self.renderer = None;
        self.window = None;
        self.window_id = None;
        self.shell.needs_redraw = true;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window_id != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.session.flush_before_exit();
                event_loop.exit();
            }
            WindowEvent::Focused(false) => {
                let _ = self.session.clear_input();
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.reconfigure(nerust_screen_wgpu::surface::SurfaceSize::new(
                        size.width,
                        size.height,
                    ));
                }
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        self.maybe_refresh_title(Instant::now());
        if let Some(window) = self.window.as_ref()
            && self
                .shell
                .wants_redraw(self.session.metrics().frame_counter)
        {
            window.request_redraw();
        }
    }
}
