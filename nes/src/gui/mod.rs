// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod filterset;

use alto::*;
use crate::glwrap::*;
use crate::nes::controller::standard_controller::{Buttons, StandardController};
use crate::nes::{Console, NesMixer, Screen};
use crc::crc64;
use gl;
use gl::types::GLint;
use glutin::{
    dpi, Api, ContextBuilder, DeviceId, ElementState, Event, EventsLoop, GlContext, GlProfile,
    GlRequest, GlWindow, KeyboardInput, VirtualKeyCode, WindowBuilder, WindowEvent,
};
use std::ops::Add;

use self::filterset::{FilterType, NesFilter};
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::os::raw::c_void;
use std::time::{Duration, Instant};
use std::{f64, mem, thread};

const CLOCK_RATE: usize = 1_789_773;

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
        let instants = VecDeque::with_capacity(64);
        let wait_instants = Instant::now();
        Self {
            instants,
            wait_instants,
            thread_sleep_nanos: Duration::from_nanos(Self::FRAME_WAIT_NANOS - 1_000_000),
            frame_wait_nanos: Duration::from_nanos(Self::FRAME_WAIT_NANOS),
        }
    }

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

    pub fn to_fps(&mut self) -> f32 {
        let new_now = Instant::now();
        let len = self.instants.len();
        if len == 0 {
            self.instants.push_back(new_now);
            return 0.0;
        }
        let duration = new_now.duration_since(if len >= 64 {
            self.instants.pop_front().unwrap()
        } else {
            *self.instants.front().unwrap()
        });
        self.instants.push_back(new_now);
        (1_000_000_000_f64
            / f64::from(duration.as_secs() as u32 * 1_000_000_000 + duration.subsec_nanos())
            * len as f64) as f32
    }
}

fn create_window(events_loop: &EventsLoop, size: PhysicalSize) -> GlWindow {
    let window = WindowBuilder::new()
        .with_dimensions(dpi::LogicalSize::new(
            f64::from(size.width),
            f64::from(size.height),
        )).with_title("Nes");
    let context = ContextBuilder::new()
        .with_double_buffer(Some(true))
        .with_gl_profile(GlProfile::Compatibility)
        // .with_vsync(true)
        .with_gl(GlRequest::Specific(Api::OpenGlEs, (2, 0)));

    let gl_window = GlWindow::new(window, context, events_loop).unwrap();

    unsafe {
        gl_window.make_current().unwrap();
        gl::load_with(|symbol| mem::transmute(gl_window.get_proc_address(symbol)));
    }
    gl_window
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
        self as *const Self as *const f32
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
    window: GlWindow,
    events_loop: Option<EventsLoop>,
    running: bool,
    fps: Fps,
    tex_name: u32,
    screen_buffer: ScreenBuffer,
    vertex_vbo: u32,
    shader: Option<Shader>,
    console: Console,
    controller: StandardController,
    keys: Buttons,
    speaker: AlSpeaker,
    paused: bool,
    frame_counter: u64,
}

impl Window {
    fn new(console: Console, speaker: AlSpeaker) -> Self {
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
        let window = create_window(&events_loop, screen_buffer.physical_size());

        Self {
            events_loop: Some(events_loop),
            window,
            running: true,
            fps: Fps::new(),
            tex_name: 0,
            screen_buffer,
            vertex_vbo: 0,
            shader: None,
            console,
            controller: StandardController::new(),
            keys: Buttons::empty(),
            speaker,
            paused: false,
            frame_counter: 0,
        }
    }

    fn run(&mut self) {
        self.on_load();
        while self.running {
            self.on_update();
            self.fps.wait();
            let mut el = mem::replace(&mut self.events_loop, None);
            el.as_mut().unwrap().poll_events(|event| match event {
                Event::WindowEvent { event, .. } => match event {
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
                },
                // Event::DeviceEvent { device_id, event } => match event {
                //     DeviceEvent::Key(input) => {
                //         self.on_keyboard_input(device_id, input);
                //     }
                //     _ => (),
                // },
                _ => (),
            });
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
            unsafe {
                mem::transmute(
                    init_screen_buffer(LogicalSize {
                        width: buffer_width,
                        height: buffer_height,
                    }).as_ptr(),
                )
            },
        ).unwrap();

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
            unsafe { mem::transmute(vertex_data.as_ptr()) },
            gl::STATIC_DRAW,
        ).unwrap();

        // attribute属性を有効にする
        enable_vertex_attrib_array(shader.get_attribute("position")).unwrap();
        enable_vertex_attrib_array(shader.get_attribute("uv")).unwrap();

        // uniform属性を設定する
        uniform_matrix_4fv(
            shader.get_uniform("unif_matrix") as GLint,
            1,
            gl::FALSE,
            unsafe { mem::transmute(Mat4::identity().as_ptr()) },
        ).unwrap();
        uniform_1i(shader.get_uniform("texture") as GLint, 0).unwrap();

