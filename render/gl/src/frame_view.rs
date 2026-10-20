//! Frame pipeline view: textures, program, and geometry.
//! Owns the game-image domain; OSD and surface state live elsewhere.

use std::{os::raw::c_void, ptr, rc::Rc};

use gl::types::GLint;
use nerust_glwrap::{
    CurrentContext, DeletionPolicy, ScopeGuard, Shader,
    raw::{
        active_texture, bind_buffer, check_frame, clear, clear_color, delete_textures, draw_arrays,
        enable_vertex_attrib_array, gen_textures, get_integer, gl_string, pixel_storei,
        tex_sub_image_2d, uniform_1i, uniform_2f, uniform_matrix_4fv, vertex_attrib_pointer,
        viewport,
    },
    vertex::{Attrib, VertexArray, VertexBuffer},
};
use nerust_render_ntsc::NTSC_TEXTURE_HEIGHT;
use nerust_render_traits::{ReservedStrip, VideoFrameFormat, VideoRenderProfile};

use crate::mat4::Mat4;
use crate::shader_compiler::{compile_shader_program, is_gles_context, supports_unpack_row_length};
use crate::surface_view::LeakedName;
use crate::upload::StagingBuf;
use crate::vec2d::Vec2D;
use crate::vertex_data::VertexData;
use crate::{
    error::ViewError,
    frame_textures::{
        allocate_texture_store, configure_ntsc_texture, upload_texture_bytes, use_texture,
    },
};

/// Convert a GL call result into a view error for `?` propagation.
pub(crate) fn ok_gl<T>(result: Result<T, nerust_glwrap::GlError>) -> Result<T, ViewError> {
    result.map_err(ViewError::Gl)
}

/// Cached uniform locations for the game pipeline: resolved once at `on_load`.
/// `last_matrix` skips redundant matrix uploads.
#[derive(Debug, Default)]
pub(crate) struct GameUniforms {
    matrix: Option<GLint>,
    frame_texture: Option<GLint>,
    palette_texture: Option<GLint>,
    ntsc_texture: Option<GLint>,
    source_size: Option<GLint>,
    output_size: Option<GLint>,
    ntsc_enabled: Option<GLint>,
}

/// Frame pipeline resources: textures, program, geometry. One owner for
/// the game-image domain; OSD and surface state live elsewhere.
#[derive(Debug, Default)]
pub(crate) struct FrameView {
    pub(crate) frame_texture: u32,
    pub(crate) palette_texture: u32,
    pub(crate) palette_width: i32,
    pub(crate) palette_height: i32,
    pub(crate) ntsc_texture: u32,
    pub(crate) is_palette_format: bool,
    pub(crate) ntsc_enabled: bool,
    pub(crate) shader: Option<Shader>,
    pub(crate) use_vao: bool,
    pub(crate) unpack_row_length: bool,
    pub(crate) vba: Option<VertexArray>,
    pub(crate) vbo: Option<Rc<VertexBuffer>>,
    pub(crate) logical_width: i32,
    pub(crate) logical_height: i32,
    /// Separate staging for the NTSC kernel upload (load-time only).
    pub(crate) ntsc_scratch: StagingBuf,
    pub(crate) uniforms: GameUniforms,
    pub(crate) last_matrix: Option<[f32; 16]>,
    pub(crate) current_program: Option<u32>,
}

/// Required uniform location. Structurally-needed names fail typed instead of `-1`.
/// Samplers GL may legally drop use `uniform_opt` directly and skip.
fn required_uniform(
    shader: &nerust_glwrap::Shader,
    program: &'static str,
    key: &'static str,
) -> Result<i32, ViewError> {
    shader
        .uniform_opt(key)
        .ok_or(ViewError::MissingUniform { name: key, program })
}

fn required_attrib(
    shader: &nerust_glwrap::Shader,
    program: &'static str,
    key: &'static str,
) -> Result<u32, ViewError> {
    shader
        .attribute_opt(key)
        .ok_or(ViewError::MissingAttribute { name: key, program })
}

impl FrameView {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Toggle VAO usage (test seam; production always enables).
    pub(crate) fn set_use_vao(&mut self, value: bool) {
        self.use_vao = value;
    }

    /// Load GL symbols on this thread (facade keeps the `GlView::` spelling).
    pub(crate) fn load_with<F: FnMut(&'static str) -> *const c_void>(get_proc_address: F) {
        gl::load_with(get_proc_address);
    }

