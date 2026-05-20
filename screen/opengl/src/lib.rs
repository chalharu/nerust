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
use gl::types::GLint;
use nerust_glwrap::*;
use nerust_screen_filter::{
    FilterType, NTSC_TEXTURE_HEIGHT, NTSC_TEXTURE_WIDTH, PALETTE_TEXTURE_WIDTH,
};
use nerust_screen_traits::LogicalSize;
use std::ffi::CStr;
use std::os::raw::c_void;
use std::ptr;
use std::rc::Rc;

const GL_LUMINANCE: u32 = 0x1909;

fn allocate(size: usize) -> Box<[u8]> {
    vec![0; size].into_boxed_slice()
}

#[derive(Debug)]
pub struct GlView {
    frame_texture: u32,
    palette_texture: u32,
    ntsc_primary_texture: u32,
    ntsc_secondary_texture: u32,
    shader: Option<Shader>,
    pipeline_mode: ShaderPipelineMode,
    use_vao: bool,
    vba: Option<VertexArray>,
    vbo: Option<Rc<VertexBuffer>>,
    source_logical_size: LogicalSize,
    single_channel_format: SingleChannelFormat,
}

#[derive(Debug, Clone, Copy)]
enum SingleChannelFormat {
    Red,
    Luminance,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ShaderPipelineMode {
    DesktopCore,
    DesktopLegacy,
    Gles2,
}

impl ShaderPipelineMode {
    fn uses_packed_ntsc_lut(self) -> bool {
        matches!(self, Self::DesktopCore)
    }
}

impl GlView {
    pub fn new() -> Self {
        Self {
            frame_texture: 0,
            palette_texture: 0,
            ntsc_primary_texture: 0,
            ntsc_secondary_texture: 0,
            shader: None,
            pipeline_mode: ShaderPipelineMode::DesktopLegacy,
            use_vao: false,
            vba: None,
            vbo: None,
            source_logical_size: LogicalSize {
                width: 0,
                height: 0,
            },
            single_channel_format: SingleChannelFormat::Red,
        }
    }

    pub fn use_vao(&mut self, value: bool) {
        self.use_vao = value;
    }

    pub fn load_with<F: FnMut(&'static str) -> *const c_void>(get_proc_address: F) {
        gl::load_with(get_proc_address);
    }

