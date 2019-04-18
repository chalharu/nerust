// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod mat4;
mod vec2d;
mod vertex_data;

use self::mat4::Mat4;
use self::vec2d::Vec2D;
use self::vertex_data::VertexData;
use gl;
use gl::types::GLint;
use nerust_glwrap::*;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_traits::LogicalSize;
use std::os::raw::c_void;
use std::{mem, ptr};

fn allocate(size: usize) -> Box<[u8]> {
    let mut buffer = Vec::with_capacity(size);
    unsafe {
        buffer.set_len(size);
    }
    buffer.into_boxed_slice()
}

pub struct GlView {
    tex_name: u32,
    vertex_vbo: u32,
    shader: Option<Shader>,
}

impl GlView {
    pub fn new() -> Self {
        Self {
            tex_name: 0,
            vertex_vbo: 0,
            shader: None,
        }
    }

    pub fn load_with<F: FnMut(&'static str) -> *const c_void>(get_proc_address: F) {
        gl::load_with(get_proc_address);
    }

    pub fn on_load(&mut self, logical_size: LogicalSize) {
        let shader = Shader::new(include_str!("vertex.glsl"), include_str!("flagment.glsl"));
        shader.use_program();

        // テクスチャのセットアップ
        gen_textures(1, &mut self.tex_name).unwrap();
        pixel_storei(gl::UNPACK_ALIGNMENT, 4).unwrap();

        bind_texture(gl::TEXTURE_2D, self.tex_name).unwrap();

        let buffer_width = logical_size.width.next_power_of_two();
        let buffer_height = logical_size.height.next_power_of_two();

        {
            tex_image_2d(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as GLint,
                buffer_width as i32,
                buffer_height as i32,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                allocate(buffer_width * buffer_height * 4).as_ptr() as *const std::ffi::c_void,
            )
            .unwrap();
        }

        tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32).unwrap();
        tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32).unwrap();
        // tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32).unwrap();
        // tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32).unwrap();

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

    pub fn on_update(&self, screen_buffer: &ScreenBuffer) {
        clear(gl::COLOR_BUFFER_BIT).unwrap();

        // モデルの描画
        tex_sub_image_2d(
            gl::TEXTURE_2D,
            0,
            0,
            0,
            screen_buffer.logical_size().width as i32,
            screen_buffer.logical_size().height as i32,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            screen_buffer.as_ptr() as *const c_void,
        )
        .unwrap();
        draw_arrays(gl::TRIANGLE_STRIP, 0, 4).unwrap();
    }

    pub fn on_resize(&mut self, scale_x: f32, scale_y: f32) {
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

    pub fn on_close(&mut self) {
        delete_buffers(1, &self.vertex_vbo).unwrap();
        delete_textures(1, &self.tex_name).unwrap();
    }
}