    /// Use the game program, skipping the GL call when already current.
    fn use_game_program(&mut self) -> Result<(), ViewError> {
        let Some(shader) = self.shader.as_ref() else {
            return Err(ViewError::NoProgram { program: "game" });
        };
        let id = shader.id();
        if self.current_program != Some(id) {
            ok_gl(shader.use_program())?;
            self.current_program = Some(id);
        }
        Ok(())
    }

    /// Upload the game matrix only when it changed since last upload.
    fn set_game_matrix(&mut self, mat: &[f32; 16]) -> Result<(), ViewError> {
        if self.last_matrix == Some(*mat) {
            return Ok(());
        }
        let Some(loc) = self.uniforms.matrix else {
            return Err(ViewError::NoProgram { program: "game" });
        };
        ok_gl(uniform_matrix_4fv(loc, 1, gl::FALSE, mat.as_ptr()))?;
        self.last_matrix = Some(*mat);
        Ok(())
    }

    /// Frame upload target plus the placeholder palette texture.
    /// Store allocates null data; rows arrive via `TexSubImage`.
    fn load_frame_textures(
        &mut self,
        render_profile: &VideoRenderProfile,
    ) -> Result<(), ViewError> {
        let frame_size = if self.is_palette_format {
            render_profile.source_logical_size
        } else {
            render_profile.logical_size
        };
        self.logical_width = i32::try_from(frame_size.width).map_err(|_| {
            crate::layout::FrameError::StrideTooWide {
                have: frame_size.width,
            }
        })?;
        self.logical_height = i32::try_from(frame_size.height).map_err(|_| {
            crate::layout::FrameError::StrideTooWide {
                have: frame_size.height,
            }
        })?;
        let (internal_fmt, data_fmt) = if self.is_palette_format {
            (gl::R8 as GLint, gl::RED)
        } else {
            (gl::RGBA as GLint, gl::RGBA)
        };
        let mut tex_names = [0; 1];
        ok_gl(gen_textures(1, tex_names.as_mut_ptr()))?;
        self.frame_texture = tex_names[0];
        allocate_texture_store(
            0,
            self.frame_texture,
            frame_size.width,
            frame_size.height,
            internal_fmt,
            data_fmt,
        )?;

        if self.is_palette_format {
            self.palette_width = 64;
            self.palette_height = 1;
            let mut pal_names = [0; 1];
            ok_gl(gen_textures(1, pal_names.as_mut_ptr()))?;
            self.palette_texture = pal_names[0];
            // 64x1 RGBA8 placeholder, zeroed via a small stack upload;
            // real data syncs from `FrameBuffer::palette_as_rgba8`.
            upload_texture_bytes(
                1,
                self.palette_texture,
                64,
                1,
                gl::RGBA as GLint,
                gl::RGBA,
                &[0u8; 64 * 4],
            )?;
        }
        Ok(())
    }

