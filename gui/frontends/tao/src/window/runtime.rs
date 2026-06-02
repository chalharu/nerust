mod host;
mod renderer;

use self::host::{HostAction, HostState};
use self::renderer::WgpuRenderer;
use crate::app_menu::{UserEvent, imp::AppMenu};
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::load::LoadRequest;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use tao::platform::macos::EventLoopExtMacOS;
use tao::{
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder},
};

pub(crate) struct WindowRuntime {
    event_loop: Option<EventLoop<UserEvent>>,
    host: HostState,
    renderer: Option<WgpuRenderer>,
}

impl WindowRuntime {
    pub(crate) fn new(default_load_request: LoadRequest) -> Self {
        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        #[cfg(target_os = "macos")]
        let event_loop = {
            let mut event_loop = event_loop;
            // Explicitly let macOS activate the app even when another app is currently active.
            event_loop.set_activate_ignoring_other_apps(true);
            event_loop
        };
        let proxy = event_loop.create_proxy();

        Self {
            event_loop: Some(event_loop),
            host: HostState::new(AppMenu::new(proxy.clone()), proxy, default_load_request),
            renderer: None,
        }
    }

    pub(crate) fn load(&mut self, data: Vec<u8>) {
        if self.host.load(data) {
            self.recreate_renderer();
            self.host.request_redraw();
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
            self.host.request_redraw();
        }
    }

    pub(crate) fn load_path(&mut self, path: &Path) -> bool {
        let loaded = self.host.load_path(path);
        if loaded {
            self.recreate_renderer();
            self.host.request_redraw();
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
                self.host.request_redraw();
                *control_flow = ControlFlow::Wait;
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self.host.is_window(window_id) => match event {
                WindowEvent::CloseRequested if self.host.prepare_close() => {
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::Focused(false) => self.host.clear_keys(),
                WindowEvent::Resized(_) => {
                    self.host.sync_fullscreen_default_from_window();
                    if let Some(window_size) = self.host.window_surface_size()
                        && let Some(renderer) = self.renderer.as_mut()
                    {
                        renderer.reconfigure(window_size);
                    }
                    self.host.request_redraw();
                }
                WindowEvent::KeyboardInput { event, .. } => self.host.on_keyboard_input(event),
                _ => (),
            },
            Event::RedrawRequested(window_id) if self.host.is_window(window_id) => self.on_update(),
            Event::MainEventsCleared => self.host.update_control_flow(control_flow),
            Event::UserEvent(command) => match command {
                UserEvent::Menu(command) => match self.host.on_menu_command(command) {
                    HostAction::None => (),
                    HostAction::RomLoaded => {
                        self.recreate_renderer();
                        self.host.request_redraw();
                    }
                    HostAction::Exit => *control_flow = ControlFlow::Exit,
                },
                UserEvent::ApplySettings { snapshot, reply } => {
                    let _ = reply.send(self.apply_settings(snapshot));
                }
                UserEvent::SettingsClosed => self.host.on_settings_closed(),
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

        let result = renderer.render(self.host.session(), window_size);
        self.host.on_render_result(result);
    }

    fn recreate_renderer(&mut self) {
        self.renderer = None;
        self.renderer = self
            .host
            .window()
            .cloned()
            .map(|window| WgpuRenderer::new(window, self.host.session()));
    }

    fn apply_settings(&mut self, settings: SettingsSnapshot) -> Result<(), String> {
        let plan = self.host.apply_settings(settings)?;
        if plan.renderer_rebuild_required {
            self.recreate_renderer();
        }
        Ok(())
    }
}

impl Drop for WindowRuntime {
    fn drop(&mut self) {
        self.renderer = None;
    }
}
