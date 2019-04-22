// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[macro_use]
extern crate log;

mod async_console;

use async_console::AsyncConsole;
use glutin::{
    dpi, Api, ContextBuilder, DeviceId, ElementState, Event, EventsLoop, GlProfile, GlRequest,
    KeyboardInput, PossiblyCurrent, VirtualKeyCode, WindowBuilder, WindowEvent, WindowedContext,
};
use nerust_core::controller::standard_controller::Buttons;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_opengl::GlView;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_timer::{Timer, CLOCK_RATE};
use std::{f64, mem};

fn create_window(events_loop: &EventsLoop, size: PhysicalSize) -> WindowedContext<PossiblyCurrent> {
    let window = WindowBuilder::new()
        .with_dimensions(dpi::LogicalSize::new(
            f64::from(size.width),
            f64::from(size.height),
        ))
        .with_title("Nes");
    let context = ContextBuilder::new()
        .with_double_buffer(Some(true))
        .with_gl_profile(GlProfile::Compatibility)
        // .with_vsync(true)
        .with_gl(GlRequest::Specific(Api::OpenGlEs, (2, 0)))
        .build_windowed(window, &events_loop)
        .unwrap();

    unsafe { context.make_current().unwrap() }
}

pub struct Window {
    view: Option<GlView>,
    context: WindowedContext<PossiblyCurrent>,
    events_loop: Option<EventsLoop>,
    running: bool,
    timer: Timer,
    keys: Buttons,
    paused: bool,
    console: AsyncConsole,
    physical_size: PhysicalSize,
    logical_size: LogicalSize,
}

impl Window {
    pub fn new() -> Self {
        // glutin initialize
        let events_loop = EventsLoop::new();
        // create opengl window
        let screen_buffer = ScreenBuffer::new(
            FilterType::NtscComposite,
            LogicalSize {
                width: 256,
                height: 240,
            },
        );
        let physical_size = screen_buffer.physical_size();
        let logical_size = screen_buffer.logical_size();
        let context = create_window(&events_loop, physical_size);
        GlView::load_with(|symbol| context.get_proc_address(symbol) as *const std::ffi::c_void);
        let view = Some(GlView::new());

        // 1024 * 5 = 107ms
        // 512 * 5 = 54ms
        // 256 * 5 = 27ms
        let speaker = OpenAl::new(48000, CLOCK_RATE as i32, 128, 20);
        let console = AsyncConsole::new(speaker, screen_buffer);

        Self {
            events_loop: Some(events_loop),
            view,
            context,
            running: true,
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
        self.on_load();
        self.console.resume();
        while self.running {
            self.on_update();
            self.timer.wait();
            let mut el = mem::replace(&mut self.events_loop, None);
            el.as_mut().unwrap().poll_events(|event| {
                if let Event::WindowEvent { event, .. } = event {
                    match event {
                        WindowEvent::CloseRequested => {
                            self.on_close();
                        }
                        WindowEvent::Resized(logical_size) => {
                            self.on_resize(logical_size);
                        }
                        WindowEvent::KeyboardInput { device_id, input } => {
                            self.on_keyboard_input(device_id, input);
                        }
                        _ => (),
                    }
                }
            });
            mem::replace(&mut self.events_loop, el);
        }
    }

    fn on_load(&mut self) {
        self.view.as_mut().unwrap().on_load(self.logical_size);
    }

    fn on_update(&mut self) {
        self.view
            .as_mut()
            .unwrap()
            .on_update(self.console.logical_size(), self.console.as_ptr());
        self.context.swap_buffers().unwrap();

        let title = if self.paused {
            "Nes -- Paused".to_owned()
        } else {
            format!("Nes")
        };
        self.context.window().set_title(title.as_str());
    }

    fn on_resize(&mut self, logical_size: dpi::LogicalSize) {
        let dpi_factor = self.context.window().get_hidpi_factor();
        let rate_x = logical_size.width / f64::from(self.physical_size.width);
        let rate_y = logical_size.height / f64::from(self.physical_size.height);
        let rate = f64::min(rate_x, rate_y);
        let scale_x = (rate / rate_x) as f32;
        let scale_y = (rate / rate_y) as f32;

        self.context.resize(logical_size.to_physical(dpi_factor));

        self.view.as_mut().unwrap().on_resize(scale_x, scale_y);
    }

    fn on_close(&mut self) {
        self.running = false;
        self.view.as_mut().unwrap().on_close();
    }

    fn on_keyboard_input(&mut self, _device_id: DeviceId, input: KeyboardInput) {
        // とりあえず、pad1のみ次の通りとする。
        // A      -> Z
        // B      -> X
        // Select -> C
        // Start  -> V
        // Up     -> Up
        // Down   -> Down
        // Left   -> Left
        // Right  -> Right
        let code = match input.virtual_keycode {
            Some(VirtualKeyCode::Z) => Buttons::A,
            Some(VirtualKeyCode::X) => Buttons::B,
            Some(VirtualKeyCode::C) => Buttons::SELECT,
            Some(VirtualKeyCode::V) => Buttons::START,
            Some(VirtualKeyCode::Up) => Buttons::UP,
            Some(VirtualKeyCode::Down) => Buttons::DOWN,
            Some(VirtualKeyCode::Left) => Buttons::LEFT,
            Some(VirtualKeyCode::Right) => Buttons::RIGHT,
            Some(VirtualKeyCode::Space) if input.state == ElementState::Pressed => {
                self.paused = !self.paused;
                if self.paused {
                    self.console.pause();
                } else {
                    self.console.resume();
                }
                Buttons::empty()
            }
            Some(VirtualKeyCode::Escape) => {
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

impl Drop for Window {
    fn drop(&mut self) {
        std::mem::replace(&mut self.view, None); // GlViewを先に解放
    }
}
