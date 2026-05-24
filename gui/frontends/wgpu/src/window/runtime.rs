// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::app_menu::{MenuCommand, UserEvent, imp::AppMenu};
use crate::surface::SurfaceTarget;
use nerust_backend_wgpu::{RenderResult, WgpuBackend};
use nerust_contract_settings::KeyboardKey;
use nerust_gui_runtime::settings::DesktopSettingsManager;
use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_gui_shell::load::NesLoadOptions;
use nerust_gui_shell::session::NesSession;
use nerust_gui_shell::settings::{
    controller_event_for_key, current_or_default, shortcut_action_for_key, shortcut_command_for_key,
};
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
    window::{Fullscreen, Window as TaoWindow, WindowBuilder},
};

fn keycode_settings_key(code: KeyCode) -> Option<KeyboardKey> {
    Some(match code {
        KeyCode::KeyA => KeyboardKey::KeyA,
        KeyCode::KeyB => KeyboardKey::KeyB,
        KeyCode::KeyC => KeyboardKey::KeyC,
        KeyCode::KeyD => KeyboardKey::KeyD,
        KeyCode::KeyE => KeyboardKey::KeyE,
        KeyCode::KeyF => KeyboardKey::KeyF,
        KeyCode::KeyG => KeyboardKey::KeyG,
        KeyCode::KeyH => KeyboardKey::KeyH,
        KeyCode::KeyI => KeyboardKey::KeyI,
        KeyCode::KeyJ => KeyboardKey::KeyJ,
        KeyCode::KeyK => KeyboardKey::KeyK,
        KeyCode::KeyL => KeyboardKey::KeyL,
        KeyCode::KeyM => KeyboardKey::KeyM,
        KeyCode::KeyN => KeyboardKey::KeyN,
        KeyCode::KeyO => KeyboardKey::KeyO,
        KeyCode::KeyP => KeyboardKey::KeyP,
        KeyCode::KeyQ => KeyboardKey::KeyQ,
        KeyCode::KeyR => KeyboardKey::KeyR,
        KeyCode::KeyS => KeyboardKey::KeyS,
        KeyCode::KeyT => KeyboardKey::KeyT,
        KeyCode::KeyU => KeyboardKey::KeyU,
        KeyCode::KeyV => KeyboardKey::KeyV,
        KeyCode::KeyW => KeyboardKey::KeyW,
        KeyCode::KeyX => KeyboardKey::KeyX,
        KeyCode::KeyY => KeyboardKey::KeyY,
        KeyCode::KeyZ => KeyboardKey::KeyZ,
        KeyCode::ArrowUp => KeyboardKey::ArrowUp,
        KeyCode::ArrowDown => KeyboardKey::ArrowDown,
        KeyCode::ArrowLeft => KeyboardKey::ArrowLeft,
        KeyCode::ArrowRight => KeyboardKey::ArrowRight,
        KeyCode::Enter => KeyboardKey::Enter,
        KeyCode::Escape => KeyboardKey::Escape,
        KeyCode::Space => KeyboardKey::Space,
        KeyCode::Tab => KeyboardKey::Tab,
        KeyCode::F1 => KeyboardKey::F1,
        KeyCode::F2 => KeyboardKey::F2,
        KeyCode::F3 => KeyboardKey::F3,
        KeyCode::F4 => KeyboardKey::F4,
        KeyCode::F5 => KeyboardKey::F5,
        KeyCode::F6 => KeyboardKey::F6,
        KeyCode::F7 => KeyboardKey::F7,
        KeyCode::F8 => KeyboardKey::F8,
        KeyCode::F9 => KeyboardKey::F9,
        KeyCode::F10 => KeyboardKey::F10,
        KeyCode::F11 => KeyboardKey::F11,
        KeyCode::F12 => KeyboardKey::F12,
        _ => return None,
    })
}

fn element_state_to_pressed(state: ElementState) -> Option<bool> {
    Some(match state {
        ElementState::Pressed => true,
        ElementState::Released => false,
        _ => return None,
    })
}

