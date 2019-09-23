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
use nerust_screen_traits::LogicalSize;
use std::os::raw::c_void;
use std::ptr;
use std::rc::Rc;

fn allocate(size: usize) -> Box<[u8]> {
    let mut buffer = Vec::with_capacity(size);
    unsafe {
        buffer.set_len(size);
    }
    buffer.into_boxed_slice()
}

#[derive(Debug)]
pub struct GlView {
    tex_name: u32,
    shader: Option<Shader>,
    use_vao: bool,
    vba: Option<VertexArray>,
    vbo: Option<Rc<VertexBuffer>>,
}

impl GlView {
    pub fn new() -> Self {
        Self {
            tex_name: 0,
            shader: None,
            use_vao: false,
            vba: None,
            vbo: None,
        }
    }

    pub fn use_vao(&mut self, value: bool) {
        self.use_vao = value;
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

        tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32).unwrap();
        tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32).unwrap();
        // tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32).unwrap();
        // tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32).unwrap();

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
                allocate(buffer_width * buffer_height * 4).as_ptr() as *const _,
            )
            .unwrap();
        }

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

        let vbo = Rc::new(VertexBuffer::from_slice(&vertex_data).unwrap());
        if self.use_vao {
            let vbo = vbo.clone();
            self.vba = Some(
                VertexArray::new(|vaic| {
                    vaic.bind_vbo(vbo, |vac| {
                        vac.attr_pointer(
                            Attrib {
                                id: shader.get_attribute("position"),
                            },
                            2,
                            gl::FLOAT,
                            16,
                            0,
                        )?;
                        vac.attr_pointer(
                            Attrib {
                                id: shader.get_attribute("uv"),
                            },
                            2,
                            gl::FLOAT,
                            16,
                            8,
                        )
                    })
                })
                .unwrap(),
            );
        } else {
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
            enable_vertex_attrib_array(shader.get_attribute("position")).unwrap();

            vertex_attrib_pointer(
                shader.get_attribute("uv"),
                2,
                gl::FLOAT,
                gl::FALSE,
                16,
                8 as *const c_void,
            )
            .unwrap();
            enable_vertex_attrib_array(shader.get_attribute("uv")).unwrap();
            self.vbo = Some(vbo.clone());
        }

        // uniform属性を設定する
        uniform_matrix_4fv(
            shader.get_uniform("unif_matrix") as GLint,
            1,
            gl::FALSE,
            Mat4::identity().as_ptr(),
        )
        .unwrap();
        uniform_1i(shader.get_uniform("texture") as GLint, 0).unwrap();

        // bind_buffer(gl::ARRAY_BUFFER, 0).unwrap();
        self.shader = Some(shader);
    }

    pub fn on_update(&self, logical_size: LogicalSize, screen_ptr: *const u8) {
        if self.use_vao {
            self.vba.as_ref().unwrap().bind_vao(|_vac| Ok(())).unwrap();
        } else {
            bind_buffer(gl::ARRAY_BUFFER, self.vbo.as_ref().unwrap().id).unwrap();
        }
        clear(gl::COLOR_BUFFER_BIT).unwrap();

        // モデルの描画
        tex_sub_image_2d(
            gl::TEXTURE_2D,
            0,
            0,
            0,
            logical_size.width as i32,
            logical_size.height as i32,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            screen_ptr as *const c_void,
        )
        .unwrap();
        draw_arrays(gl::TRIANGLE_STRIP, 0, 4).unwrap();
    }

    pub fn on_resize(&mut self, scale_x: f32, scale_y: f32) {
        if self.use_vao {
            self.vba.as_ref().unwrap().bind_vao(|_vac| Ok(())).unwrap();
        } else {
            bind_buffer(gl::ARRAY_BUFFER, self.vbo.as_ref().unwrap().id).unwrap();
        }
        uniform_matrix_4fv(
            self.shader.as_ref().unwrap().get_uniform("unif_matrix") as GLint,
            1,
            gl::FALSE,
            Mat4::scale(scale_x, scale_y, 1.0).as_ptr(),
        )
        .unwrap();
        // bind_buffer(gl::ARRAY_BUFFER, 0).unwrap();
    }

    pub fn on_close(&mut self) {
        delete_textures(1, &self.tex_name).unwrap();
    }
}

impl Default for GlView {
    fn default() -> Self {
        Self::new()
    }
}
