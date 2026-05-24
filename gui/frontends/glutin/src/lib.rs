// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use glutin::config::{Config, ConfigTemplateBuilder};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentContext, PossiblyCurrentContext,
    Version,
};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::prelude::*;
use glutin::surface::{Surface, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use nerust_backend_opengl::GlBackend;
use nerust_contract_settings::{input::KeyboardKey, shortcut::ShortcutAction};
use nerust_gui_runtime::settings::DesktopSettingsManager;
use nerust_gui_runtime::shell::NativeShellState;
use nerust_gui_session::commands::{SessionCommand, SessionCommandOutcome};
use nerust_gui_session::core::WindowSize;
use nerust_gui_shell::session::NesSession;
use nerust_gui_shell::settings::{
    bindings::events::{
        controller::controller_event_for_key,
        shortcut::{shortcut_action_for_key, shortcut_command_for_key},
    },
    defaults::manager::{current_or_default, load_settings_manager},
};
use raw_window_handle::HasWindowHandle;
use std::f64;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize as WinitLogicalSize, PhysicalSize as WinitPhysicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window as WinitWindow, WindowAttributes};

fn create_window_attributes(
    size: WindowSize,
    settings: &DesktopSettingsManager,
) -> WindowAttributes {
    let (width, height) = settings
        .effective_window_size(size.width as u32, size.height as u32)
        .unwrap_or((size.width as u32, size.height as u32));
    WinitWindow::default_attributes()
        .with_inner_size(WinitLogicalSize::new(f64::from(width), f64::from(height)))
        .with_title("Nes")
}

fn create_gl_context(window: &WinitWindow, gl_config: &Config) -> NotCurrentContext {
    let raw_window_handle = window.window_handle().ok().map(|handle| handle.as_raw());
    let gl_display = gl_config.display();
    let context_attributes = ContextAttributesBuilder::new()
        .with_profile(GlProfile::Core)
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(raw_window_handle);
    let fallback_context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
        .build(raw_window_handle);

    unsafe {
        gl_display
            .create_context(gl_config, &context_attributes)
            .unwrap_or_else(|_| {
                gl_display
                    .create_context(gl_config, &fallback_context_attributes)
                    .expect("failed to create GL context")
            })
    }
}

