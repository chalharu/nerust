// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::app_menu::{MenuCommand, UserEvent, imp::AppMenu};
use crate::surface::SurfaceTarget;
use nerust_backend_wgpu::{RenderResult, WgpuBackend};
use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_gui_shell::load::NesLoadOptions;
use nerust_gui_shell::session::NesSession;
use nerust_input_nes::{
    NES_ATTACHMENT_PLAYER_ONE, NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_DOWN, NES_CONTROL_LEFT,
    NES_CONTROL_RIGHT, NES_CONTROL_SELECT, NES_CONTROL_START, NES_CONTROL_UP,
};
use nerust_input_schema::{DigitalControlId, DigitalInputEvent, DigitalInputState};
use nerust_screen_wgpu::surface::SurfaceSize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
#[cfg(target_os = "macos")]
use tao::platform::macos::EventLoopExtMacOS;
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::{ElementState, Event, KeyEvent, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopWindowTarget},
    keyboard::KeyCode,
    window::{Window as TaoWindow, WindowBuilder},
};

fn keycode_controller_input(code: KeyCode) -> Option<DigitalControlId> {
    Some(match code {
        KeyCode::KeyZ => NES_CONTROL_A,
        KeyCode::KeyX => NES_CONTROL_B,
        KeyCode::KeyC => NES_CONTROL_SELECT,
        KeyCode::KeyV => NES_CONTROL_START,
        KeyCode::ArrowUp => NES_CONTROL_UP,
        KeyCode::ArrowDown => NES_CONTROL_DOWN,
        KeyCode::ArrowLeft => NES_CONTROL_LEFT,
        KeyCode::ArrowRight => NES_CONTROL_RIGHT,
        _ => return None,
    })
}

fn element_state_to_input_state(state: ElementState) -> Option<DigitalInputState> {
    Some(match state {
        ElementState::Pressed => DigitalInputState::Pressed,
        ElementState::Released => DigitalInputState::Released,
        _ => return None,
    })
}

fn create_window_builder(size: WindowSize, title: String) -> WindowBuilder {
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
    backend: Option<WgpuBackend<SurfaceTarget>>,
    session: NesSession,
    app_menu: AppMenu,
    shell: NativeShellState,
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
            backend: None,
            session: NesSession::new(),
            app_menu,
            shell: NativeShellState::new(),
        }
    }

    pub(crate) fn load(&mut self, data: Vec<u8>) {
        self.load_with_options(None, data, NesLoadOptions::default());
    }

    pub(crate) fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        options: NesLoadOptions,
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
                        self.shell.needs_redraw = true;
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
                if self.shell.wants_redraw(metrics.frame_counter) {
                    window.request_redraw();
                }

                if self.shell.wants_poll(metrics.loaded, metrics.paused) {
                    *control_flow =
                        ControlFlow::WaitUntil(now + NativeShellState::FRAME_POLL_INTERVAL);
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
            create_window_builder(self.session.window_size(), self.current_window_title())
                .build(event_loop)
                .unwrap(),
        );
        let surface_target = SurfaceTarget::new(window.clone(), self.session.window_size());
        self.app_menu.init_for_window(&window);
        self.sync_menu_state();
        let initial_size = window_surface_size(window.inner_size());
        let video = self.session.video();
        let backend = WgpuBackend::new(
            surface_target,
            initial_size,
            video.presentation(),
            video
                .console_video_assets()
                .expect("NES session always has video assets"),
        )
        .unwrap();
        self.window = Some(window);
        self.backend = Some(backend);
        self.shell.needs_redraw = true;
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
        if self.shell.should_refresh_title(now) {
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
            self.shell.needs_redraw = true;
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
        if let Some(backend) = self.backend.as_mut() {
            backend.reconfigure(window_size);
        }
    }

    fn on_update(&mut self) {
        let Some(window_size) = self
            .window
            .as_ref()
            .map(|window| window_surface_size(window.inner_size()))
        else {
            return;
        };
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let result = self
            .session
            .with_frame_buffer(|frame_buffer| backend.render(frame_buffer, window_size));
        match result {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.metrics().frame_counter);
                self.maybe_refresh_window_title(Instant::now());
            }
            RenderResult::Skipped => {
                self.shell.needs_redraw = true;
            }
            RenderResult::Error => {
                // Error already logged by the backend. Keep one redraw pending so
                // paused/idle sessions can recover when the surface becomes ready.
                self.shell.needs_redraw = true;
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
            self.session.handle_controller_input(DigitalInputEvent::new(
                NES_ATTACHMENT_PLAYER_ONE,
                controller_input,
                input_state,
            ));
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
        self.backend = None;
        self.window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::keycode_controller_input;
    use nerust_input_nes::{NES_CONTROL_A, NES_CONTROL_B, NES_CONTROL_RIGHT, NES_CONTROL_UP};
    use tao::keyboard::KeyCode;

    #[test]
    fn keycode_mapping_matches_controller_layout() {
        assert_eq!(keycode_controller_input(KeyCode::KeyZ), Some(NES_CONTROL_A));
        assert_eq!(keycode_controller_input(KeyCode::KeyX), Some(NES_CONTROL_B));
        assert_eq!(
            keycode_controller_input(KeyCode::ArrowUp),
            Some(NES_CONTROL_UP)
        );
        assert_eq!(
            keycode_controller_input(KeyCode::ArrowRight),
            Some(NES_CONTROL_RIGHT)
        );
        assert_eq!(keycode_controller_input(KeyCode::Enter), None);
    }
}