    /// Palette sampler binding plus the NTSC kernel texture and its
    /// uniforms. Locations are cached for the per-frame path.
    fn load_palette_ntsc(&mut self, render_profile: &VideoRenderProfile) -> Result<(), ViewError> {
        let palette_loc = {
            let Some(shader) = self.shader.as_ref() else {
                return Err(ViewError::NoProgram { program: "game" });
            };
            required_uniform(shader, "game", "palette_texture")?
        };
        self.uniforms.palette_texture = Some(palette_loc);
        ok_gl(uniform_1i(palette_loc, 1))?;

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
                &mut self.ntsc_scratch,
            )?;
            let loc = {
                let Some(shader) = self.shader.as_ref() else {
                    return Err(ViewError::NoProgram { program: "game" });
                };
                required_uniform(shader, "game", "ntsc_texture")?
            };
            self.uniforms.ntsc_texture = Some(loc);
            ok_gl(uniform_1i(loc, 2))?;
        } else {
            let mut ntsc_names = [0; 1];
            ok_gl(gen_textures(1, ntsc_names.as_mut_ptr()))?;
            self.ntsc_texture = ntsc_names[0];
            let ntsc_height = NTSC_TEXTURE_HEIGHT as usize;
            allocate_texture_store(
                2,
                self.ntsc_texture,
                64,
                ntsc_height,
                gl::RGBA as GLint,
                gl::RGBA,
            )?;
            let loc = {
                let Some(shader) = self.shader.as_ref() else {
                    return Err(ViewError::NoProgram { program: "game" });
                };
                required_uniform(shader, "game", "ntsc_texture")?
            };
            self.uniforms.ntsc_texture = Some(loc);
            ok_gl(uniform_1i(loc, 2))?;
        }

        let (source_loc, output_loc, enabled_loc) = {
            let Some(shader) = self.shader.as_ref() else {
                return Err(ViewError::NoProgram { program: "game" });
            };
            (
                required_uniform(shader, "game", "source_size")?,
                required_uniform(shader, "game", "output_size")?,
                required_uniform(shader, "game", "ntsc_enabled")?,
            )
        };
        self.uniforms.source_size = Some(source_loc);
        self.uniforms.output_size = Some(output_loc);
        self.uniforms.ntsc_enabled = Some(enabled_loc);
        ok_gl(uniform_2f(
            source_loc,
            render_profile.source_logical_size.width as f32,
            render_profile.source_logical_size.height as f32,
        ))?;
        ok_gl(uniform_2f(
            output_loc,
            ntsc_size.width as f32,
            ntsc_size.height as f32,
        ))?;
        ok_gl(uniform_1i(enabled_loc, self.ntsc_enabled as i32))?;
        Ok(())
    }

    /// Quad geometry plus the game-pipeline uniform cache.
    fn init_geometry(&mut self) -> Result<(), ViewError> {
        let vertex_data: [VertexData; 4] = [
            VertexData::new(Vec2D::new(-1.0, 1.0), Vec2D::new(0.0, 0.0)),
            VertexData::new(Vec2D::new(-1.0, -1.0), Vec2D::new(0.0, 1.0)),
            VertexData::new(Vec2D::new(1.0, 1.0), Vec2D::new(1.0, 0.0)),
            VertexData::new(Vec2D::new(1.0, -1.0), Vec2D::new(1.0, 1.0)),
        ];

        let vbo = Rc::new(ok_gl(VertexBuffer::from_slice(&vertex_data))?);
        let (position, uv) = {
            let Some(shader) = self.shader.as_ref() else {
                return Err(ViewError::NoProgram { program: "game" });
            };
            (
                required_attrib(shader, "game", "position")?,
                required_attrib(shader, "game", "uv")?,
            )
        };
        if self.use_vao {
            let vbo = vbo.clone();
            self.vba = Some(ok_gl(VertexArray::new(|vaic| {
                vaic.bind_vbo(vbo, |vac| {
                    vac.attr_pointer(Attrib { id: position }, 2, gl::FLOAT, 16, 0)?;
                    vac.attr_pointer(Attrib { id: uv }, 2, gl::FLOAT, 16, 8)
                })
            }))?);
        } else {
            ok_gl(vertex_attrib_pointer(
                position,
                2,
                gl::FLOAT,
                gl::FALSE,
                16,
                ptr::null(),
            ))?;
            ok_gl(enable_vertex_attrib_array(position))?;
            ok_gl(vertex_attrib_pointer(
                uv,
                2,
                gl::FLOAT,
                gl::FALSE,
                16,
                8 as *const c_void,
            ))?;
            ok_gl(enable_vertex_attrib_array(uv))?;
            self.vbo = Some(vbo.clone());
        }

        let (matrix_loc, frame_loc) = {
            let Some(shader) = self.shader.as_ref() else {
                return Err(ViewError::NoProgram { program: "game" });
            };
            (
                required_uniform(shader, "game", "unif_matrix")?,
                required_uniform(shader, "game", "frame_texture")?,
            )
        };
        self.uniforms.matrix = Some(matrix_loc);
        self.uniforms.frame_texture = Some(frame_loc);
        let identity = Mat4::identity();
        let mat: [f32; 16] = identity.as_array();
        ok_gl(uniform_matrix_4fv(
            matrix_loc,
            1,
            gl::FALSE,
            identity.as_ptr(),
        ))?;
        self.last_matrix = Some(mat);
        ok_gl(uniform_1i(frame_loc, 0))?;
        Ok(())
    }

    pub(crate) fn update_palette_texture(&self, rgba8: &[u8; 256]) -> Result<(), ViewError> {
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
        // Explicit restore: unit 0 + frame texture, so the game upload
        // below never samples the palette on unit 1.
        if self.frame_texture != 0 {
            use_texture(0, self.frame_texture)?;
        } else {
            ok_gl(active_texture(gl::TEXTURE0))?;
        }
        Ok(())
    }

    pub(crate) fn on_update(&mut self, screen: &[u8], row_length: i32) -> Result<(), ViewError> {
        let screen_ptr = screen.as_ptr().cast();
        self.use_game_program()?;

        use_texture(0, self.frame_texture)?;

        // Row-length override under a scope guard: construction failure
        // leaves state untouched, and the drop restores best-effort.
        let saved_state = if row_length != self.logical_width {
            let alignment = ok_gl(get_integer(gl::UNPACK_ALIGNMENT))?;
            debug_assert_eq!(
                alignment, 1,
                "FrameView owns UNPACK_ALIGNMENT=1; foreign value would mis-stride uploads",
            );
            let saved = ok_gl(get_integer(gl::UNPACK_ROW_LENGTH))?;
            ok_gl(pixel_storei(gl::UNPACK_ROW_LENGTH, row_length))?;
            Some((saved, alignment))
        } else {
            None
        };
        let _row_guard = ScopeGuard::new(move || {
            if let Some((saved, alignment)) = saved_state {
                if pixel_storei(gl::UNPACK_ROW_LENGTH, saved).is_err() {
                    log::debug!("FrameView: UNPACK_ROW_LENGTH reset failed");
                }
                if pixel_storei(gl::UNPACK_ALIGNMENT, alignment).is_err() {
                    log::debug!("FrameView: UNPACK_ALIGNMENT reset failed");
                }
            }
        });

        if self.is_palette_format {
            use_texture(1, self.palette_texture)?;
            use_texture(2, self.ntsc_texture)?;
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
                return Err(ViewError::NoVertexArray { pipeline: "game" });
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.vbo.as_ref() else {
                return Err(ViewError::NoVertexBuffer { pipeline: "game" });
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
        drop(_row_guard);
        ok_gl(draw_arrays(gl::TRIANGLE_STRIP, 0, 4))?;
        // Release-time batched drain (debug wrappers already drained).
        let _ = check_frame();
        // Explicit restore: back to unit 0 so the OSD pass starts clean.
        ok_gl(active_texture(gl::TEXTURE0))?;
        Ok(())
    }

    /// Whether the context accepts UNPACK_ROW_LENGTH (no CPU repack).
    pub(crate) fn supports_unpack_row_length(&self) -> bool {
        self.unpack_row_length
    }

    /// Bind game geometry for the OSD composite pass.
    pub(crate) fn bind_geometry(&self) -> Result<(), ViewError> {
        if self.use_vao {
            let Some(vba) = self.vba.as_ref() else {
                return Err(ViewError::NoVertexArray { pipeline: "game" });
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.vbo.as_ref() else {
                return Err(ViewError::NoVertexBuffer { pipeline: "game" });
            };
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, vbo.id))?;
        }
        Ok(())
    }

    pub(crate) fn current_program(&self) -> Option<u32> {
        self.current_program
    }

    pub(crate) fn set_current_program(&mut self, id: Option<u32>) {
        self.current_program = id;
    }

    /// Rebind the game texture plus program after the OSD composite.
    /// The next game upload never samples the overlay.
    pub(crate) fn restore_game_binding(&mut self) -> Result<(), ViewError> {
        if self.frame_texture != 0 {
            use_texture(0, self.frame_texture)?;
        }
        self.use_game_program().ok();
        Ok(())
    }

    /// Load the frame pipeline for a render profile.
    /// Returns the GLES verdict for the overlay program selection.
    pub(crate) fn load_frame(
        &mut self,
        render_profile: &VideoRenderProfile,
    ) -> Result<bool, ViewError> {
        self.uniforms = GameUniforms::default();
        self.last_matrix = None;
        self.current_program = None;
        self.is_palette_format = render_profile.frame_format == VideoFrameFormat::Palette;
        let version = gl_string(gl::VERSION);
        let is_gles = is_gles_context(version.as_deref());
        self.unpack_row_length = supports_unpack_row_length(version.as_deref());
        let shader = compile_shader_program(self.is_palette_format, is_gles)?;
        self.shader = Some(shader);
        self.current_program = None;
        self.use_game_program()?;
        ok_gl(clear_color(0.0, 0.0, 0.0, 1.0))?;

        ok_gl(pixel_storei(gl::UNPACK_ALIGNMENT, 1))?;

        self.load_frame_textures(render_profile)?;

        if self.is_palette_format {
            self.load_palette_ntsc(render_profile)?;
        }

        self.init_geometry()?;
        Ok(is_gles)
    }

    /// Fit the game into the surface minus reserved OSD strips.
    /// GL viewports start at the bottom-left.
    pub(crate) fn apply_viewport(
        &mut self,
        viewport_width: i32,
        viewport_height: i32,
        strip: ReservedStrip,
    ) -> Result<[i32; 4], ViewError> {
        // Pure fit (f64 ratios, tested in `layout`): the view only
        // uploads the resulting box and matrix.
        let (game_box, arr) = crate::layout::fit_viewport(
            viewport_width,
            viewport_height,
            strip,
            self.logical_width,
            self.logical_height,
        )?;
        let [_, bottom, area_width, area_height] = game_box;

        self.use_game_program()?;
        self.bind_geometry()?;
        ok_gl(viewport(0, bottom, area_width, area_height))?;
        // Resize always records: force the upload even if the cached
        // matrix matches, so a stale GPU value cannot survive.
        self.last_matrix = None;
        self.set_game_matrix(&arr)?;
        Ok(game_box)
    }

    /// Explicit teardown: `None` (no current context) skips GL calls
    /// and reports each live name as leaked instead of deleting blind.
    pub(crate) fn close(&mut self, current: Option<CurrentContext>) -> Vec<LeakedName> {
        // Teardown ledger policy: every missed delete counts (see `CloseReport`).
        let policy = DeletionPolicy::Count;
        let mut leaked = Vec::new();
        let has_context = current.is_some();
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
        if !ids.is_empty() {
            if !has_context {
                leaked.extend(
                    ids.iter()
                        .filter(|_| policy.note(true))
                        .map(|_| LeakedName::Texture(0)),
                );
                log::debug!("FrameView teardown skipped: no current context");
            } else if delete_textures(ids.len() as i32, ids.as_ptr()).is_err() {
                log::debug!("FrameView teardown failed");
                if policy.note(true) {
                    leaked.push(LeakedName::Texture(0));
                }
            }
        }
        self.frame_texture = 0;
        self.palette_texture = 0;
        self.ntsc_texture = 0;
        self.last_matrix = None;
        self.current_program = None;
        if let Some(mut shader) = self.shader.take()
            && shader.destroy_if_current(current)
        {
            leaked.push(LeakedName::Program(0));
        }
        if let Some(mut vba) = self.vba.take()
            && vba.destroy_if_current(current)
        {
            leaked.push(LeakedName::Array(0));
        }
        if let Some(vbo) = self.vbo.take()
            && vbo.destroy_if_current(current)
        {
            leaked.push(LeakedName::Buffer(0));
        }
        leaked
    }
}

