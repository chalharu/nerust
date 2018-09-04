// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// #[macro_use]
// extern crate log;
extern crate gl;
extern crate glutin;
extern crate simple_logger;
#[macro_use]
extern crate failure;

mod glwrap;

use gl::types::GLint;
use glutin::dpi::LogicalSize;
use glutin::{
    Api, ContextBuilder, Event, EventsLoop, GlContext, GlProfile, GlRequest, GlWindow,
    WindowBuilder, WindowEvent,
};
use glwrap::*;
use std::collections::VecDeque;
use std::time::Instant;
use std::{f64, mem};

struct Fps {
    instants: VecDeque<Instant>,
}

impl Fps {
    pub fn new() -> Self {
        let mut instants = VecDeque::new();
        for _ in 0..64 {
            instants.push_back(Instant::now());
        }
        Self { instants }
    }

    pub fn to_fps(&mut self) -> f32 {
        let new_now = Instant::now();
        let duration = new_now.duration_since(self.instants.pop_front().unwrap());
        self.instants.push_back(new_now);
        (1_000_000_f64
            / f64::from(duration.as_secs() as u32 * 1_000_000 + duration.subsec_micros())
            * 64.0) as f32
    }
}

fn update(screen_buffer: &mut [u8]) {
    for (i, s) in screen_buffer.iter_mut().enumerate() {
        match i & 3 {
            0 | 1 | 2 => *s = (*s).wrapping_add(1),
            _ => {}
        }
    }
}

fn create_window(events_loop: &EventsLoop) -> GlWindow {
    let window = WindowBuilder::new()
        .with_dimensions(LogicalSize::new(320.0, 240.0))
        .with_title("Nes");
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

fn init_screen_buffer() -> [u8; 320 * 240 * 4] {
    let mut screen_buffer = [0_u8; 320 * 240 * 4];
    for (i, s) in screen_buffer.iter_mut().enumerate() {
        let p = i & 3;
        let x = (i >> 2) % 320;
        let y = (i >> 2) / 320;
        let r = ((x * 0xFF) / 320) as u8;
        *s = match p {
            0 => r,
            1 => ((y * 0xFF) / 240) as u8,
            2 => !r,
            _ => 0,
        };
    }
    screen_buffer
}

struct Window {
    window: GlWindow,
    events_loop: Option<EventsLoop>,
    running: bool,
    fps: Fps,
    tex_name: u32,
    screen_buffer: [u8; 320 * 240 * 4],
    vertex_vbo: u32,
    shader: Option<Shader>,
}

impl Window {
    fn new() -> Self {
        // glutin initialize
        let events_loop = EventsLoop::new();
        // create opengl window
        let window = create_window(&events_loop);

        Self {
            events_loop: Some(events_loop),
            window,
            running: true,
            fps: Fps::new(),
            tex_name: 0,
            screen_buffer: init_screen_buffer(),
            vertex_vbo: 0,
            shader: None,
        }
    }

    fn run(&mut self) {
        self.on_load();
        while self.running {
            self.on_update();
            let mut el = mem::replace(&mut self.events_loop, None);
            el.as_mut().unwrap().poll_events(|event| match event {
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::CloseRequested => {
                        self.on_close();
                    }
                    WindowEvent::Resized(logical_size) => {
                        self.on_resize(logical_size);
                    }
                    _ => (),
                },
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
        tex_image_2d(
            gl::TEXTURE_2D,
            0,
            gl::RGBA as GLint,
            512,
            256,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            unsafe { mem::transmute([0_u8; 256 * 512 * 4].as_ptr()) },
        ).unwrap();

        tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32).unwrap();
        tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32).unwrap();

        // bind_texture(gl::TEXTURE_2D, 0).unwrap();

        // vbo
        let vertex_data: [VertexData; 4] = [
            VertexData::new(Vec2D::new(-1.0, 1.0), Vec2D::new(0.0, 0.0)),
            VertexData::new(Vec2D::new(-1.0, -1.0), Vec2D::new(0.0, 240.0 / 256.0)),
            VertexData::new(Vec2D::new(1.0, 1.0), Vec2D::new(320.0 / 512.0, 0.0)),
            VertexData::new(
                Vec2D::new(1.0, -1.0),
                Vec2D::new(320.0 / 512.0, 240.0 / 256.0),
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
            0 as *const std::os::raw::c_void,
        ).unwrap();
        vertex_attrib_pointer(
            shader.get_attribute("uv"),
            2,
            gl::FLOAT,
            gl::FALSE,
            16,
            8 as *const std::os::raw::c_void,
        ).unwrap();
        bind_buffer(gl::ARRAY_BUFFER, 0).unwrap();
        self.shader = Some(shader);
    }

    fn on_update(&mut self) {
        update(&mut self.screen_buffer);

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
            320,
            240,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            self.screen_buffer.as_ptr() as *const std::os::raw::c_void,
        ).unwrap();
        draw_arrays(gl::TRIANGLE_STRIP, 0, 4).unwrap();
        self.window.swap_buffers().unwrap();

        let title = format!("Nes -- FPS: {:.2}", self.fps.to_fps());
        self.window.set_title(title.as_str());
    }

    fn on_resize(&mut self, logical_size: LogicalSize) {
        let dpi_factor = self.window.get_hidpi_factor();
        self.window.resize(logical_size.to_physical(dpi_factor));

        let rate_x = logical_size.width / 320.0;
        let rate_y = logical_size.height / 240.0;
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
}

fn main() {
    // log initialize
    simple_logger::init().unwrap();

    let mut window = Window::new();
    window.run();
}
