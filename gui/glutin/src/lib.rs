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
use nerust_console::Console;
use nerust_core::controller::standard_controller::Buttons;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_opengl::GlView;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_timer::{CLOCK_RATE, Timer};
use raw_window_handle::HasWindowHandle;
use std::f64;
use std::ffi::CString;
use std::num::NonZeroU32;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize as WinitLogicalSize, PhysicalSize as WinitPhysicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window as WinitWindow, WindowAttributes};

fn create_window_attributes(size: PhysicalSize) -> WindowAttributes {
    WinitWindow::default_attributes()
        .with_inner_size(WinitLogicalSize::new(
            f64::from(size.width),
            f64::from(size.height),
        ))
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
    size: PhysicalSize,
) -> (WinitWindow, PossiblyCurrentContext, Surface<WindowSurface>) {
    let template = ConfigTemplateBuilder::new().with_alpha_size(8);
    let display_builder =
        DisplayBuilder::new().with_window_attributes(Some(create_window_attributes(size)));
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
    GlView::load_with(|symbol| {
        let symbol = CString::new(symbol).unwrap();
        gl_display.get_proc_address(symbol.as_c_str()).cast()
    });

    let _ =
        gl_surface.set_swap_interval(&gl_context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()));

    (window, gl_context, gl_surface)
}

pub struct Window {
    view: Option<GlView>,
    gl_context: Option<PossiblyCurrentContext>,
    gl_surface: Option<Surface<WindowSurface>>,
    window: Option<WinitWindow>,
    event_loop: Option<EventLoop<()>>,
    timer: Timer,
    keys: Buttons,
    paused: bool,
    console: Console,
    physical_size: PhysicalSize,
    logical_size: LogicalSize,
}

impl Window {
    pub fn new() -> Self {
        let screen_buffer = ScreenBuffer::new(
            FilterType::NtscComposite,
            LogicalSize {
                width: 256,
                height: 240,
            },
        );
        let physical_size = screen_buffer.physical_size();
        let logical_size = screen_buffer.logical_size();
        let speaker = OpenAl::new(48000, CLOCK_RATE as i32, 128, 20);
        let console = Console::new(speaker, screen_buffer);

        Self {
            event_loop: Some(EventLoop::new().unwrap()),
            view: None,
            gl_context: None,
            gl_surface: None,
            window: None,
            timer: Timer::new(),
            keys: Buttons::empty(),
            paused: false,
            console,
            physical_size,
            logical_size,
        }
    }

    pub fn load(&mut self, data: Vec<u8>) {
        self.console.load(data);
    }

    pub fn run(&mut self) {
        self.console.resume();
        let event_loop = self.event_loop.take().unwrap();
        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop.run_app(self).unwrap();
    }

    fn on_load(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let (window, gl_context, gl_surface) = create_window(event_loop, self.physical_size);
        let mut view = GlView::new();
        view.use_vao(true);
        view.on_load(self.logical_size);

        self.window = Some(window);
        self.gl_context = Some(gl_context);
        self.gl_surface = Some(gl_surface);
        self.view = Some(view);
    }

    fn on_update(&mut self) {
        let logical_size = self.console.logical_size();
        self.console.with_frame_buffer(|frame_buffer| {
            self.view
                .as_ref()
                .unwrap()
                .on_update(logical_size, frame_buffer.as_ptr());
        });
        self.gl_surface
            .as_ref()
            .unwrap()
            .swap_buffers(self.gl_context.as_ref().unwrap())
            .unwrap();
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

        let rate_x = physical_size.width as f32 / self.physical_size.width;
        let rate_y = physical_size.height as f32 / self.physical_size.height;
        let rate = f32::min(rate_x, rate_y);
        let scale_x = rate / rate_x;
        let scale_y = rate / rate_y;

        self.view.as_mut().unwrap().on_resize(scale_x, scale_y);
    }

    fn on_close(&mut self) {
        if let Some(view) = self.view.as_mut() {
            view.on_close();
        }
        self.view = None;
        self.gl_surface = None;
        self.gl_context = None;
        self.window = None;
    }

    fn on_keyboard_input(&mut self, input: KeyEvent) {
        // とりあえず、pad1のみ次の通りとする。
        // A      -> Z
        // B      -> X
        // Select -> C
        // Start  -> V
        // Up     -> Up
        // Down   -> Down
        // Left   -> Left
        // Right  -> Right
        let code = match input.physical_key {
            PhysicalKey::Code(KeyCode::KeyZ) => Buttons::A,
            PhysicalKey::Code(KeyCode::KeyX) => Buttons::B,
            PhysicalKey::Code(KeyCode::KeyC) => Buttons::SELECT,
            PhysicalKey::Code(KeyCode::KeyV) => Buttons::START,
            PhysicalKey::Code(KeyCode::ArrowUp) => Buttons::UP,
            PhysicalKey::Code(KeyCode::ArrowDown) => Buttons::DOWN,
            PhysicalKey::Code(KeyCode::ArrowLeft) => Buttons::LEFT,
            PhysicalKey::Code(KeyCode::ArrowRight) => Buttons::RIGHT,
            PhysicalKey::Code(KeyCode::Space) if input.state == ElementState::Pressed => {
                self.paused = !self.paused;
                let title = if self.paused {
                    self.console.pause();
                    "Nes -- Paused".to_string()
                } else {
                    self.console.resume();
                    "Nes".to_string()
                };
                self.window.as_ref().unwrap().set_title(title.as_str());
                Buttons::empty()
            }
            PhysicalKey::Code(KeyCode::Escape) => {
                if input.state == ElementState::Released {
                    self.console.reset();
                }
                Buttons::empty()
            }
            _ => Buttons::empty(),
        };
        self.keys = match input.state {
            ElementState::Pressed => self.keys | code,
            ElementState::Released => self.keys & !code,
        };
        self.console.set_pad1(self.keys);
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
            WindowEvent::CloseRequested => {
                self.on_close();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => self.on_resize(size),
            WindowEvent::KeyboardInput { event, .. } => self.on_keyboard_input(event),
            WindowEvent::RedrawRequested => self.on_update(),
            _ => (),
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            self.timer.wait();
            window.request_redraw();
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.on_close();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.on_close();
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.on_close();
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}
