use std::{os::raw::c_void, ptr, rc::Rc};

use gl::types::GLint;
use nerust_glwrap::{Shader, raw::*, vertex::*};
use nerust_render_ntsc::NTSC_TEXTURE_HEIGHT;
use nerust_render_traits::{VideoFrameFormat, VideoRenderProfile, renderer::OsdLayer};

use crate::{mat4::Mat4, vec2d::Vec2D, vertex_data::VertexData};

/// View failures: GL call errors, shader-pipeline errors, and
/// short-frame conditions. Programmer-invariant violations are a
/// separate variant so they stay greppable at the render boundary.
/// Converted to a warned `RenderResult::Error` at the render boundary;
/// OSD helpers degrade to a debug log instead.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ViewError {
    #[error(transparent)]
    Gl(#[from] nerust_glwrap::GlError),
    #[error("{0}")]
    Shader(String),
    #[error("internal error: {0}")]
    Internal(&'static str),
    #[error("{0}")]
    Frame(&'static str),
}

/// GL failures surface as view errors; init and per-frame game paths
/// `?` into this, and render() turns it into a warned `Error` outcome.
/// OSD helpers degrade to a debug log instead of killing the frame.
fn ok_gl<T>(result: Result<T, nerust_glwrap::GlError>) -> Result<T, ViewError> {
    result.map_err(ViewError::Gl)
}

/// Bind a texture unit in one step; the active/bind pair repeats at
/// every upload and draw site.
fn use_texture(unit: u32, texture: u32) -> Result<(), ViewError> {
    ok_gl(active_texture(gl::TEXTURE0 + unit))?;
    ok_gl(bind_texture(gl::TEXTURE_2D, texture))?;
    Ok(())
}

/// Point-sampled clamp addressing, shared by all upload textures.
fn set_nearest_params() -> Result<(), ViewError> {
    ok_gl(tex_parameteri(
        gl::TEXTURE_2D,
        gl::TEXTURE_WRAP_S,
        gl::CLAMP_TO_EDGE as i32,
    ))?;
    ok_gl(tex_parameteri(
        gl::TEXTURE_2D,
        gl::TEXTURE_WRAP_T,
        gl::CLAMP_TO_EDGE as i32,
    ))?;
    ok_gl(tex_parameteri(
        gl::TEXTURE_2D,
        gl::TEXTURE_MAG_FILTER,
        gl::NEAREST as i32,
    ))?;
    ok_gl(tex_parameteri(
        gl::TEXTURE_2D,
        gl::TEXTURE_MIN_FILTER,
        gl::NEAREST as i32,
    ))?;
    Ok(())
}

const DIRECT_FRAGMENT_DESKTOP: &str = r#"
uniform sampler2D frame_texture;
in vec2 vuv;
out vec4 frag_color;

void main(void) {
    frag_color = texture(frame_texture, vuv);
}
"#;

const DIRECT_FRAGMENT_COMPAT: &str = r#"
uniform sampler2D frame_texture;
varying NERUST_MEDIUMP vec2 vuv;

void main(void) {
    gl_FragColor = texture2D(frame_texture, vuv);
}
"#;

const PALETTE_FRAGMENT_DESKTOP: &str = include_str!("fragment_desktop_combined.glsl");

const OSD_FRAGMENT_DESKTOP: &str = r#"
uniform sampler2D osd_texture;
in vec2 vuv;
out vec4 frag_color;

void main(void) {
    frag_color = texture(osd_texture, vuv);
}
"#;

const OSD_FRAGMENT_COMPAT: &str = r#"
uniform sampler2D osd_texture;
varying NERUST_MEDIUMP vec2 vuv;

void main(void) {
    gl_FragColor = texture2D(osd_texture, vuv);
}
"#;

fn allocate(size: usize) -> Box<[u8]> {
    vec![0; size].into_boxed_slice()
}

#[derive(Debug)]
pub struct GlView {
    frame_texture: u32,
    palette_texture: u32,
    palette_width: i32,
    palette_height: i32,
    ntsc_texture: u32,
    is_palette_format: bool,
    ntsc_enabled: bool,
    shader: Option<Shader>,
    use_vao: bool,
    unpack_row_length: bool,
    osd_texture: u32,
    osd_size: (i32, i32),
    osd_prev: Option<(i32, i32, i32, i32)>,
    osd_dirty: bool,
    osd_shader: Option<Shader>,
    osd_layer: Option<OsdLayer>,
    vba: Option<VertexArray>,
    vbo: Option<Rc<VertexBuffer>>,
    logical_width: i32,
    logical_height: i32,
}

impl GlView {
    pub fn new() -> Self {
        Self {
            frame_texture: 0,
            palette_texture: 0,
            palette_width: 0,
            palette_height: 0,
            ntsc_texture: 0,
            is_palette_format: false,
            ntsc_enabled: false,
            shader: None,
            use_vao: false,
            unpack_row_length: false,
            osd_texture: 0,
            osd_size: (0, 0),
            osd_prev: None,
            osd_dirty: false,
            osd_shader: None,
            osd_layer: None,
            vba: None,
            vbo: None,
            logical_width: 0,
            logical_height: 0,
        }
    }

    pub fn use_vao(&mut self, value: bool) {
        self.use_vao = value;
    }

    pub fn load_with<F: FnMut(&'static str) -> *const c_void>(get_proc_address: F) {
        gl::load_with(get_proc_address);
    }

    pub fn on_load(&mut self, render_profile: &VideoRenderProfile) -> Result<(), ViewError> {
        self.is_palette_format = render_profile.frame_format == VideoFrameFormat::Palette;
        // Palette モードでは frame data は source_logical_size、RGBA では logical_size
        let frame_size = if self.is_palette_format {
            render_profile.source_logical_size
        } else {
            render_profile.logical_size
        };
        self.logical_width = frame_size.width as i32;
        self.logical_height = frame_size.height as i32;

        let bpp: usize = if self.is_palette_format { 1 } else { 4 };
        let version = gl_string(gl::VERSION);
        let is_gles = is_gles_context(version.as_deref());
        // Stride-padded rows upload as-is via UNPACK_ROW_LENGTH
        // (desktop GL, or GLES3+); older ES repacks instead.
        self.unpack_row_length = supports_unpack_row_length(version.as_deref());
        let shader = compile_shader_program(self.is_palette_format, is_gles)?;
        ok_gl(shader.use_program())?;
        ok_gl(clear_color(0.0, 0.0, 0.0, 1.0))?;

        ok_gl(pixel_storei(gl::UNPACK_ALIGNMENT, 1))?;

        // frame texture (palette 時は R8、RGBA 時は RGBA)
        let (internal_fmt, data_fmt) = if self.is_palette_format {
            (gl::R8 as GLint, gl::RED)
        } else {
            (gl::RGBA as GLint, gl::RGBA)
        };
        let mut tex_names = [0; 1];
        ok_gl(gen_textures(1, tex_names.as_mut_ptr()))?;
        self.frame_texture = tex_names[0];
        configure_frame_texture(
            0,
            self.frame_texture,
            frame_size.width,
            frame_size.height,
            internal_fmt,
            data_fmt,
            allocate(frame_size.width * frame_size.height * bpp).as_ref(),
        )?;

        // palette texture: 常に 64x1 RGBA8、ゼロ初期化。
        // 実データは render 時に FrameBuffer.palette_as_rgba8() から同期される。
        if self.is_palette_format {
            self.palette_width = 64;
            self.palette_height = 1;
            let palette_data =
                vec![0u8; self.palette_width as usize * self.palette_height as usize * 4];
            let mut pal_names = [0; 1];
            ok_gl(gen_textures(1, pal_names.as_mut_ptr()))?;
            self.palette_texture = pal_names[0];
            configure_frame_texture(
                1,
                self.palette_texture,
                self.palette_width as usize,
                self.palette_height as usize,
                gl::RGBA as GLint,
                gl::RGBA,
                &palette_data,
            )?;
            ok_gl(uniform_1i(shader.get_uniform("palette_texture"), 1))?;

            // NTSC texture
            let ntsc_size = render_profile.logical_size;
            if let Some(ntsc_data) = render_profile.ntsc_packed_rgba8.as_deref() {
                self.ntsc_enabled = true;
                let mut ntsc_names = [0; 1];
                ok_gl(gen_textures(1, ntsc_names.as_mut_ptr()))?;
                self.ntsc_texture = ntsc_names[0];
                configure_ntsc_texture(
                    2,
                    self.ntsc_texture,
                    64,
                    NTSC_TEXTURE_HEIGHT as usize,
                    ntsc_data,
                )?;
                ok_gl(uniform_1i(shader.get_uniform("ntsc_texture"), 2))?;
            } else {
                // ダミー (sampler2D 未バインド防止)
                let mut ntsc_names = [0; 1];
                ok_gl(gen_textures(1, ntsc_names.as_mut_ptr()))?;
                self.ntsc_texture = ntsc_names[0];
                let ntsc_height = NTSC_TEXTURE_HEIGHT as usize;
                let dummy = vec![0u8; 64 * ntsc_height * 4];
                configure_frame_texture(
                    2,
                    self.ntsc_texture,
                    64,
                    ntsc_height,
                    gl::RGBA as GLint,
                    gl::RGBA,
                    &dummy,
                )?;
                ok_gl(uniform_1i(shader.get_uniform("ntsc_texture"), 2))?;
            }

            ok_gl(uniform_2f(
                shader.get_uniform("source_size"),
                render_profile.source_logical_size.width as f32,
                render_profile.source_logical_size.height as f32,
            ))?;
            ok_gl(uniform_2f(
                shader.get_uniform("output_size"),
                ntsc_size.width as f32,
                ntsc_size.height as f32,
            ))?;
            ok_gl(uniform_1i(
                shader.get_uniform("ntsc_enabled"),
                self.ntsc_enabled as i32,
            ))?;
        }

        let vertex_data: [VertexData; 4] = [
            VertexData::new(Vec2D::new(-1.0, 1.0), Vec2D::new(0.0, 0.0)),
            VertexData::new(Vec2D::new(-1.0, -1.0), Vec2D::new(0.0, 1.0)),
            VertexData::new(Vec2D::new(1.0, 1.0), Vec2D::new(1.0, 0.0)),
            VertexData::new(Vec2D::new(1.0, -1.0), Vec2D::new(1.0, 1.0)),
        ];

        let vbo = Rc::new(ok_gl(VertexBuffer::from_slice(&vertex_data))?);
        if self.use_vao {
            let vbo = vbo.clone();
            self.vba = Some(ok_gl(VertexArray::new(|vaic| {
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
            }))?);
        } else {
            ok_gl(vertex_attrib_pointer(
                shader.get_attribute("position"),
                2,
                gl::FLOAT,
                gl::FALSE,
                16,
                ptr::null(),
            ))?;
            ok_gl(enable_vertex_attrib_array(shader.get_attribute("position")))?;
            ok_gl(vertex_attrib_pointer(
                shader.get_attribute("uv"),
                2,
                gl::FLOAT,
                gl::FALSE,
                16,
                8 as *const c_void,
            ))?;
            ok_gl(enable_vertex_attrib_array(shader.get_attribute("uv")))?;
            self.vbo = Some(vbo.clone());
        }

        ok_gl(uniform_matrix_4fv(
            shader.get_uniform("unif_matrix"),
            1,
            gl::FALSE,
            Mat4::identity().as_ptr(),
        ))?;
        ok_gl(uniform_1i(shader.get_uniform("frame_texture"), 0))?;
        self.shader = Some(shader);
        self.init_osd(is_gles);
        Ok(())
    }

    /// Best-effort overlay program plus a placeholder texture.
    /// Unlike the game pipeline, failure only drops the OSD.
    fn init_osd(&mut self, is_gles: bool) {
        let mut names = [0u32; 1];
        if gen_textures(1, names.as_mut_ptr()).is_err() {
            return;
        }
        self.osd_texture = names[0];
        if configure_frame_texture(
            0,
            self.osd_texture,
            1,
            1,
            gl::RGBA as i32,
            gl::RGBA,
            &[0, 0, 0, 0],
        )
        .is_err()
        {
            log::debug!("GL view OSD placeholder upload failed; overlay disabled");
            return;
        }
        self.osd_size = (1, 1);
        let candidates: Vec<(&str, String, String)> = if is_gles {
            vec![
                (
                    "osd-gles3",
                    build_glsl_source(
                        include_str!("vertex_desktop.glsl"),
                        Some("#version 300 es"),
                        &["precision mediump float;"],
                    ),
                    compose_glsl_source(
                        "#version 300 es",
                        &["precision mediump float;"],
                        &[OSD_FRAGMENT_DESKTOP],
                    ),
                ),
                (
                    "osd-gles2",
                    include_str!("vertex.glsl").to_owned(),
                    compose_glsl_source(
                        "#version 100",
                        &["#define NERUST_MEDIUMP mediump"],
                        &[OSD_FRAGMENT_COMPAT],
                    ),
                ),
            ]
        } else {
            vec![(
                "osd-desktop",
                include_str!("vertex_desktop.glsl").to_owned(),
                compose_glsl_source("#version 150", &[], &[OSD_FRAGMENT_DESKTOP]),
            )]
        };
        for (name, vertex, fragment) in candidates {
            match Shader::try_new(vertex.as_str(), fragment.as_str()) {
                Ok(shader) => {
                    log::info!("selected {name} osd pipeline");
                    self.osd_shader = Some(shader);
                    return;
                }
                Err(err) => log::warn!("osd shader {name} unavailable: {err}"),
            }
        }
    }

    /// Stage an OSD layer for the next present. `None` clears.
    pub fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        self.osd_layer = layer;
        self.osd_dirty = true;
    }

    /// Composite the staged layer over the game image, if any.
    /// Runs inside the current frame (context already current).
    /// Best-effort: a GL failure drops the overlay, never the frame.
    pub fn draw_osd(&mut self, surface_w: i32, surface_h: i32) -> Result<(), ViewError> {
        if std::mem::replace(&mut self.osd_dirty, false) {
            self.sync_osd_texture(surface_w, surface_h)?;
        }
        let (Some(shader), Some(_)) = (self.osd_shader.as_ref(), self.osd_layer.as_ref()) else {
            return Ok(());
        };
        ok_gl(shader.use_program())?;
        ok_gl(uniform_matrix_4fv(
            shader.get_uniform("unif_matrix"),
            1,
            gl::FALSE,
            Mat4::identity().as_ptr(),
        ))?;
        ok_gl(uniform_1i(shader.get_uniform("osd_texture"), 0))?;
        use_texture(0, self.osd_texture)?;
        if self.use_vao {
            let Some(vba) = self.vba.as_ref() else {
                return Err(ViewError::Internal("GL view has no vertex array"));
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.vbo.as_ref() else {
                return Err(ViewError::Internal("GL view has no vertex buffer"));
            };
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, vbo.id))?;
        }
        ok_gl(viewport(0, 0, surface_w, surface_h))?;
        ok_gl(enable(gl::BLEND))?;
        ok_gl(blend_func(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA))?;
        ok_gl(draw_arrays(gl::TRIANGLE_STRIP, 0, 4))?;
        ok_gl(disable(gl::BLEND))?;
        Ok(())
    }

    /// Upload the staged layer into the full-surface texture.
    /// Runs only when the layer changed; the draw path only binds.
    /// Best-effort: failures keep the previous texture, never panic.
    fn sync_osd_texture(&mut self, surface_w: i32, surface_h: i32) -> Result<(), ViewError> {
        let layer = self.osd_layer.clone();
        // Layer cleared: wipe the last rect so no ghost remains.
        let Some(layer) = layer else {
            if let Some((px, py, pw, ph)) = self.osd_prev.take() {
                Self::blit_rect(self.osd_texture, px, py, pw, ph, None)?;
            }
            return Ok(());
        };
        if self.osd_texture == 0 {
            return Ok(());
        }
        // Full-surface texture: placement rides on texture coordinates
        // instead of small viewports (driver-quirky on some backends).
        if self.osd_size != (surface_w, surface_h) {
            use_texture(0, self.osd_texture)?;
            let zeros = vec![0u8; surface_w.max(1) as usize * surface_h.max(1) as usize * 4];
            ok_gl(tex_image_2d(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                surface_w,
                surface_h,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                zeros.as_ptr().cast(),
            ))?;
            self.osd_size = (surface_w, surface_h);
        }
        if layer.width == 0 || layer.height == 0 {
            return Ok(());
        }
        let (w, h) = (layer.width as i32, layer.height as i32);
        // Clear the previous rect so moved/shrunk layers leave no ghosts.
        let rect = (layer.x as i32, layer.y as i32, w, h);
        if self.osd_prev != Some(rect) {
            if let Some((px, py, pw, ph)) = self.osd_prev {
                Self::blit_rect(self.osd_texture, px, py, pw, ph, None)?;
            }
            self.osd_prev = Some(rect);
        }
        let stride = w as usize * 4;
        let mut padded = vec![0u8; stride * h as usize];
        for (index, row) in layer.rgba.chunks_exact(stride).take(h as usize).enumerate() {
            padded[index * stride..index * stride + stride].copy_from_slice(row);
        }
        Self::blit_rect(
            self.osd_texture,
            layer.x as i32,
            layer.y as i32,
            w,
            h,
            Some(&padded),
        )?;
        Ok(())
    }

    /// Upload RGBA rows (or zeros) into a texture subrect.
    /// Best-effort like the rest of the OSD path: the caller degrades
    /// to a debug log instead of killing the frame.
    fn blit_rect(
        texture: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        rgba: Option<&[u8]>,
    ) -> Result<(), ViewError> {
        let blank;
        let bytes = match rgba {
            Some(bytes) => bytes,
            None => {
                blank = vec![0u8; w.max(1) as usize * h.max(1) as usize * 4];
                &blank
            }
        };
        use_texture(0, texture)?;
        ok_gl(tex_sub_image_2d(
            gl::TEXTURE_2D,
            0,
            x,
            y,
            w,
            h,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            bytes.as_ptr().cast(),
        ))?;
        Ok(())
    }

    pub fn update_palette_texture(&self, rgba8: &[u8; 256]) -> Result<(), ViewError> {
        if !self.is_palette_format {
            return Ok(());
        }
        use_texture(1, self.palette_texture)?;
        ok_gl(tex_sub_image_2d(
            gl::TEXTURE_2D,
            0,
            0,
            0,
            self.palette_width,
            self.palette_height,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            rgba8.as_ptr().cast(),
        ))?;
        ok_gl(active_texture(gl::TEXTURE0))?;
        Ok(())
    }

    pub fn on_update(&self, screen: &[u8], row_length: i32) -> Result<(), ViewError> {
        let screen_ptr = screen.as_ptr().cast();
        let Some(shader) = self.shader.as_ref() else {
            return Err(ViewError::Internal("GL view has no shader program"));
        };
        ok_gl(shader.use_program())?;

        use_texture(0, self.frame_texture)?;

        // Padded rows upload as-is; row length is global pixel-store
        // state, so restore the default right after the upload.
        let padded = row_length != self.logical_width;
        if padded {
            ok_gl(pixel_storei(gl::UNPACK_ROW_LENGTH, row_length))?;
        }

        if self.is_palette_format {
            use_texture(1, self.palette_texture)?;
            use_texture(2, self.ntsc_texture)?;
            // frame texture (palette index を R8 → GL_RED で upload)
            use_texture(0, self.frame_texture)?;
            ok_gl(tex_sub_image_2d(
                gl::TEXTURE_2D,
                0,
                0,
                0,
                self.logical_width,
                self.logical_height,
                gl::RED,
                gl::UNSIGNED_BYTE,
                screen_ptr,
            ))?;
        }

        if self.use_vao {
            let Some(vba) = self.vba.as_ref() else {
                return Err(ViewError::Internal("GL view has no vertex array"));
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.vbo.as_ref() else {
                return Err(ViewError::Internal("GL view has no vertex buffer"));
            };
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, vbo.id))?;
        }

        ok_gl(clear(gl::COLOR_BUFFER_BIT))?;

        if !self.is_palette_format {
            ok_gl(tex_sub_image_2d(
                gl::TEXTURE_2D,
                0,
                0,
                0,
                self.logical_width,
                self.logical_height,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                screen_ptr,
            ))?;
        }
        if padded {
            ok_gl(pixel_storei(gl::UNPACK_ROW_LENGTH, 0))?;
        }
        ok_gl(draw_arrays(gl::TRIANGLE_STRIP, 0, 4))?;
        Ok(())
    }

    /// Whether the context accepts UNPACK_ROW_LENGTH (no CPU repack).
    pub fn supports_unpack_row_length(&self) -> bool {
        self.unpack_row_length
    }

    /// Fit the game into the surface minus reserved OSD strips.
    /// GL viewports start at the bottom-left, so the drawable rect
    /// sits `strip.bottom` above the surface base.
    pub fn on_resize_with_strip(
        &mut self,
        viewport_width: i32,
        viewport_height: i32,
        strip: nerust_render_traits::ReservedStrip,
    ) -> Result<(), ViewError> {
        let height_i = viewport_height.max(0);
        let top = strip.top.min(height_i as u32) as i32;
        let bottom = strip.bottom.min(height_i as u32 - top as u32) as i32;
        let area_height = (height_i - top - bottom).max(1);
        let window_aspect = viewport_width as f32 / area_height as f32;
        let content_aspect = self.logical_width as f32 / self.logical_height as f32;

        let (scale_x, scale_y) = if window_aspect > content_aspect {
            // Window is wider than content → letterbox (black bars on sides)
            (content_aspect / window_aspect, 1.0)
        } else {
            // Window is taller than content → pillarbox (black bars on top/bottom)
            (1.0, window_aspect / content_aspect)
        };

        let Some(shader) = self.shader.as_ref() else {
            return Err(ViewError::Internal("GL view has no shader program"));
        };
        ok_gl(shader.use_program())?;
        if self.use_vao {
            let Some(vba) = self.vba.as_ref() else {
                return Err(ViewError::Internal("GL view has no vertex array"));
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.vbo.as_ref() else {
                return Err(ViewError::Internal("GL view has no vertex buffer"));
            };
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, vbo.id))?;
        }
        ok_gl(viewport(0, bottom, viewport_width, area_height))?;
        ok_gl(uniform_matrix_4fv(
            shader.get_uniform("unif_matrix"),
            1,
            gl::FALSE,
            Mat4::scale(scale_x, scale_y, 1.0).as_ptr(),
        ))?;
        Ok(())
    }

    pub fn on_close(&mut self) {
        let mut ids = Vec::new();
        if self.frame_texture != 0 {
            ids.push(self.frame_texture);
        }
        if self.palette_texture != 0 {
            ids.push(self.palette_texture);
        }
        if self.ntsc_texture != 0 {
            ids.push(self.ntsc_texture);
        }
        // Teardown best-effort: a failed delete must not panic the
        // profile rebuild that called here.
        if !ids.is_empty()
            && let Err(e) = delete_textures(ids.len() as i32, ids.as_ptr())
        {
            log::warn!("GL view teardown failed: {e}");
        }
    }
}

