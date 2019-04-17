// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::nes::controller::standard_controller::{Buttons, StandardController};
use crate::nes::Console;
use crc::crc64;
use glutin::{
    dpi, Api, ContextBuilder, DeviceId, ElementState, Event, EventsLoop, GlProfile, GlRequest,
    KeyboardInput, PossiblyCurrent, VirtualKeyCode, WindowBuilder, WindowEvent, WindowedContext,
};
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_opengl::GlView;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_sound_traits::{MixerInput, Sound};
use nerust_timer::{Timer, CLOCK_RATE};
use std::hash::{Hash, Hasher};
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

struct Window {
    view: Option<GlView>,
    context: WindowedContext<PossiblyCurrent>,
    events_loop: Option<EventsLoop>,
    running: bool,
    timer: Timer,
    controller: StandardController,
    keys: Buttons,
    paused: bool,
    frame_counter: u64,
    screen_buffer: ScreenBuffer,
}

impl Window {
    fn new() -> Self {
        let screen_buffer = ScreenBuffer::new(
            FilterType::NtscComposite,
            LogicalSize {
                width: 256,
                height: 240,
            },
        );
        // glutin initialize
        let events_loop = EventsLoop::new();
        // create opengl window
        let context = create_window(&events_loop, screen_buffer.physical_size());
        let view = Some(GlView::new(|symbol| {
            context.get_proc_address(symbol) as *const std::ffi::c_void
        }));

        Self {
            events_loop: Some(events_loop),
            view,
            context,
            running: true,
            timer: Timer::new(),
            controller: StandardController::new(),
            keys: Buttons::empty(),
            paused: false,
            frame_counter: 0,
            screen_buffer,
        }
    }

    fn run<S: Sound + MixerInput>(&mut self, mut console: Console, mut speaker: S) {
        self.on_load();
        speaker.start();
        while self.running {
            self.on_update(&mut console, &mut speaker);
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
                            self.on_keyboard_input(device_id, input, &mut console, &mut speaker);
                        }
                        _ => (),
                    }
                }
            });
            mem::replace(&mut self.events_loop, el);
        }
    }

    fn on_load(&mut self) {
        self.view
            .as_mut()
            .unwrap()
            .on_load(self.screen_buffer.logical_size());
    }

    fn on_update<S: Sound + MixerInput>(&mut self, console: &mut Console, speaker: &mut S) {
        if !self.paused {
            while !console.step(&mut self.screen_buffer, &mut self.controller, speaker) {}
            self.frame_counter += 1;
        }

        self.view
            .as_mut()
            .unwrap()
            .on_update(&mut self.screen_buffer);
        self.context.swap_buffers().unwrap();

        let fps = self.timer.as_fps();
        let title = if self.paused {
            "Nes -- Paused".to_owned()
        } else {
            format!("Nes -- FPS: {:.2}", fps)
        };
        self.context.window().set_title(title.as_str());
    }

    fn on_resize(&mut self, logical_size: dpi::LogicalSize) {
        let dpi_factor = self.context.window().get_hidpi_factor();
        let rate_x = logical_size.width / f64::from(self.screen_buffer.physical_size().width);
        let rate_y = logical_size.height / f64::from(self.screen_buffer.physical_size().height);
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

    fn on_keyboard_input<S: Sound + MixerInput>(
        &mut self,
        _device_id: DeviceId,
        input: KeyboardInput,
        console: &mut Console,
        speaker: &mut S,
    ) {
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
                    speaker.pause();
                    let mut hasher = crc64::Digest::new(crc64::ECMA);
                    self.screen_buffer.hash(&mut hasher);
                    info!(
                        "Paused -- FrameCounter : {}, ScreenHash : 0x{:016X}",
                        self.frame_counter,
                        hasher.finish()
                    );
                } else {
                    speaker.start();
                }
                Buttons::empty()
            }
            Some(VirtualKeyCode::Escape) => {
                if input.state == ElementState::Released {
                    console.reset();
                }
                Buttons::empty()
            }
            _ => Buttons::empty(),
        };
        self.keys = match input.state {
            ElementState::Pressed => self.keys | code,
            ElementState::Released => self.keys & !code,
        };
        self.controller.set_pad1(self.keys);
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        std::mem::replace(&mut self.view, None); // GlViewを先に解放
    }
}

pub struct Gui {
    console: Console,
}

impl Gui {
    pub fn new(console: Console) -> Self {
        Self { console }
    }

    pub fn run(self) {
        // 1024 * 5 = 107ms
        // 512 * 5 = 54ms
        // 256 * 5 = 27ms
        Window::new().run(self.console, OpenAl::new(48000, CLOCK_RATE as i32, 128, 20));
    }
}