    pub fn on_load(&mut self, filter_type: FilterType, source_logical_size: LogicalSize) {
        let layout = filter_type.layout(source_logical_size);
        let logical_size = layout.logical_size;
        let (shader, pipeline_mode, single_channel_format) = compile_shader_program();
        self.source_logical_size = source_logical_size;
        self.pipeline_mode = pipeline_mode;
        self.single_channel_format = single_channel_format;
        shader.use_program();
        clear_color(0.0, 0.0, 0.0, 1.0).unwrap();

        let mut texture_names = [0; 4];
        gen_textures(4, texture_names.as_mut_ptr()).unwrap();
        self.frame_texture = texture_names[0];
        self.palette_texture = texture_names[1];
        self.ntsc_primary_texture = texture_names[2];
        self.ntsc_secondary_texture = texture_names[3];
        pixel_storei(gl::UNPACK_ALIGNMENT, 1).unwrap();

        let frame_buffer_width = source_logical_size.width.next_power_of_two();
        let frame_buffer_height = source_logical_size.height.next_power_of_two();
        configure_palette_frame_texture(
            0,
            self.frame_texture,
            frame_buffer_width,
            frame_buffer_height,
            single_channel_format,
        );
        configure_rgba_texture(
            1,
            self.palette_texture,
            PALETTE_TEXTURE_WIDTH as usize,
            1,
            filter_type.encoded_palette_rgba8().as_ref(),
        );
        if self.pipeline_mode.uses_packed_ntsc_lut() {
            if let Some(texture) = filter_type.encoded_packed_ntsc_texture_rgba8() {
                configure_rgba8ui_texture(
                    2,
                    self.ntsc_primary_texture,
                    NTSC_TEXTURE_WIDTH as usize,
                    NTSC_TEXTURE_HEIGHT as usize,
                    texture.rgba8.as_ref(),
                );
            } else {
                configure_rgba8ui_texture(2, self.ntsc_primary_texture, 1, 1, &[0, 0, 0, 0]);
            }
            configure_rgba_texture(3, self.ntsc_secondary_texture, 1, 1, &[0, 0, 0, 0]);
        } else if let Some(textures) = filter_type.encoded_ntsc_textures_rgba8() {
            configure_rgba_texture(
                2,
                self.ntsc_primary_texture,
                NTSC_TEXTURE_WIDTH as usize,
                NTSC_TEXTURE_HEIGHT as usize,
                textures.primary_rgba8.as_ref(),
            );
            configure_rgba_texture(
                3,
                self.ntsc_secondary_texture,
                NTSC_TEXTURE_WIDTH as usize,
                NTSC_TEXTURE_HEIGHT as usize,
                textures.secondary_rgba8.as_ref(),
            );
        } else {
            configure_rgba_texture(2, self.ntsc_primary_texture, 1, 1, &[0, 0, 0, 0]);
            configure_rgba_texture(3, self.ntsc_secondary_texture, 1, 1, &[0, 0, 0, 0]);
        }

        // vbo
        let vertex_data: [VertexData; 4] = [
            VertexData::new(Vec2D::new(-1.0, 1.0), Vec2D::new(0.0, 0.0)),
            VertexData::new(Vec2D::new(-1.0, -1.0), Vec2D::new(0.0, 1.0)),
            VertexData::new(Vec2D::new(1.0, 1.0), Vec2D::new(1.0, 0.0)),
            VertexData::new(Vec2D::new(1.0, -1.0), Vec2D::new(1.0, 1.0)),
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
            shader.get_uniform("unif_matrix"),
            1,
            gl::FALSE,
            Mat4::identity().as_ptr(),
        )
        .unwrap();
        uniform_1i(shader.get_uniform("frame_texture"), 0).unwrap();
        uniform_1i(shader.get_uniform("palette_texture"), 1).unwrap();
        if self.pipeline_mode.uses_packed_ntsc_lut() {
            uniform_1i(shader.get_uniform("ntsc_texture"), 2).unwrap();
        } else {
            uniform_1i(shader.get_uniform("ntsc_primary_texture"), 2).unwrap();
            uniform_1i(shader.get_uniform("ntsc_secondary_texture"), 3).unwrap();
        }
        uniform_1i(
            shader.get_uniform("source_width"),
            source_logical_size.width as i32,
        )
        .unwrap();
        uniform_1i(
            shader.get_uniform("source_height"),
            source_logical_size.height as i32,
        )
        .unwrap();
        uniform_1i(
            shader.get_uniform("output_width"),
            logical_size.width as i32,
        )
        .unwrap();
        uniform_1i(
            shader.get_uniform("output_height"),
            logical_size.height as i32,
        )
        .unwrap();
        uniform_1i(
            shader.get_uniform("filter_mode"),
            if matches!(filter_type, FilterType::None) {
                0
            } else {
                1
            },
        )
        .unwrap();
        uniform_2f(
            shader.get_uniform("frame_uv_size"),
            source_logical_size.width as f32 / frame_buffer_width as f32,
            source_logical_size.height as f32 / frame_buffer_height as f32,
        )
        .unwrap();

        // bind_buffer(gl::ARRAY_BUFFER, 0).unwrap();
        self.shader = Some(shader);
    }

    pub fn on_update(&self, screen_ptr: *const u8) {
        self.shader.as_ref().unwrap().use_program();
        active_texture(gl::TEXTURE0).unwrap();
        bind_texture(gl::TEXTURE_2D, self.frame_texture).unwrap();
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
            self.source_logical_size.width as i32,
            self.source_logical_size.height as i32,
            match self.single_channel_format {
                SingleChannelFormat::Red => gl::RED,
                SingleChannelFormat::Luminance => GL_LUMINANCE,
            },
            gl::UNSIGNED_BYTE,
            screen_ptr as *const c_void,
        )
        .unwrap();
        draw_arrays(gl::TRIANGLE_STRIP, 0, 4).unwrap();
    }

    pub fn on_resize(
        &mut self,
        scale_x: f32,
        scale_y: f32,
        viewport_width: i32,
        viewport_height: i32,
    ) {
        self.shader.as_ref().unwrap().use_program();
        if self.use_vao {
            self.vba.as_ref().unwrap().bind_vao(|_vac| Ok(())).unwrap();
        } else {
            bind_buffer(gl::ARRAY_BUFFER, self.vbo.as_ref().unwrap().id).unwrap();
        }
        viewport(0, 0, viewport_width, viewport_height).unwrap();
        uniform_matrix_4fv(
            self.shader.as_ref().unwrap().get_uniform("unif_matrix"),
            1,
            gl::FALSE,
            Mat4::scale(scale_x, scale_y, 1.0).as_ptr(),
        )
        .unwrap();
        // bind_buffer(gl::ARRAY_BUFFER, 0).unwrap();
    }