        // attribute属性を登録
        vertex_attrib_pointer(
            shader.get_attribute("position"),
            2,
            gl::FLOAT,
            gl::FALSE,
            16,
            0 as *const c_void,
        ).unwrap();
        vertex_attrib_pointer(
            shader.get_attribute("uv"),
            2,
            gl::FLOAT,
            gl::FALSE,
            16,
            8 as *const c_void,
        ).unwrap();
        bind_buffer(gl::ARRAY_BUFFER, 0).unwrap();
        self.shader = Some(shader);
    }

    fn on_update(&mut self) {
        if !self.paused {
            let mut speak_count = 0;
            while !self.console.step(
                &mut self.screen_buffer,
                &mut self.controller,
                self.speaker.mixer_mut(),
            ) {
                if speak_count == 0 {
                    speak_count = 1000;
                    self.speaker.step();
                } else {
                    speak_count -= 1;
                }
            }
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
        ).unwrap();
        draw_arrays(gl::TRIANGLE_STRIP, 0, 4).unwrap();
        self.window.swap_buffers().unwrap();

        let fps = self.fps.to_fps();
        let title = if self.paused {
            "Nes -- Paused".to_owned()
        } else {
            format!("Nes -- FPS: {:.2}", fps)
        };
        self.window.set_title(title.as_str());
    }

    fn on_resize(&mut self, logical_size: dpi::LogicalSize) {
        let dpi_factor = self.window.get_hidpi_factor();
        self.window.resize(logical_size.to_physical(dpi_factor));

        let physical_size = self.screen_buffer.physical_size();

        let rate_x = logical_size.width / physical_size.width as f64;
        let rate_y = logical_size.height / physical_size.height as f64;
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
        ).unwrap();
        bind_buffer(gl::ARRAY_BUFFER, 0).unwrap();
    }

    fn on_close(&mut self) {
        self.running = false;
        delete_buffers(1, &self.vertex_vbo).unwrap();
        delete_textures(1, &self.tex_name).unwrap();
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
                    self.speaker.pause();
                    let mut hasher = crc64::Digest::new(crc64::ECMA);
                    self.screen_buffer.hash(&mut hasher);
                    info!(
                        "Paused -- FrameCounter : {}, ScreenHash : 0x{:016X}",
                        self.frame_counter,
                        hasher.finish()
                    );
                } else {
                    self.speaker.resume();
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
        self.controller.set_pad1(self.keys);
    }
}

struct AlSpeaker {
    // alto: Option<Alto>,
    // dev: Option<OutputDevice>,
    // ctx: Option<Context>,
    src: Option<StreamingSource>,
    mixer: NesMixer,
    sample_rate: i32,
}

impl AlSpeaker {
    pub fn new(sample_rate: i32, buffer_width: usize, buffer_count: usize) -> Self {
        let mut mixer =
            NesMixer::nes_mixer(sample_rate as f32, buffer_width * buffer_count, CLOCK_RATE);
        let src = if let Ok(src) = Alto::load_default()
            .and_then(|alto| alto.open(None))
            .and_then(|dev| dev.new_context(None))
            .and_then(|ctx| ctx.new_streaming_source().map(|src| (src, ctx)))
            .and_then(|(mut src, mut ctx)| {
                for _ in 0..buffer_count {
                    Self::add_buffer(&mut ctx, &mut src, sample_rate, &mut mixer, buffer_width);
                }
                Ok(src)
            }) {
            Some(src)
        } else {
            error!("No OpenAL implementation present!");
            None
        };

        Self {
            src,
            mixer,
            sample_rate,
        }
    }

    pub fn pause(&mut self) {
        if let Some(ref mut src) = self.src.as_mut() {
            match src.state() {
                SourceState::Playing => src.pause(),
                _ => (),
            }
        }
    }

    pub fn resume(&mut self) {
        if let Some(ref mut src) = self.src.as_mut() {
            match src.state() {
                SourceState::Playing => (),
                _ => src.play(),
            }
        }
    }

    pub fn mixer_mut(&mut self) -> &mut NesMixer {
        &mut self.mixer
    }

    fn add_buffer(
        ctx: &mut Context,
        src: &mut StreamingSource,
        sample_rate: i32,
        mixer: &mut NesMixer,
        buffer_width: usize,
    ) {
        let data = &mixer
            .take(buffer_width)
            .map(|x| Mono { center: x })
            .collect::<Vec<_>>();
        let buf = ctx.new_buffer(data, sample_rate).unwrap();
        src.queue_buffer(buf).unwrap();
    }

    fn fill_buffer(src: &mut StreamingSource, sample_rate: i32, mixer: &mut NesMixer) {
        let mut buf = src.unqueue_buffer().unwrap();
        let data = &mixer
            .take(buf.size() as usize / 2) // i16 でのバイト数
            .map(|x| Mono { center: x })
            .collect::<Vec<_>>();
        buf.set_data(data, sample_rate).unwrap();
        src.queue_buffer(buf).unwrap();
    }

    pub fn step(&mut self) {
        if let Some(ref mut src) = self.src.as_mut() {
            let buffers_processed = src.buffers_processed();
            for _ in 0..buffers_processed {
                Self::fill_buffer(src, self.sample_rate, &mut self.mixer);
            }
            match src.state() {
                SourceState::Playing => (),
                _ => src.play(),
            }
        }
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
        Window::new(self.console, AlSpeaker::new(48000, 128, 15)).run();
    }
}
