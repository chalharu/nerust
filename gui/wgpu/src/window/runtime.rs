// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::app_menu::{AppMenu, MenuCommand, UserEvent};
use crate::surface::SurfaceTarget;
use nerust_core::CoreOptions;
use nerust_gui_runtime::{
    ControllerInput, ControllerPort, GuiSession, InputState, SessionCommand, SessionCommandOutcome,
};
use nerust_screen_traits::PhysicalSize;
use nerust_screen_wgpu::{RenderOutcome, Renderer};
use nerust_wgpuwrap::{RenderSurface, SurfaceSize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(target_os = "macos")]
use tao::platform::macos::EventLoopExtMacOS;
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::{ElementState, Event, KeyEvent, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopWindowTarget},
    keyboard::KeyCode,
    window::{Window as TaoWindow, WindowBuilder},
};

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);

fn keycode_controller_input(code: KeyCode) -> Option<ControllerInput> {
    Some(match code {
        KeyCode::KeyZ => ControllerInput::A,
        KeyCode::KeyX => ControllerInput::B,
        KeyCode::KeyC => ControllerInput::Select,
        KeyCode::KeyV => ControllerInput::Start,
        KeyCode::ArrowUp => ControllerInput::Up,
        KeyCode::ArrowDown => ControllerInput::Down,
        KeyCode::ArrowLeft => ControllerInput::Left,
        KeyCode::ArrowRight => ControllerInput::Right,
        _ => return None,
    })
}

fn element_state_to_input_state(state: ElementState) -> Option<InputState> {
    Some(match state {
        ElementState::Pressed => InputState::Pressed,
        ElementState::Released => InputState::Released,
        _ => return None,
    })
}

fn create_window_builder(size: PhysicalSize, title: String) -> WindowBuilder {
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(TaoLogicalSize::new(
            f64::from(size.width),
            f64::from(size.height),
        ))
}

fn window_surface_size(size: TaoPhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize::new(size.width, size.height)
}

pub(crate) struct WindowRuntime {
    event_loop: Option<EventLoop<UserEvent>>,
    window: Option<Arc<TaoWindow>>,
    render_surface: Option<RenderSurface<SurfaceTarget>>,
    renderer: Option<Renderer>,
    last_render_error: Option<String>,
    session: GuiSession,
    app_menu: AppMenu,
    last_title_update: Instant,
    last_presented_frame_counter: u64,
    needs_redraw: bool,
}

impl WindowRuntime {
    pub(crate) fn new() -> Self {
        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        #[cfg(target_os = "macos")]
        let event_loop = {
            let mut event_loop = event_loop;
            // Explicitly let macOS activate the app even when another app is currently active.
            event_loop.set_activate_ignoring_other_apps(true);
            event_loop
        };
        let proxy = event_loop.create_proxy();
        let app_menu = AppMenu::new(proxy);

        Self {
            event_loop: Some(event_loop),
            window: None,
            render_surface: None,
            renderer: None,
            last_render_error: None,
            session: GuiSession::default(),
            app_menu,
            last_title_update: Instant::now(),
            last_presented_frame_counter: 0,
            needs_redraw: true,
        }
    }

    pub(crate) fn load(&mut self, data: Vec<u8>) {
        self.load_with_options(None, data, CoreOptions::default());
    }