#[cfg(test)]
mod tests {
    use super::FrameView;
    use crate::error::ViewError;

    #[test]
    fn program_binding_without_shader_names_game() {
        let mut view = FrameView::new();
        assert!(matches!(
            view.on_update(&[0; 4], 1),
            Err(ViewError::NoProgram { program: "game" })
        ));
        assert!(matches!(view.restore_game_binding(), Ok(())));
    }

    #[test]
    fn palette_upload_skips_rgba_frames() {
        let view = FrameView::new();
        assert!(view.update_palette_texture(&[0; 256]).is_ok());
    }

    #[test]
    fn geometry_binding_names_missing_pipeline() {
        // Default build skips the VAO: missing VBO names the buffer.
        let view = FrameView::new();
        assert!(matches!(
            view.bind_geometry(),
            Err(ViewError::NoVertexBuffer { pipeline: "game" })
        ));
        let mut vao = FrameView::new();
        vao.set_use_vao(true);
        assert!(matches!(
            vao.bind_geometry(),
            Err(ViewError::NoVertexArray { pipeline: "game" })
        ));
    }

    #[test]
    fn matrix_cache_hit_skips_upload() {
        // Identical matrix returns before any GL call.
        let mut view = FrameView::new();
        let mat = crate::mat4::Mat4::identity().as_array();
        view.last_matrix = Some(mat);
        assert!(view.set_game_matrix(&mat).is_ok());
        // Stale cache with no program fails typed, not with `-1`.
        let mut fresh = FrameView::new();
        assert!(matches!(
            fresh.set_game_matrix(&mat),
            Err(ViewError::NoProgram { program: "game" })
        ));
    }

    #[test]
    fn current_program_tracks_explicit_marks() {
        let mut view = FrameView::new();
        assert_eq!(view.current_program(), None);
        view.set_current_program(Some(7));
        assert_eq!(view.current_program(), Some(7));
        view.set_current_program(None);
        assert_eq!(view.current_program(), None);
    }

    #[test]
    fn close_without_context_leaks_live_names() {
        use nerust_glwrap::CurrentContext;
        let mut view = FrameView::new();
        assert!(view.close(CurrentContext::assert_current(false)).is_empty());
        let mut live = FrameView::new();
        live.frame_texture = 7;
        live.palette_texture = 8;
        let leaked = live.close(CurrentContext::assert_current(false));
        assert_eq!(leaked.len(), 2);
        assert_eq!(live.frame_texture, 0);
    }

    #[test]
    fn viewport_rejects_degenerate_content_before_gl() {
        use nerust_render_traits::ReservedStrip;
        let mut view = FrameView::new();
        view.logical_width = 0;
        view.logical_height = 0;
        assert!(
            view.apply_viewport(64, 64, ReservedStrip::default())
                .is_err()
        );
    }
}