fn create_window(
    event_loop: &ActiveEventLoop,
    size: WindowSize,
    settings: &DesktopSettingsManager,
) -> (WinitWindow, PossiblyCurrentContext, Surface<WindowSurface>) {
    let template = ConfigTemplateBuilder::new().with_alpha_size(8);
    let display_builder = DisplayBuilder::new()
        .with_window_attributes(Some(create_window_attributes(size, settings)));
    let (window, gl_config) = display_builder
        .build(event_loop, template, |configs| {
            configs
                .reduce(|accum, config| {
                    if config.num_samples() > accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .unwrap()
        })
        .unwrap();
    let window = window.unwrap();
    let attrs = window
        .build_surface_attributes(Default::default())
        .expect("failed to build GL surface attributes");
    let gl_surface = unsafe {
        gl_config
            .display()
            .create_window_surface(&gl_config, &attrs)
            .expect("failed to create GL surface")
    };
    let gl_context = create_gl_context(&window, &gl_config)
        .make_current(&gl_surface)
        .expect("failed to make GL context current");

    let gl_display = gl_config.display();
    GlBackend::load_with(|symbol| {
        let symbol = CString::new(symbol).unwrap();
        gl_display.get_proc_address(symbol.as_c_str()).cast()
    });

    let _ =
        gl_surface.set_swap_interval(&gl_context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()));

    if current_or_default(settings).video.fullscreen {
        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
    }

    (window, gl_context, gl_surface)
}

fn physical_key_settings_key(key: PhysicalKey) -> Option<KeyboardKey> {
    Some(match key {
        PhysicalKey::Code(KeyCode::KeyA) => KeyboardKey::KeyA,
        PhysicalKey::Code(KeyCode::KeyB) => KeyboardKey::KeyB,
        PhysicalKey::Code(KeyCode::KeyC) => KeyboardKey::KeyC,
        PhysicalKey::Code(KeyCode::KeyD) => KeyboardKey::KeyD,
        PhysicalKey::Code(KeyCode::KeyE) => KeyboardKey::KeyE,
        PhysicalKey::Code(KeyCode::KeyF) => KeyboardKey::KeyF,
        PhysicalKey::Code(KeyCode::KeyG) => KeyboardKey::KeyG,
        PhysicalKey::Code(KeyCode::KeyH) => KeyboardKey::KeyH,
        PhysicalKey::Code(KeyCode::KeyI) => KeyboardKey::KeyI,
        PhysicalKey::Code(KeyCode::KeyJ) => KeyboardKey::KeyJ,
        PhysicalKey::Code(KeyCode::KeyK) => KeyboardKey::KeyK,
        PhysicalKey::Code(KeyCode::KeyL) => KeyboardKey::KeyL,
        PhysicalKey::Code(KeyCode::KeyM) => KeyboardKey::KeyM,
        PhysicalKey::Code(KeyCode::KeyN) => KeyboardKey::KeyN,
        PhysicalKey::Code(KeyCode::KeyO) => KeyboardKey::KeyO,
        PhysicalKey::Code(KeyCode::KeyP) => KeyboardKey::KeyP,
        PhysicalKey::Code(KeyCode::KeyQ) => KeyboardKey::KeyQ,
        PhysicalKey::Code(KeyCode::KeyR) => KeyboardKey::KeyR,
        PhysicalKey::Code(KeyCode::KeyS) => KeyboardKey::KeyS,
        PhysicalKey::Code(KeyCode::KeyT) => KeyboardKey::KeyT,
        PhysicalKey::Code(KeyCode::KeyU) => KeyboardKey::KeyU,
        PhysicalKey::Code(KeyCode::KeyV) => KeyboardKey::KeyV,
        PhysicalKey::Code(KeyCode::KeyW) => KeyboardKey::KeyW,
        PhysicalKey::Code(KeyCode::KeyX) => KeyboardKey::KeyX,
        PhysicalKey::Code(KeyCode::KeyY) => KeyboardKey::KeyY,
        PhysicalKey::Code(KeyCode::KeyZ) => KeyboardKey::KeyZ,
        PhysicalKey::Code(KeyCode::ArrowUp) => KeyboardKey::ArrowUp,
        PhysicalKey::Code(KeyCode::ArrowDown) => KeyboardKey::ArrowDown,
        PhysicalKey::Code(KeyCode::ArrowLeft) => KeyboardKey::ArrowLeft,
        PhysicalKey::Code(KeyCode::ArrowRight) => KeyboardKey::ArrowRight,
        PhysicalKey::Code(KeyCode::Enter) => KeyboardKey::Enter,
        PhysicalKey::Code(KeyCode::Escape) => KeyboardKey::Escape,
        PhysicalKey::Code(KeyCode::Space) => KeyboardKey::Space,
        PhysicalKey::Code(KeyCode::Tab) => KeyboardKey::Tab,
        PhysicalKey::Code(KeyCode::F1) => KeyboardKey::F1,
        PhysicalKey::Code(KeyCode::F2) => KeyboardKey::F2,
        PhysicalKey::Code(KeyCode::F3) => KeyboardKey::F3,
        PhysicalKey::Code(KeyCode::F4) => KeyboardKey::F4,
        PhysicalKey::Code(KeyCode::F5) => KeyboardKey::F5,
        PhysicalKey::Code(KeyCode::F6) => KeyboardKey::F6,
        PhysicalKey::Code(KeyCode::F7) => KeyboardKey::F7,
        PhysicalKey::Code(KeyCode::F8) => KeyboardKey::F8,
        PhysicalKey::Code(KeyCode::F9) => KeyboardKey::F9,
        PhysicalKey::Code(KeyCode::F10) => KeyboardKey::F10,
        PhysicalKey::Code(KeyCode::F11) => KeyboardKey::F11,
        PhysicalKey::Code(KeyCode::F12) => KeyboardKey::F12,
        _ => return None,
    })
}

fn element_state_to_pressed(state: ElementState) -> bool {
    matches!(state, ElementState::Pressed)
}

pub struct Window {
    view: Option<GlBackend>,
    gl_context: Option<PossiblyCurrentContext>,
    gl_surface: Option<Surface<WindowSurface>>,
    window: Option<WinitWindow>,
    event_loop: Option<EventLoop<()>>,
    session: NesSession,
    shell: NativeShellState,
    settings: DesktopSettingsManager,
    fullscreened: bool,
}

impl Window {
    pub fn new() -> Self {
        let settings = load_settings_manager();
        let fullscreened = current_or_default(&settings).video.fullscreen;
        Self {
            event_loop: Some(EventLoop::new().unwrap()),
            view: None,
            gl_context: None,
            gl_surface: None,
            window: None,
            session: NesSession::new(settings.clone()),
            shell: NativeShellState::new(),
            fullscreened,
            settings,
        }
    }

    pub fn load(&mut self, rom_path: Option<PathBuf>, data: Vec<u8>) {
        if self.session.load(rom_path.clone(), data)
            && let Some(path) = rom_path.as_deref()
        {
            let _ = self.settings.record_opened_rom(path);
        }
    }

    pub fn run(&mut self) {
        self.session.resume();
        let event_loop = self.event_loop.take().unwrap();
        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop.run_app(self).unwrap();
    }

    fn on_load(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let (window, gl_context, gl_surface) =
            create_window(event_loop, self.session.window_size(), &self.settings);
        let mut view = GlBackend::new();
        view.use_vao(true);
        let video = self.session.video();
        view.on_load(
            video.presentation(),
            video
                .console_video_assets()
                .expect("NES session always has video assets"),
        )
        .unwrap();
        let initial_size = window.inner_size();

        self.window = Some(window);
        self.gl_context = Some(gl_context);
        self.gl_surface = Some(gl_surface);
        self.view = Some(view);
        self.on_resize(initial_size);
        self.refresh_window_title();
    }

    fn on_update(&mut self) {
        self.session.with_frame_buffer(|frame_buffer| {
            self.view.as_ref().unwrap().on_update(frame_buffer);
        });
        self.gl_surface
            .as_ref()
            .unwrap()
            .swap_buffers(self.gl_context.as_ref().unwrap())
            .unwrap();
        self.shell
            .on_frame_presented(self.session.metrics().frame_counter);
        self.maybe_refresh_window_title(Instant::now());
    }

    fn on_resize(&mut self, physical_size: WinitPhysicalSize<u32>) {
        let Some(width) = NonZeroU32::new(physical_size.width) else {
            return;
        };
        let Some(height) = NonZeroU32::new(physical_size.height) else {
            return;
        };

        self.gl_surface
            .as_ref()
            .unwrap()
            .resize(self.gl_context.as_ref().unwrap(), width, height);

        let session_size = self.session.window_size();
        let rate_x = physical_size.width as f32 / session_size.width;
        let rate_y = physical_size.height as f32 / session_size.height;
        let rate = f32::min(rate_x, rate_y);
        let scale_x = rate / rate_x;
        let scale_y = rate / rate_y;

        self.view.as_mut().unwrap().on_resize(
            scale_x,
            scale_y,
            physical_size.width as i32,
            physical_size.height as i32,
        );
        if !self.fullscreened
            && let Some(window) = self.window.as_ref()
        {
            let logical_size = physical_size.to_logical::<f64>(window.scale_factor());
            let width = logical_size.width.round().max(1.0) as u32;
            let height = logical_size.height.round().max(1.0) as u32;
            let _ = self.settings.remember_window_size(width, height);
        }
        self.shell.needs_redraw = true;
    }

    fn toggle_fullscreen(&mut self) {
        self.fullscreened = !self.fullscreened;
        if let Some(window) = self.window.as_ref() {
            window.set_fullscreen(self.fullscreened.then_some(Fullscreen::Borderless(None)));
        }
    }

    fn on_close(&mut self) -> bool {
        self.session.flush_before_exit();
        if let Some(view) = self.view.as_mut() {
            view.on_close();
        }
        self.view = None;
        self.gl_surface = None;
        self.gl_context = None;
        self.window = None;
        true
    }

    fn current_window_title(&self) -> String {
        self.session.window_title()
    }

    fn refresh_window_title(&mut self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(self.current_window_title().as_str());
        }
    }

    fn maybe_refresh_window_title(&mut self, now: Instant) {
        if self.shell.should_refresh_title(now) {
            self.refresh_window_title();
        }
    }

    fn apply_command_outcome(&mut self, outcome: SessionCommandOutcome) {
        if outcome.needs_redraw {
            self.shell.needs_redraw = true;
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
        self.refresh_window_title();
    }

    fn apply_session_command(&mut self, command: SessionCommand) {
        let outcome = self.session.run_command(command);
        if outcome.executed
            && let SessionCommand::SelectNextSlot | SessionCommand::SelectPreviousSlot = command
            && let Some(slot_id) = self.session.active_slot_id()
        {
            log::info!("selected save slot {slot_id}");
        }
        self.apply_command_outcome(outcome);
    }

    fn on_keyboard_input(&mut self, input: KeyEvent) {
        let settings = current_or_default(&self.settings);
        if let Some(key) = physical_key_settings_key(input.physical_key) {
            if input.state == ElementState::Released
                && let Some(controller_event) = controller_event_for_key(&settings, key, false)
            {
                self.session.handle_controller_input(controller_event);
            }
            if input.state == ElementState::Released
                && !input.repeat
                && let Some(action) = shortcut_action_for_key(&settings, key)
                && matches!(action, ShortcutAction::ToggleFullscreen)
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
            if element_state_to_pressed(input.state)
                && let Some(event) = controller_event_for_key(&settings, key, true)
            {
                self.session.handle_controller_input(event);
            }
        }
    }
}

impl ApplicationHandler for Window {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.on_load(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested if self.on_close() => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                self.on_resize(size);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::Focused(false) => {
                let settings = current_or_default(&self.settings);
                if settings.host.clear_input_on_focus_loss {
                    self.session.clear_controller_input();
                }
                if settings.host.pause_on_focus_loss {
                    self.apply_session_command(SessionCommand::Pause);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.on_keyboard_input(event),
            WindowEvent::RedrawRequested => self.on_update(),
            _ => (),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.maybe_refresh_window_title(now);

        let Some(window) = self.window.as_ref() else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };

        let metrics = self.session.metrics();
        if self.shell.wants_redraw(metrics.frame_counter) {
            window.request_redraw();
        }

        if self.shell.wants_poll(metrics.loaded, metrics.paused) {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                now + NativeShellState::FRAME_POLL_INTERVAL,
            ));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.on_close();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.on_close();
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.on_close();
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::physical_key_settings_key;
    use nerust_contract_settings::input::KeyboardKey;
    use winit::keyboard::{KeyCode, PhysicalKey};

    #[test]
    fn physical_key_mapping_matches_settings_keys() {
        assert_eq!(
            physical_key_settings_key(PhysicalKey::Code(KeyCode::KeyZ)),
            Some(KeyboardKey::KeyZ)
        );
        assert_eq!(
            physical_key_settings_key(PhysicalKey::Code(KeyCode::KeyX)),
            Some(KeyboardKey::KeyX)
        );
        assert_eq!(
            physical_key_settings_key(PhysicalKey::Code(KeyCode::ArrowUp)),
            Some(KeyboardKey::ArrowUp)
        );
        assert_eq!(
            physical_key_settings_key(PhysicalKey::Code(KeyCode::ArrowRight)),
            Some(KeyboardKey::ArrowRight)
        );
        assert_eq!(
            physical_key_settings_key(PhysicalKey::Code(KeyCode::Enter)),
            Some(KeyboardKey::Enter)
        );
    }
}