impl Default for GlView {
    fn default() -> Self {
        Self::new()
    }
}

/// NTSC kernel texture (RGBA8)。packed_ntsc_rgba8 は big-endian u32 を RGBA8 に変換。
/// 実際の kernel entry stride を data 長から計算して texture 高さとする。
/// (固定 NTSC_TEXTURE_HEIGHT=42 では不足。ntsc_entry が phase_row+offset > 41 を読むため)
fn configure_ntsc_texture(
    unit: u32,
    texture: u32,
    width: usize,
    height: usize,
    be_data: &[u8],
) -> Result<(), ViewError> {
    // be_data は PALETTE_TEXTURE_WIDTH × entry_stride × 4 bytes
    let entry_count = be_data.len() / (width * 4);
    let actual_height = entry_count.max(height);
    let mut rgba = Vec::with_capacity(width * actual_height * 4);
    for row in 0..actual_height {
        for col in 0..width {
            let base = (row * width + col) * 4;
            if base + 4 <= be_data.len() {
                rgba.extend_from_slice(&be_data[base..base + 4]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    use_texture(unit, texture)?;
    set_nearest_params()?;
    ok_gl(tex_image_2d(
        gl::TEXTURE_2D,
        0,
        gl::RGBA as GLint,
        width as i32,
        actual_height as i32,
        0,
        gl::RGBA,
        gl::UNSIGNED_BYTE,
        rgba.as_ptr().cast(),
    ))?;
    Ok(())
}

fn configure_frame_texture(
    unit: u32,
    texture: u32,
    width: usize,
    height: usize,
    internal_fmt: gl::types::GLint,
    data_fmt: gl::types::GLenum,
    data: &[u8],
) -> Result<(), ViewError> {
    use_texture(unit, texture)?;
    set_nearest_params()?;
    ok_gl(tex_image_2d(
        gl::TEXTURE_2D,
        0,
        internal_fmt,
        width as i32,
        height as i32,
        0,
        data_fmt,
        gl::UNSIGNED_BYTE,
        data.as_ptr().cast(),
    ))?;
    Ok(())
}

fn build_glsl_source(
    source: &str,
    version_override: Option<&str>,
    extra_preamble: &[&str],
) -> String {
    let (version_line, body) = source
        .split_once('\n')
        .expect("GLSL source must start with a version line");
    compose_glsl_source(
        version_override.unwrap_or(version_line),
        extra_preamble,
        &[body],
    )
}

fn compose_glsl_source(version_line: &str, extra_preamble: &[&str], parts: &[&str]) -> String {
    let mut output = String::new();
    output.push_str(version_line);
    output.push('\n');
    for line in extra_preamble {
        output.push_str(line);
        output.push('\n');
    }
    for part in parts {
        output.push_str(part);
        if !part.ends_with('\n') {
            output.push('\n');
        }
    }
    output
}

fn compile_shader_program(is_palette: bool, is_gles: bool) -> Result<Shader, ViewError> {
    let context_version = gl_string(gl::VERSION);
    let shading_version = gl_string(gl::SHADING_LANGUAGE_VERSION);

    log::info!(
        "initializing OpenGL renderer with context {:?} and shading language {:?} (palette={is_palette})",
        context_version,
        shading_version,
    );

    let fragment_desktop = if is_palette {
        PALETTE_FRAGMENT_DESKTOP
    } else {
        DIRECT_FRAGMENT_DESKTOP
    };
    let fragment_compat = DIRECT_FRAGMENT_COMPAT; // no palette fallback for legacy

    let candidates: Vec<(&str, String, String)> = if is_gles {
        vec![
            (
                "gles3",
                build_glsl_source(
                    include_str!("vertex_desktop.glsl"),
                    Some("#version 300 es"),
                    &["precision mediump float;"],
                ),
                compose_glsl_source(
                    "#version 300 es",
                    &["precision mediump float;"],
                    &[fragment_desktop],
                ),
            ),
            (
                "gles2",
                include_str!("vertex.glsl").to_owned(),
                compose_glsl_source(
                    "#version 100",
                    &["#define NERUST_MEDIUMP mediump"],
                    &[fragment_compat],
                ),
            ),
        ]
    } else {
        let mut desktop = vec![(
            "desktop-core",
            include_str!("vertex_desktop.glsl").to_owned(),
            compose_glsl_source("#version 150", &[], &[fragment_desktop]),
        )];
        if !is_palette {
            desktop.push((
                "desktop-legacy",
                include_str!("vertex_legacy.glsl").to_owned(),
                compose_glsl_source(
                    "#version 120",
                    &["#define NERUST_MEDIUMP"],
                    &[fragment_compat],
                ),
            ));
        }
        desktop
    };

    let mut errors = Vec::new();
    for (name, vertex, fragment) in candidates {
        match Shader::try_new(vertex.as_str(), fragment.as_str()) {
            Ok(shader) => {
                log::info!("selected {name} shader pipeline");
                return Ok(shader);
            }
            Err(err) => errors.push(format!("{name}: {err}")),
        }
    }

    Err(ViewError::Shader(format!(
        "failed to compile shader pipeline for context {context_version:?} / {shading_version:?}: {}",
        errors.join(" | ")
    )))
}

fn is_gles_context(version: Option<&str>) -> bool {
    version.is_some_and(|value| value.contains("OpenGL ES"))
}

/// UNPACK_ROW_LENGTH is core on desktop GL and arrived in GLES3;
/// older ES has no row stride control, so frames repack on CPU.
fn supports_unpack_row_length(version: Option<&str>) -> bool {
    let Some(value) = version else {
        return false;
    };
    if !value.contains("OpenGL ES") {
        return true;
    }
    value.contains("OpenGL ES 3")
}

#[cfg(test)]
mod tests {
    use super::{
        build_glsl_source, compose_glsl_source, is_gles_context, supports_unpack_row_length,
    };

    #[test]
    fn detects_gles_context_strings() {
        assert!(is_gles_context(Some("OpenGL ES 3.2 Mesa 24.1.0")));
        assert!(!is_gles_context(Some("4.6 (Core Profile) Mesa 24.1.0")));
        assert!(!is_gles_context(None));
    }

    #[test]
    fn detects_unpack_row_length_support() {
        assert!(supports_unpack_row_length(Some(
            "4.5 (Core Profile) Mesa 26.0.8"
        )));
        assert!(supports_unpack_row_length(Some(
            "OpenGL ES 3.2 Mesa 24.1.0"
        )));
        assert!(!supports_unpack_row_length(Some(
            "OpenGL ES 2.0 Mesa 24.1.0"
        )));
        assert!(!supports_unpack_row_length(None));
    }

    #[test]
    fn inserts_extra_preamble_after_version_line() {
        let source = build_glsl_source(
            "#version 120\nvoid main(void) {}\n",
            None,
            &["#define TEST 1"],
        );
        assert_eq!(source, "#version 120\n#define TEST 1\nvoid main(void) {}\n");
    }

    #[test]
    fn composes_glsl_parts_in_order() {
        let source = compose_glsl_source(
            "#version 120",
            &["#define NERUST_MEDIUMP"],
            &["void helper(void) {}\n", "void main(void) {}\n"],
        );

        assert_eq!(
            source,
            "#version 120\n#define NERUST_MEDIUMP\nvoid helper(void) {}\nvoid main(void) {}\n"
        );
    }
}