    pub(crate) fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        options: CoreOptions,
    ) {
        if self.session.load_with_options(rom_path, data, options) {
            self.sync_menu_state();
        }
    }

    pub(crate) fn run(mut self) {
        self.session.resume();
        let event_loop = self.event_loop.take().unwrap();

        event_loop.run(move |event, event_loop, control_flow| match event {
            Event::NewEvents(StartCause::Init) => {
                self.ensure_window(event_loop);
                *control_flow = ControlFlow::Wait;
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self
                .window
                .as_ref()
                .is_some_and(|window| window_id == window.id()) =>
            {
                match event {
                    WindowEvent::CloseRequested if self.prepare_close() => {
                        *control_flow = ControlFlow::Exit;
                    }
                    WindowEvent::Focused(false) => self.clear_keys(),
                    WindowEvent::Resized(_) => {
                        self.reconfigure_surface();
                        self.needs_redraw = true;
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
                        }
                    }
                    WindowEvent::KeyboardInput { event, .. } => self.on_keyboard_input(event),
                    _ => (),
                }
            }
            Event::RedrawRequested(window_id)
                if self
                    .window
                    .as_ref()
                    .is_some_and(|window| window_id == window.id()) =>
            {
                self.on_update()
            }
            Event::MainEventsCleared => {
                let now = Instant::now();
                self.maybe_refresh_window_title(now);

                let Some(window) = self.window.as_ref() else {
                    *control_flow = ControlFlow::Wait;
                    return;
                };

                let metrics = self.session.metrics();
                if self.needs_redraw || metrics.frame_counter != self.last_presented_frame_counter {
                    window.request_redraw();
                }

                if self.needs_redraw || (metrics.loaded && !metrics.paused) {
                    *control_flow = ControlFlow::WaitUntil(now + FRAME_POLL_INTERVAL);
                } else {
                    *control_flow = ControlFlow::Wait;
                }
            }
            Event::UserEvent(UserEvent::Menu(command)) => {
                self.on_menu_command(control_flow, command);
            }
            Event::LoopDestroyed => {
                self.app_menu.clear_event_handler();
            }
            _ => (),
        });
    }

    fn ensure_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.window.is_some() {
            return;
        }

        let window = Arc::new(
            create_window_builder(self.session.physical_size(), self.current_window_title())
                .build(event_loop)
                .unwrap(),
        );
        let surface_target = SurfaceTarget::new(window.clone(), self.session.physical_size());
        self.app_menu.init_for_window(&window);
        self.sync_menu_state();
        let render_surface = RenderSurface::new(surface_target).unwrap();
        let renderer = pollster::block_on(Renderer::new(
            &render_surface,
            render_surface.surface_size(window_surface_size(window.inner_size())),
            self.session.presentation(),
        ))
        .unwrap();
        self.window = Some(window);
        self.render_surface = Some(render_surface);
        self.renderer = Some(renderer);
        self.needs_redraw = true;
        self.refresh_window_title();
    }

    fn current_window_title(&self) -> String {
        self.session.window_title()
    }

    fn refresh_window_title(&mut self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&self.current_window_title());
        }
    }

    fn maybe_refresh_window_title(&mut self, now: Instant) {
        if now.duration_since(self.last_title_update) >= TITLE_UPDATE_INTERVAL {
            self.last_title_update = now;
            self.refresh_window_title();
        }
    }

    fn sync_menu_state(&mut self) {
        self.app_menu.update(
            self.session.loaded(),
            self.session.paused(),
            self.session.slots(),
            self.session.active_slot_id(),
        );
    }

    fn apply_command_outcome(&mut self, outcome: SessionCommandOutcome) {
        if outcome.needs_redraw {
            self.needs_redraw = true;
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
        self.sync_menu_state();
        self.refresh_window_title();
    }

    fn apply_session_command(&mut self, command: SessionCommand) {
        let outcome = self.session.run_command(command);
        self.apply_command_outcome(outcome);
    }

    fn on_menu_command(&mut self, control_flow: &mut ControlFlow, command: MenuCommand) {
        match command {
            MenuCommand::Session(command) => self.apply_session_command(command),
            MenuCommand::Quit => {
                if self.prepare_close() {
                    *control_flow = ControlFlow::Exit;
                }
            }
        }
    }

    fn reconfigure_surface(&mut self) {
        let Some(window_size) = self
            .window
            .as_ref()
            .map(|window| window_surface_size(window.inner_size()))
        else {
            return;
        };
        let (Some(renderer), Some(render_surface)) =
            (self.renderer.as_mut(), self.render_surface.as_mut())
        else {
            return;
        };
        renderer.reconfigure_surface(render_surface, render_surface.surface_size(window_size));
    }

    fn on_update(&mut self) {
        let Some(window_size) = self
            .window
            .as_ref()
            .map(|window| window_surface_size(window.inner_size()))
        else {
            return;
        };
        let (Some(renderer), Some(render_surface)) =
            (self.renderer.as_mut(), self.render_surface.as_mut())
        else {
            return;
        };
        let surface_size = render_surface.surface_size(window_size);
        let render_result = self.session.with_frame_buffer(|frame_buffer| {
            renderer.render(render_surface, surface_size, frame_buffer)
        });

        match render_result {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                self.last_presented_frame_counter = self.session.metrics().frame_counter;
                self.needs_redraw = false;
                self.maybe_refresh_window_title(Instant::now());
            }
            Ok(RenderOutcome::Skipped) => {
                self.needs_redraw = true;
            }
            Ok(RenderOutcome::RecreateSurface) => {
                self.needs_redraw = true;
                match render_surface.recreate_surface() {
                    Ok(()) => {
                        self.last_render_error = None;
                        renderer.reconfigure_surface(
                            render_surface,
                            render_surface.surface_size(window_size),
                        );
                    }
                    Err(err) => {
                        let should_log = self.last_render_error.as_deref() != Some(err.as_str());
                        self.last_render_error = Some(err.clone());
                        if should_log {
                            log::error!("{err}");
                        }
                    }
                }
            }
            Err(err) => {
                let should_log = self.last_render_error.as_deref() != Some(err.as_str());
                self.last_render_error = Some(err.clone());
                self.needs_redraw = true;
                if should_log {
                    log::error!("{err}");
                }
            }
        }
    }

    fn on_keyboard_input(&mut self, input: KeyEvent) {
        let code = match input.physical_key {
            KeyCode::Space if input.state == ElementState::Pressed && !input.repeat => {
                self.apply_session_command(SessionCommand::TogglePause);
                None
            }
            KeyCode::Escape if input.state == ElementState::Released => {
                self.apply_session_command(SessionCommand::Reset);
                None
            }
            KeyCode::F5 if input.state == ElementState::Released && !input.repeat => {
                self.apply_session_command(SessionCommand::SaveActiveSlotOrNew);
                None
            }
            KeyCode::F8 if input.state == ElementState::Released && !input.repeat => {
                self.apply_session_command(SessionCommand::LoadActiveSlot);
                None
            }
            code => keycode_controller_input(code),
        };

        if let Some(input_state) = element_state_to_input_state(input.state)
            && let Some(controller_input) = code
        {
            self.session.handle_controller_input(
                ControllerPort::One,
                controller_input,
                input_state,
            );
        }
    }

    fn clear_keys(&mut self) {
        self.session.clear_controller_input();
    }

    fn prepare_close(&mut self) -> bool {
        self.session.flush_before_exit();
        true
    }
}

impl Drop for WindowRuntime {
    fn drop(&mut self) {
        self.renderer = None;
        self.render_surface = None;
        self.window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::keycode_controller_input;
    use nerust_gui_runtime::ControllerInput;
    use tao::keyboard::KeyCode;

    #[test]
    fn keycode_mapping_matches_controller_layout() {
        assert_eq!(
            keycode_controller_input(KeyCode::KeyZ),
            Some(ControllerInput::A)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::KeyX),
            Some(ControllerInput::B)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::ArrowUp),
            Some(ControllerInput::Up)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::ArrowRight),
            Some(ControllerInput::Right)
        );
        assert_eq!(keycode_controller_input(KeyCode::Enter), None);
    }
}