    pub fn on_close(&mut self) {
        let textures = [
            self.frame_texture,
            self.palette_texture,
            self.ntsc_primary_texture,
            self.ntsc_secondary_texture,
        ];
        delete_textures(textures.len() as i32, textures.as_ptr()).unwrap();
    }
}

impl Default for GlView {
    fn default() -> Self {
        Self::new()
    }
}

fn configure_palette_frame_texture(
    unit: u32,
    texture: u32,
    width: usize,
    height: usize,
    format: SingleChannelFormat,
) {
    active_texture(gl::TEXTURE0 + unit).unwrap();
    bind_texture(gl::TEXTURE_2D, texture).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32).unwrap();
    let (internal_format, upload_format) = match format {
        SingleChannelFormat::Red => (gl::R8 as GLint, gl::RED),
        SingleChannelFormat::Luminance => (GL_LUMINANCE as GLint, GL_LUMINANCE),
    };
    tex_image_2d(
        gl::TEXTURE_2D,
        0,
        internal_format,
        width as i32,
        height as i32,
        0,
        upload_format,
        gl::UNSIGNED_BYTE,
        allocate(width * height).as_ptr() as *const _,
    )
    .unwrap();
}

fn configure_rgba_texture(unit: u32, texture: u32, width: usize, height: usize, data: &[u8]) {
    active_texture(gl::TEXTURE0 + unit).unwrap();
    bind_texture(gl::TEXTURE_2D, texture).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32).unwrap();
    tex_image_2d(
        gl::TEXTURE_2D,
        0,
        gl::RGBA as GLint,
        width as i32,
        height as i32,
        0,
        gl::RGBA,
        gl::UNSIGNED_BYTE,
        data.as_ptr() as *const _,
    )
    .unwrap();
}

fn configure_rgba8ui_texture(unit: u32, texture: u32, width: usize, height: usize, data: &[u8]) {
    active_texture(gl::TEXTURE0 + unit).unwrap();
    bind_texture(gl::TEXTURE_2D, texture).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32).unwrap();
    tex_parameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32).unwrap();
    tex_image_2d(
        gl::TEXTURE_2D,
        0,
        gl::RGBA8UI as GLint,
        width as i32,
        height as i32,
        0,
        gl::RGBA_INTEGER,
        gl::UNSIGNED_BYTE,
        data.as_ptr() as *const _,
    )
    .unwrap();
}

fn compile_shader_program() -> (Shader, ShaderPipelineMode, SingleChannelFormat) {
    let context_version = gl_string(gl::VERSION);
    let shading_version = gl_string(gl::SHADING_LANGUAGE_VERSION);
    let is_gles = context_version
        .as_deref()
        .is_some_and(|version| version.contains("OpenGL ES"));

    log::info!(
        "initializing OpenGL renderer with context {:?} and shading language {:?}",
        context_version,
        shading_version
    );

    let candidates: &[(&str, &str, &str)] = if is_gles {
        &[(
            "gles2",
            include_str!("vertex.glsl"),
            include_str!("flagment.glsl"),
        )]
    } else {
        &[
            (
                "desktop-core",
                include_str!("vertex_desktop.glsl"),
                include_str!("fragment_desktop.glsl"),
            ),
            (
                "desktop-legacy",
                include_str!("vertex_legacy.glsl"),
                include_str!("fragment_legacy.glsl"),
            ),
        ]
    };

    let mut errors = Vec::new();
    for (name, vertex, fragment) in candidates {
        match Shader::try_new(vertex, fragment) {
            Ok(shader) => {
                log::info!("selected {name} shader pipeline");
                let (pipeline_mode, single_channel_format) = match *name {
                    "desktop-core" => (ShaderPipelineMode::DesktopCore, SingleChannelFormat::Red),
                    "desktop-legacy" => (
                        ShaderPipelineMode::DesktopLegacy,
                        SingleChannelFormat::Luminance,
                    ),
                    "gles2" => (ShaderPipelineMode::Gles2, SingleChannelFormat::Luminance),
                    _ => unreachable!(),
                };
                return (shader, pipeline_mode, single_channel_format);
            }
            Err(err) => errors.push(format!("{name}: {err}")),
        }
    }

    panic!(
        "failed to compile shader pipeline for context {:?} / {:?}: {}",
        context_version,
        shading_version,
        errors.join(" | ")
    );
}

fn gl_string(name: u32) -> Option<String> {
    let value = unsafe { gl::GetString(name) };
    if value.is_null() {
        return None;
    }

    Some(
        unsafe { CStr::from_ptr(value.cast()) }
            .to_string_lossy()
            .into_owned(),
    )
}