fn create_window_builder(
    size: WindowSize,
    title: String,
    settings: &DesktopSettingsManager,
) -> WindowBuilder {
    let (width, height) = settings
        .effective_window_size(size.width as u32, size.height as u32)
        .unwrap_or((size.width as u32, size.height as u32));
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(TaoLogicalSize::new(f64::from(width), f64::from(height)))
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
    settings: DesktopSettingsManager,
    fullscreened: bool,
}

impl WindowRuntime {
    pub(crate) fn new(settings: DesktopSettingsManager) -> Self {
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
        let fullscreened = current_or_default(&settings).video.fullscreen;

        Self {
            event_loop: Some(event_loop),
            window: None,
            backend: None,
            session: NesSession::new(settings.clone()),
            app_menu,
            shell: NativeShellState::new(),
            settings,
            fullscreened,
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
        if self
            .session
            .load_with_options(rom_path.clone(), data, options)
        {
            if let Some(path) = rom_path.as_deref() {
                let _ = self.settings.record_opened_rom(path);
            }
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
            create_window_builder(
                self.session.window_size(),
                self.current_window_title(),
                &self.settings,
            )
            .build(event_loop)
            .unwrap(),
        );
        if current_or_default(&self.settings).video.fullscreen {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
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
        if !self.fullscreened
            && let Some(window) = self.window.as_ref()
        {
            let logical_size = window.inner_size().to_logical::<f64>(window.scale_factor());
            let width = logical_size.width.round().max(1.0) as u32;
            let height = logical_size.height.round().max(1.0) as u32;
            let _ = self.settings.remember_window_size(width, height);
        }
    }

    fn toggle_fullscreen(&mut self) {
        self.fullscreened = !self.fullscreened;
        if let Some(window) = self.window.as_ref() {
            window.set_fullscreen(self.fullscreened.then_some(Fullscreen::Borderless(None)));
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
        let settings = current_or_default(&self.settings);
        if let Some(key) = keycode_settings_key(input.physical_key) {
            if input.state == ElementState::Released
                && let Some(controller_event) = controller_event_for_key(&settings, key, false)
            {
                self.session.handle_controller_input(controller_event);
            }
            if input.state == ElementState::Released
                && !input.repeat
                && let Some(action) = shortcut_action_for_key(&settings, key)
                && matches!(
                    action,
                    nerust_contract_settings::ShortcutAction::ToggleFullscreen
                )
            {
                self.toggle_fullscreen();
                return;
            }
            if input.state == ElementState::Released
                && !input.repeat
                && let Some(command) = shortcut_command_for_key(&settings, key)
            {
                match command {
                    SessionCommand::TogglePause
                    | SessionCommand::Reset
                    | SessionCommand::SaveActiveSlotOrNew
                    | SessionCommand::LoadActiveSlot
                    | SessionCommand::SelectNextSlot
                    | SessionCommand::SelectPreviousSlot => self.apply_session_command(command),
                    _ => {}
                }
                return;
            }
            if let Some(pressed) = element_state_to_pressed(input.state).filter(|pressed| *pressed)
                && let Some(event) = controller_event_for_key(&settings, key, pressed)
            {
                self.session.handle_controller_input(event);
            }
        }
    }

    fn clear_keys(&mut self) {
        let settings = current_or_default(&self.settings);
        if settings.host.clear_input_on_focus_loss {
            self.session.clear_controller_input();
        }
        if settings.host.pause_on_focus_loss {
            self.apply_session_command(SessionCommand::Pause);
        }
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
    use super::keycode_settings_key;
    use nerust_contract_settings::KeyboardKey;
    use tao::keyboard::KeyCode;

    #[test]
    fn keycode_mapping_matches_settings_keys() {
        assert_eq!(keycode_settings_key(KeyCode::KeyZ), Some(KeyboardKey::KeyZ));
        assert_eq!(keycode_settings_key(KeyCode::KeyX), Some(KeyboardKey::KeyX));
        assert_eq!(
            keycode_settings_key(KeyCode::ArrowUp),
            Some(KeyboardKey::ArrowUp)
        );
        assert_eq!(
            keycode_settings_key(KeyCode::ArrowRight),
            Some(KeyboardKey::ArrowRight)
        );
        assert_eq!(
            keycode_settings_key(KeyCode::Enter),
            Some(KeyboardKey::Enter)
        );
    }
}
