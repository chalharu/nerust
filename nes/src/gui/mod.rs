// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod filterset;
mod sound;

use crate::glwrap::*;
use crate::nes::controller::standard_controller::{Buttons, StandardController};
use crate::nes::{Console, Screen};
use crc::crc64;
use gl;
use gl::types::GLint;
use glutin::{
    dpi, Api, ContextBuilder, ContextTrait, DeviceId, ElementState, Event, EventsLoop, GlProfile,
    GlRequest, KeyboardInput, VirtualKeyCode, WindowBuilder, WindowEvent, WindowedContext,
};
use std::ops::Add;

use self::filterset::{FilterType, NesFilter};
use self::sound::Sound;
use crate::nes::MixerInput;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::os::raw::c_void;
use std::time::{Duration, Instant};
use std::{f64, mem, ptr, thread};

pub const CLOCK_RATE: usize = 1_789_773;

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct RGB {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl From<u32> for RGB {
    fn from(value: u32) -> RGB {
        RGB {
            red: ((value >> 16) & 0xFF) as u8,
            green: ((value >> 8) & 0xFF) as u8,
            blue: (value & 0xFF) as u8,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct PhysicalSize {
    pub width: f32,
    pub height: f32,
}

impl From<LogicalSize> for PhysicalSize {
    fn from(value: LogicalSize) -> PhysicalSize {
        PhysicalSize {
            width: value.width as f32,
            height: value.height as f32,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct LogicalSize {
    pub width: usize,
    pub height: usize,
}

// const PAR: f64 = 7.0 / 8.0;

struct Fps {
    instants: VecDeque<Instant>,
    wait_instants: Instant,
    thread_sleep_nanos: Duration,
    frame_wait_nanos: Duration,
}

impl Fps {
    pub fn new() -> Self {
        let instants = VecDeque::with_capacity(Self::CALC_FRAMES);
        let wait_instants = Instant::now();
        Self {
            instants,
            wait_instants,
            thread_sleep_nanos: Duration::from_nanos(Self::FRAME_WAIT_NANOS - 1_000_000),
            frame_wait_nanos: Duration::from_nanos(Self::FRAME_WAIT_NANOS),
        }
    }

    const CALC_FRAMES: usize = 64;
    const FRAME_DOTS: f64 = 89341.5;
    const VSYNC_RATE: f64 = CLOCK_RATE as f64 * 3.0 / Self::FRAME_DOTS;
    const FRAME_WAIT_NANOS: u64 = (1.0 / Self::VSYNC_RATE * 1_000_000_000.0) as u64;

    pub fn wait(&mut self) {
        let new_now = Instant::now();
        let duration = new_now.duration_since(self.wait_instants);
        if let Some(wait) = self.thread_sleep_nanos.checked_sub(duration) {
            thread::sleep(wait);
        }
        let next = self.wait_instants.add(self.frame_wait_nanos);
        let mut wait_instants = Instant::now();
        while wait_instants < next {
            wait_instants = Instant::now();
        }
        self.wait_instants = wait_instants;
    }

    pub fn as_fps(&mut self) -> f32 {
        let new_now = Instant::now();
        let len = self.instants.len();
        if len == 0 {
            self.instants.push_back(new_now);
            return 0.0;
        }
        let duration = new_now.duration_since(if len >= Self::CALC_FRAMES {
            self.instants.pop_front().unwrap()
        } else {
            *self.instants.front().unwrap()
        });
        self.instants.push_back(new_now);
        (1_000_000_000_f64
            / (duration.as_secs() * 1_000_000_000 + u64::from(duration.subsec_nanos())) as f64
            * len as f64) as f32
    }
}

fn create_window(events_loop: &EventsLoop, size: PhysicalSize) -> WindowedContext {
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

    unsafe {
        context.make_current().unwrap();
        gl::load_with(|symbol| context.get_proc_address(symbol) as *const std::ffi::c_void);
    }
    context
}

#[repr(packed)]
#[derive(Copy, Clone)]
struct Vec2D {
    pub x: f32,
    pub y: f32,
}

impl Vec2D {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[repr(packed)]
#[derive(Copy, Clone)]
struct Mat4 {
    _data: [[f32; 4]; 4],
}

impl Mat4 {
    // pub fn new(data: [[f32; 4]; 4]) -> Self {
    //     Self { _data: data }
    // }

    pub fn identity() -> Self {
        Self {
            _data: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub fn scale(x: f32, y: f32, z: f32) -> Self {
        Self {
            _data: [
                [x, 0.0, 0.0, 0.0],
                [0.0, y, 0.0, 0.0],
                [0.0, 0.0, z, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub fn as_ptr(&self) -> *const f32 {
        &{ self._data } as *const _ as *const f32
    }
}

#[repr(packed)]
#[derive(Copy, Clone)]
struct VertexData {
    pub position: Vec2D,
    pub uv: Vec2D,
}

impl VertexData {
    pub fn new(position: Vec2D, uv: Vec2D) -> Self {
        Self { position, uv }
    }
}

fn allocate(size: usize) -> Box<[u8]> {
    let mut buffer = Vec::with_capacity(size);
    unsafe {
        buffer.set_len(size);
    }
    buffer.into_boxed_slice()
}

fn init_screen_buffer(size: LogicalSize) -> Box<[u8]> {
    allocate(size.width * size.height * 4)
}

struct ScreenBufferUnit {
    buffer: Box<[u8]>,
    pos: usize,
}

impl ScreenBufferUnit {
    pub fn new(size: LogicalSize) -> Self {
        Self {
            buffer: init_screen_buffer(size),
            pos: 0,
        }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.buffer.as_ref() as *const [u8] as *const u8
    }
}

pub struct ScreenBuffer {
    filter: Box<NesFilter>,
    dest: ScreenBufferUnit,
    src_buffer: Box<[u8]>,
    src_buffer_next: Box<[u8]>,
    // src_size: LogicalSize,
    src_pos: usize,
}

// fn init_screen_buffer(size: Size) -> [u8; 602 * 480 * 4] {
//     let mut screen_buffer = [0_u8; 602 * 480 * 4];
//     for (i, s) in screen_buffer.iter_mut().enumerate() {
//         let p = i & 3;
//         let x = (i >> 2) % 602;
//         let y = (i >> 2) / 602;
//         let r = ((x * 0xFF) / 602) as u8;
//         *s = match p {
//             0 => r,
//             1 => ((y * 0xFF) / 480) as u8,
//             2 => !r,
//             _ => 0,
//         };
//     }
//     screen_buffer
// }

impl ScreenBuffer {
    pub fn new(filter_type: FilterType, src_size: LogicalSize) -> Self {
        let filter = filter_type.generate(src_size);
        let src_buffer_size = src_size.height * src_size.width;
        let src_buffer = allocate(src_buffer_size);
        let src_buffer_next = allocate(src_buffer_size);

        Self {
            dest: ScreenBufferUnit::new(filter.logical_size()),
            filter,
            src_buffer,
            src_buffer_next,
            // src_size,
            src_pos: 0,
        }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.dest.as_ptr()
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.filter.logical_size()
    }

    pub fn physical_size(&self) -> PhysicalSize {
        self.filter.physical_size()
    }

    // pub fn change_filter(&mut self, filter_type: FilterType) {
    //     let filter = filter_type.generate(self.src_size);
    //     let buffer = init_screen_buffer(filter.logical_size());
    //     self.filter = filter;
    //     self.dest_buffer = buffer;
    //     for i in 0..self.dest_pos {
    //         self.dest_buffer[i] = self.src_buffe
    //     }
    // }
}

impl Screen for ScreenBuffer {
    fn push(&mut self, value: u8) {
        let dest = &mut self.dest;
        self.filter.as_mut().push(
            value,
            Box::new(|color: RGB| {
                let pos = dest.pos << 2;
                dest.buffer[pos] = color.red;
                dest.buffer[pos + 1] = color.green;
                dest.buffer[pos + 2] = color.blue;
                dest.pos += 1;
            }),
        );
        self.src_buffer_next[self.src_pos] = value;
        self.src_pos += 1;
    }

    fn render(&mut self) {
        mem::swap(&mut self.src_buffer, &mut self.src_buffer_next);
        self.src_pos = 0;
        self.dest.pos = 0;
    }
}

impl Hash for ScreenBuffer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.src_buffer.hash(state);
    }
}

struct Window {
    context: WindowedContext,
    events_loop: Option<EventsLoop>,
    running: bool,
    fps: Fps,
    tex_name: u32,
    screen_buffer: ScreenBuffer,
    vertex_vbo: u32,
    shader: Option<Shader>,
    controller: StandardController,
    keys: Buttons,
    paused: bool,
    frame_counter: u64,
}

impl Window {
    fn new() -> Self {
        let screen_buffer = ScreenBuffer::new(
            // FilterType::NtscComposite,
            FilterType::None,
            LogicalSize {
                width: 256,
                height: 240,
            },
        );
        // glutin initialize
        let events_loop = EventsLoop::new();
        // create opengl window
        let context = create_window(&events_loop, screen_buffer.physical_size());

        Self {
            events_loop: Some(events_loop),
            context,
            running: true,
            fps: Fps::new(),
            tex_name: 0,
            screen_buffer,
            vertex_vbo: 0,
            shader: None,
            controller: StandardController::new(),
            keys: Buttons::empty(),
            paused: false,
            frame_counter: 0,
        }
    }

    fn run<S: Sound + MixerInput>(&mut self, mut console: Console, mut speaker: S) {
        self.on_load();
        speaker.start();
        while self.running {
            self.on_update(&mut console, &mut speaker);
            self.fps.wait();
            let mut el = mem::replace(&mut self.events_loop, None);
            el.as_mut().unwrap().poll_events(|event|
                if let Event::WindowEvent { event, .. } = event { match event {
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
                }}
                // Event::DeviceEvent { device_id, event } => match event {
                //     DeviceEvent::Key(input) => {
                //         self.on_keyboard_input(device_id, input);
                //     }
                //     _ => (),
                // },
            );
            mem::replace(&mut self.events_loop, el);
        }
    }

    fn on_load(&mut self) {
        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("flagment.glsl"));
        shader.use_program();

        // テクスチャのセットアップ
        gen_textures(1, &mut self.tex_name).unwrap();
        pixel_storei(gl::UNPACK_ALIGNMENT, 4).unwrap();

        bind_texture(gl::TEXTURE_2D, self.tex_name).unwrap();

        let logical_size = self.screen_buffer.logical_size();
        let buffer_width = logical_size.width.next_power_of_two();
        let buffer_height = logical_size.height.next_power_of_two();

        tex_image_2d(
            gl::TEXTURE_2D,
            0,
            gl::RGBA as GLint,
            buffer_width as i32,
            buffer_height as i32,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            init_screen_buffer(LogicalSize {
                width: buffer_width,
                height: buffer_height,
            })
            .as_ptr() as *const std::ffi::c_void,
        )
        .unwrap();

        tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32).unwrap();
        tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32).unwrap();
        // tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32).unwrap();
        // tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32).unwrap();

        // bind_texture(gl::TEXTURE_2D, 0).unwrap();

        // vbo
        let vertex_data: [VertexData; 4] = [
            VertexData::new(Vec2D::new(-1.0, 1.0), Vec2D::new(0.0, 0.0)),
            VertexData::new(
                Vec2D::new(-1.0, -1.0),
                Vec2D::new(0.0, logical_size.height as f32 / buffer_height as f32),
            ),
            VertexData::new(
                Vec2D::new(1.0, 1.0),
                Vec2D::new(logical_size.width as f32 / buffer_width as f32, 0.0),
            ),
            VertexData::new(
                Vec2D::new(1.0, -1.0),
                Vec2D::new(
                    logical_size.width as f32 / buffer_width as f32,
                    logical_size.height as f32 / buffer_height as f32,
                ),
            ),
        ];

        gen_buffers(1, &mut self.vertex_vbo).unwrap();
        bind_buffer(gl::ARRAY_BUFFER, self.vertex_vbo).unwrap();
        buffer_data(
            gl::ARRAY_BUFFER,
            mem::size_of_val(&vertex_data) as isize,
            vertex_data.as_ptr() as *const std::ffi::c_void,
            gl::STATIC_DRAW,
        )
        .unwrap();

        // attribute属性を有効にする
        enable_vertex_attrib_array(shader.get_attribute("position")).unwrap();
        enable_vertex_attrib_array(shader.get_attribute("uv")).unwrap();

        // uniform属性を設定する
        uniform_matrix_4fv(
            shader.get_uniform("unif_matrix") as GLint,
            1,
            gl::FALSE,
            Mat4::identity().as_ptr(),
        )
        .unwrap();
        uniform_1i(shader.get_uniform("texture") as GLint, 0).unwrap();

        // attribute属性を登録
        vertex_attrib_pointer(
            shader.get_attribute("position"),
            2,
            gl::FLOAT,
            gl::FALSE,
            16,
            ptr::null(),
        )
        .unwrap();
        vertex_attrib_pointer(
            shader.get_attribute("uv"),
            2,
            gl::FLOAT,
            gl::FALSE,
            16,
            8 as *const c_void,
        )
        .unwrap();
        bind_buffer(gl::ARRAY_BUFFER, 0).unwrap();
        self.shader = Some(shader);
    }

    fn on_update<S: Sound + MixerInput>(&mut self, console: &mut Console, speaker: &mut S) {
        if !self.paused {
            while !console.step(&mut self.screen_buffer, &mut self.controller, speaker) {}
            self.frame_counter += 1;
        }

        // clear_color(0.0, 0.0, 0.0, 0.0).unwrap();
        // clear_depth(1.0).unwrap();
        // clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT).unwrap();
        clear(gl::COLOR_BUFFER_BIT).unwrap();

        // モデルの描画
        // bind_texture(gl::TEXTURE_2D, self.tex_name).unwrap();
        tex_sub_image_2d(
            gl::TEXTURE_2D,
            0,
            0,
            0,
            self.screen_buffer.logical_size().width as i32,
            self.screen_buffer.logical_size().height as i32,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            self.screen_buffer.as_ptr() as *const c_void,
        )
        .unwrap();
        draw_arrays(gl::TRIANGLE_STRIP, 0, 4).unwrap();
        self.context.swap_buffers().unwrap();

        let fps = self.fps.as_fps();
        let title = if self.paused {
            "Nes -- Paused".to_owned()
        } else {
            format!("Nes -- FPS: {:.2}", fps)
        };
        self.context.set_title(title.as_str());
    }

    fn on_resize(&mut self, logical_size: dpi::LogicalSize) {
        let dpi_factor = self.context.get_hidpi_factor();
        self.context.resize(logical_size.to_physical(dpi_factor));

        let physical_size = self.screen_buffer.physical_size();

        let rate_x = logical_size.width / f64::from(physical_size.width);
        let rate_y = logical_size.height / f64::from(physical_size.height);
        let rate = f64::min(rate_x, rate_y);
        let scale_x = (rate / rate_x) as f32;
        let scale_y = (rate / rate_y) as f32;

        gen_buffers(1, &mut self.vertex_vbo).unwrap();
        bind_buffer(gl::ARRAY_BUFFER, self.vertex_vbo).unwrap();
        uniform_matrix_4fv(
            self.shader.as_ref().unwrap().get_uniform("unif_matrix") as GLint,
            1,
            gl::FALSE,
            Mat4::scale(scale_x, scale_y, 1.0).as_ptr(),
        )
        .unwrap();
        bind_buffer(gl::ARRAY_BUFFER, 0).unwrap();
    }

    fn on_close(&mut self) {
        self.running = false;
        delete_buffers(1, &self.vertex_vbo).unwrap();
        delete_textures(1, &self.tex_name).unwrap();
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
        Window::new().run(self.console, sound::OpenAl::new(48000, 128, 20));
    }
}
