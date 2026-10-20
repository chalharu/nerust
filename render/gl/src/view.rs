use std::{os::raw::c_void, ptr, rc::Rc};

use gl::types::GLint;
use nerust_glwrap::{CurrentContext, Shader, raw::*, vertex::*};
use nerust_render_ntsc::NTSC_TEXTURE_HEIGHT;
use nerust_render_traits::{
    ReservedStrip, VideoFrameFormat, VideoRenderProfile, renderer::OsdLayer,
};

use crate::{
    frame_textures::{
        allocate_texture_store, configure_ntsc_texture, upload_texture_bytes, use_texture,
    },
    mat4::Mat4,
    osd_overlay::{
        OSDRect, RetryBudget, RetryDecision, checked_clear_need, checked_osd_gl_y, checked_osd_rect,
    },
    shader_compiler::{
        compile_osd_program, compile_shader_program, is_gles_context, supports_unpack_row_length,
    },
    vec2d::Vec2D,
    vertex_data::VertexData,
};

pub(crate) use crate::error::ViewError;

/// Outcome of one OSD draw. Transient failures propagate as `Err` (retried);
/// only the give-up transition reports `Dropped`; `Skipped` means nothing staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsdOutcome {
    Presented,
    Skipped,
    Dropped,
}

/// Convert a GL call result into a view error for `?` propagation.
fn ok_gl<T>(result: Result<T, nerust_glwrap::GlError>) -> Result<T, ViewError> {
    result.map_err(ViewError::Gl)
}

/// Convert a GL call in the OSD path into a rect-carrying error so
/// logs name the failing subrect and stage instead of a bare GL code.
fn ok_osd_gl<T>(
    result: Result<T, nerust_glwrap::GlError>,
    rect: (i32, i32, i32, i32),
    stage: &'static str,
) -> Result<T, ViewError> {
    result.map_err(|source| ViewError::OsdUpload {
        rect,
        stage,
        source,
    })
}

/// RAII reset for `UNPACK_ROW_LENGTH` and `UNPACK_ALIGNMENT`.
/// Drop restores both best-effort; construction failure leaves state untouched.
struct PixelStoreGuard {
    alignment: i32,
    row_length: i32,
    active: bool,
}

impl PixelStoreGuard {
    fn set(row_length: i32) -> Result<Self, ViewError> {
        let alignment = ok_gl(get_integer(gl::UNPACK_ALIGNMENT))?;
        debug_assert_eq!(
            alignment, 1,
            "GlView owns UNPACK_ALIGNMENT=1; foreign value would mis-stride uploads",
        );
        let saved = ok_gl(get_integer(gl::UNPACK_ROW_LENGTH))?;
        ok_gl(pixel_storei(gl::UNPACK_ROW_LENGTH, row_length))?;
        Ok(Self {
            alignment,
            row_length: saved,
            active: true,
        })
    }

    fn none() -> Self {
        Self {
            alignment: 1,
            row_length: 0,
            active: false,
        }
    }
}

impl Drop for PixelStoreGuard {
    fn drop(&mut self) {
        if self.active {
            if pixel_storei(gl::UNPACK_ROW_LENGTH, self.row_length).is_err() {
                log::debug!("GlView: UNPACK_ROW_LENGTH reset failed");
            }
            if pixel_storei(gl::UNPACK_ALIGNMENT, self.alignment).is_err() {
                log::debug!("GlView: UNPACK_ALIGNMENT reset failed");
            }
        }
    }
}

/// RAII viewport save/restore for the OSD composite.
struct ViewportGuard {
    saved: [i32; 4],
}

impl ViewportGuard {
    fn fullscreen(surface_w: i32, surface_h: i32) -> Result<Self, ViewError> {
        let mut read = [0; 4];
        ok_gl(get_viewport(&mut read))?;
        ok_gl(viewport(0, 0, surface_w, surface_h))?;
        Ok(Self { saved: read })
    }

    /// Assume a memoized game viewport: skip `GetIntegerv`, restore on drop.
    /// Only correct when the caller memoized the live box.
    fn assume(saved: [i32; 4], surface_w: i32, surface_h: i32) -> Result<Self, ViewError> {
        ok_gl(viewport(0, 0, surface_w, surface_h))?;
        Ok(Self { saved })
    }

    fn saved_viewport(&self) -> [i32; 4] {
        self.saved
    }
}

impl Drop for ViewportGuard {
    fn drop(&mut self) {
        let [x, y, w, h] = self.saved;
        if viewport(x, y, w, h).is_err() {
            log::debug!("GlView: viewport restore failed");
        }
    }
}

/// RAII blend save/restore: pre-enabled BLEND survives the composite.
struct BlendGuard {
    was_enabled: bool,
    src_rgb: u32,
    dst_rgb: u32,
    src_alpha: u32,
    dst_alpha: u32,
}

impl BlendGuard {
    fn enable_src_alpha() -> Result<Self, ViewError> {
        let was_enabled = ok_gl(is_enabled(gl::BLEND))?;
        // Checked `u32` reads: a negative GL enum refuses instead of
        // wrapping through `as u32` at the restore site.
        let src_rgb = ok_gl(get_integer_u32(gl::BLEND_SRC_RGB))?;
        let dst_rgb = ok_gl(get_integer_u32(gl::BLEND_DST_RGB))?;
        let src_alpha = ok_gl(get_integer_u32(gl::BLEND_SRC_ALPHA))?;
        let dst_alpha = ok_gl(get_integer_u32(gl::BLEND_DST_ALPHA))?;
        ok_gl(enable(gl::BLEND))?;
        ok_gl(blend_func(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA))?;
        Ok(Self {
            was_enabled,
            src_rgb,
            dst_rgb,
            src_alpha,
            dst_alpha,
        })
    }
}

impl Drop for BlendGuard {
    fn drop(&mut self) {
        // Restore factors first, then the enable flag: pre-enabled BLEND
        // survives, pre-disabled never leaks on. Errors land in the debug log.
        if let Err(e) =
            blend_func_separate(self.src_rgb, self.dst_rgb, self.src_alpha, self.dst_alpha)
        {
            log::debug!("GlView: blend factor restore failed: {e}");
        }
        if !self.was_enabled && disable(gl::BLEND).is_err() {
            log::debug!("GlView: blend disable restore failed");
        }
    }
}

/// RAII program + active-texture + VAO save/restore.
struct ProgramGuard {
    program: u32,
    active_texture: u32,
    vao_binding: u32,
    array_buffer: u32,
}

impl ProgramGuard {
    fn save() -> Result<Self, ViewError> {
        Ok(Self {
            program: ok_gl(get_integer_u32(gl::CURRENT_PROGRAM))?,
            active_texture: ok_gl(get_integer_u32(gl::ACTIVE_TEXTURE))?,
            vao_binding: ok_gl(get_integer_u32(gl::VERTEX_ARRAY_BINDING))?,
            array_buffer: ok_gl(get_integer_u32(gl::ARRAY_BUFFER_BINDING))?,
        })
    }
}

impl Drop for ProgramGuard {
    fn drop(&mut self) {
        // No raw `gl::` calls: wrappers drain per call and report the
        // GL error chain in the debug log on failure.
        if active_texture(self.active_texture).is_err() {
            log::debug!("GlView: active texture restore failed");
        }
        if bind_buffer(gl::ARRAY_BUFFER, self.array_buffer).is_err() {
            log::debug!("GlView: array buffer restore failed");
        }
        if bind_vertex_array(self.vao_binding).is_err() {
            log::debug!("GlView: VAO restore failed");
        }
        if use_program(self.program).is_err() {
            log::debug!("GlView: program restore failed");
        }
    }
}

/// RAII viewport + blend + program guard for the OSD composite.
/// Drop restores all best-effort so a failed draw never leaks into the next frame.
struct CompositeGuard {
    viewport: ViewportGuard,
    _blend: BlendGuard,
    _program: ProgramGuard,
}

impl CompositeGuard {
    fn fullscreen(
        surface_w: i32,
        surface_h: i32,
        saved: Option<[i32; 4]>,
    ) -> Result<Self, ViewError> {
        // `Some` skips `GetIntegerv` (caller memoized); `None` reads once
        // inside the guard. The guard restores on drop.
        let viewport = match saved {
            Some(memo) => ViewportGuard::assume(memo, surface_w, surface_h)?,
            None => ViewportGuard::fullscreen(surface_w, surface_h)?,
        };
        Ok(Self {
            viewport,
            _blend: BlendGuard::enable_src_alpha()?,
            _program: ProgramGuard::save()?,
        })
    }

    fn saved_viewport(&self) -> [i32; 4] {
        self.viewport.saved_viewport()
    }
}

/// Fallback upload attempts before a failing OSD layer drops.
/// Production views take the max from `RendererConfig`.
pub const MAX_OSD_UPLOAD_RETRIES: u8 = 3;

/// Cached uniform locations for the game pipeline: resolved once at `on_load`.
/// `last_matrix` skips redundant matrix uploads.
#[derive(Debug, Default)]
struct GameUniforms {
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
struct FrameResources {
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
    vba: Option<VertexArray>,
    vbo: Option<Rc<VertexBuffer>>,
    logical_width: i32,
    logical_height: i32,
    /// Separate staging for the NTSC kernel upload (load-time only).
    ntsc_scratch: Vec<u8>,
    uniforms: GameUniforms,
    last_matrix: Option<[f32; 16]>,
    current_program: Option<u32>,
}

/// OSD pipeline resources: layer, program, retry, staging. One owner for
/// the overlay domain; the frame pipeline never touches these.
#[derive(Debug)]
struct OsdResources {
    osd_texture: u32,
    osd_size: (i32, i32),
    osd_prev: Option<(i32, i32, i32, i32)>,
    osd_dirty: bool,
    /// Bounded retry for the staged layer (see `RetryBudget`).
    osd_retry: RetryBudget,
    osd_shader: Option<Shader>,
    osd_layer: Option<OsdLayer>,
    /// Grow-only staging reused across OSD uploads (no steady allocs).
    osd_upload_scratch: Vec<u8>,
    /// Grow-only zeros reused for OSD clears (no steady allocs).
    osd_clear_scratch: Vec<u8>,
    osd_matrix: Option<GLint>,
    last_osd_matrix: Option<[f32; 16]>,
    /// Sampler uniform resolved once in `init_osd` (not per frame).
    osd_texture_loc: Option<GLint>,
    /// Last uploaded sampler unit; unit 0 is the only value ever
    /// bound, so the second and later composites skip the upload.
    last_osd_sampler: Option<i32>,
}

/// Surface memo: last resize inputs plus the applied game viewport.
/// Skips redundant GL state writes and the per-frame `GetIntegerv`.
#[derive(Debug, Default)]
struct SurfaceState {
    last_resize: Option<(i32, i32, ReservedStrip)>,
    game_viewport: Option<[i32; 4]>,
}

impl Default for OsdResources {
    fn default() -> Self {
        Self {
            osd_texture: 0,
            osd_size: (0, 0),
            osd_prev: None,
            osd_dirty: false,
            osd_retry: RetryBudget::new(MAX_OSD_UPLOAD_RETRIES),
            osd_shader: None,
            osd_layer: None,
            osd_upload_scratch: Vec::new(),
            osd_clear_scratch: Vec::new(),
            osd_matrix: None,
            last_osd_matrix: None,
            osd_texture_loc: None,
            last_osd_sampler: None,
        }
    }
}

/// One failed-delete bucket per GL name kind. Best-effort teardown: a failed
/// delete warns but never fails the rebuild that called it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CloseReport {
    pub failed_textures: u32,
    pub failed_programs: u32,
    pub failed_arrays: u32,
    pub failed_buffers: u32,
}

impl CloseReport {
    pub fn is_ok(self) -> bool {
        self.failed_textures == 0
            && self.failed_programs == 0
            && self.failed_arrays == 0
            && self.failed_buffers == 0
    }

    /// Coarse total for warn-once logs.
    pub fn failed_names(self) -> u32 {
        self.failed_textures
            .saturating_add(self.failed_programs)
            .saturating_add(self.failed_arrays)
            .saturating_add(self.failed_buffers)
    }
}

#[derive(Debug)]
pub struct GlView {
    frame: FrameResources,
    osd: OsdResources,
    surface: SurfaceState,
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

impl Default for GlView {
    fn default() -> Self {
        Self::new()
    }
}

impl GlView {
    pub fn new() -> Self {
        Self {
            frame: FrameResources::default(),
            osd: OsdResources::default(),
            surface: SurfaceState::default(),
        }
    }

    pub fn use_vao(&mut self, value: bool) {
        self.frame.use_vao = value;
    }

    pub fn load_with<F: FnMut(&'static str) -> *const c_void>(get_proc_address: F) {
        gl::load_with(get_proc_address);
    }

    /// Use the game program, skipping the GL call when already current.
    fn use_game_program(&mut self) -> Result<(), ViewError> {
        let Some(shader) = self.frame.shader.as_ref() else {
            return Err(ViewError::NoProgram { program: "game" });
        };
        let id = shader.id();
        if self.frame.current_program != Some(id) {
            ok_gl(shader.use_program())?;
            self.frame.current_program = Some(id);
        }
        Ok(())
    }

    /// Upload the game matrix only when it changed since last upload.
    fn set_game_matrix(&mut self, mat: &[f32; 16]) -> Result<(), ViewError> {
        if self.frame.last_matrix == Some(*mat) {
            return Ok(());
        }
        let Some(loc) = self.frame.uniforms.matrix else {
            return Err(ViewError::NoProgram { program: "game" });
        };
        ok_gl(uniform_matrix_4fv(loc, 1, gl::FALSE, mat.as_ptr()))?;
        self.frame.last_matrix = Some(*mat);
        Ok(())
    }

    /// Frame upload target plus the placeholder palette texture.
    /// Store allocates null data; rows arrive via `TexSubImage`.
    fn load_frame_textures(
        &mut self,
        render_profile: &VideoRenderProfile,
    ) -> Result<(), ViewError> {
        let frame_size = if self.frame.is_palette_format {
            render_profile.source_logical_size
        } else {
            render_profile.logical_size
        };
        self.frame.logical_width = i32::try_from(frame_size.width).map_err(|_| {
            crate::layout::FrameError::StrideTooWide {
                have: frame_size.width,
            }
        })?;
        self.frame.logical_height = i32::try_from(frame_size.height).map_err(|_| {
            crate::layout::FrameError::StrideTooWide {
                have: frame_size.height,
            }
        })?;
        let (internal_fmt, data_fmt) = if self.frame.is_palette_format {
            (gl::R8 as GLint, gl::RED)
        } else {
            (gl::RGBA as GLint, gl::RGBA)
        };
        let mut tex_names = [0; 1];
        ok_gl(gen_textures(1, tex_names.as_mut_ptr()))?;
        self.frame.frame_texture = tex_names[0];
        allocate_texture_store(
            0,
            self.frame.frame_texture,
            frame_size.width,
            frame_size.height,
            internal_fmt,
            data_fmt,
        )?;

        if self.frame.is_palette_format {
            self.frame.palette_width = 64;
            self.frame.palette_height = 1;
            let mut pal_names = [0; 1];
            ok_gl(gen_textures(1, pal_names.as_mut_ptr()))?;
            self.frame.palette_texture = pal_names[0];
            // 64x1 RGBA8 placeholder, zeroed via a small stack upload;
            // real data syncs from `FrameBuffer::palette_as_rgba8`.
            upload_texture_bytes(
                1,
                self.frame.palette_texture,
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
            let Some(shader) = self.frame.shader.as_ref() else {
                return Err(ViewError::NoProgram { program: "game" });
            };
            required_uniform(shader, "game", "palette_texture")?
        };
        self.frame.uniforms.palette_texture = Some(palette_loc);
        ok_gl(uniform_1i(palette_loc, 1))?;

        let ntsc_size = render_profile.logical_size;
        if let Some(ntsc_data) = render_profile.ntsc_packed_rgba8.as_deref() {
            self.frame.ntsc_enabled = true;
            let mut ntsc_names = [0; 1];
            ok_gl(gen_textures(1, ntsc_names.as_mut_ptr()))?;
            self.frame.ntsc_texture = ntsc_names[0];
            configure_ntsc_texture(
                2,
                self.frame.ntsc_texture,
                64,
                NTSC_TEXTURE_HEIGHT as usize,
                ntsc_data,
                &mut self.frame.ntsc_scratch,
            )?;
            let loc = {
                let Some(shader) = self.frame.shader.as_ref() else {
                    return Err(ViewError::NoProgram { program: "game" });
                };
                required_uniform(shader, "game", "ntsc_texture")?
            };
            self.frame.uniforms.ntsc_texture = Some(loc);
            ok_gl(uniform_1i(loc, 2))?;
        } else {
            let mut ntsc_names = [0; 1];
            ok_gl(gen_textures(1, ntsc_names.as_mut_ptr()))?;
            self.frame.ntsc_texture = ntsc_names[0];
            let ntsc_height = NTSC_TEXTURE_HEIGHT as usize;
            allocate_texture_store(
                2,
                self.frame.ntsc_texture,
                64,
                ntsc_height,
                gl::RGBA as GLint,
                gl::RGBA,
            )?;
            let loc = {
                let Some(shader) = self.frame.shader.as_ref() else {
                    return Err(ViewError::NoProgram { program: "game" });
                };
                required_uniform(shader, "game", "ntsc_texture")?
            };
            self.frame.uniforms.ntsc_texture = Some(loc);
            ok_gl(uniform_1i(loc, 2))?;
        }

        let (source_loc, output_loc, enabled_loc) = {
            let Some(shader) = self.frame.shader.as_ref() else {
                return Err(ViewError::NoProgram { program: "game" });
            };
            (
                required_uniform(shader, "game", "source_size")?,
                required_uniform(shader, "game", "output_size")?,
                required_uniform(shader, "game", "ntsc_enabled")?,
            )
        };
        self.frame.uniforms.source_size = Some(source_loc);
        self.frame.uniforms.output_size = Some(output_loc);
        self.frame.uniforms.ntsc_enabled = Some(enabled_loc);
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
        ok_gl(uniform_1i(enabled_loc, self.frame.ntsc_enabled as i32))?;
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
            let Some(shader) = self.frame.shader.as_ref() else {
                return Err(ViewError::NoProgram { program: "game" });
            };
            (
                required_attrib(shader, "game", "position")?,
                required_attrib(shader, "game", "uv")?,
            )
        };
        if self.frame.use_vao {
            let vbo = vbo.clone();
            self.frame.vba = Some(ok_gl(VertexArray::new(|vaic| {
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
            self.frame.vbo = Some(vbo.clone());
        }

        let (matrix_loc, frame_loc) = {
            let Some(shader) = self.frame.shader.as_ref() else {
                return Err(ViewError::NoProgram { program: "game" });
            };
            (
                required_uniform(shader, "game", "unif_matrix")?,
                required_uniform(shader, "game", "frame_texture")?,
            )
        };
        self.frame.uniforms.matrix = Some(matrix_loc);
        self.frame.uniforms.frame_texture = Some(frame_loc);
        let identity = Mat4::identity();
        let mat: [f32; 16] = identity.as_array();
        ok_gl(uniform_matrix_4fv(
            matrix_loc,
            1,
            gl::FALSE,
            identity.as_ptr(),
        ))?;
        self.frame.last_matrix = Some(mat);
        ok_gl(uniform_1i(frame_loc, 0))?;
        Ok(())
    }

    pub fn on_load(&mut self, render_profile: &VideoRenderProfile) -> Result<(), ViewError> {
        // Reuse must not leak names: tear down while current first.
        let _ = self.on_close(CurrentContext::assert_current(true));
        debug_assert_eq!(self.frame.frame_texture, 0);
        self.surface.last_resize = None;
        self.surface.game_viewport = None;
        self.frame.uniforms = GameUniforms::default();
        self.frame.last_matrix = None;
        self.frame.current_program = None;
        self.osd.osd_matrix = None;
        self.osd.last_osd_matrix = None;
        self.osd.osd_texture_loc = None;
        self.osd.last_osd_sampler = None;
        self.frame.is_palette_format = render_profile.frame_format == VideoFrameFormat::Palette;
        let version = gl_string(gl::VERSION);
        let is_gles = is_gles_context(version.as_deref());
        self.frame.unpack_row_length = supports_unpack_row_length(version.as_deref());
        let shader = compile_shader_program(self.frame.is_palette_format, is_gles)?;
        self.frame.shader = Some(shader);
        self.frame.current_program = None;
        self.use_game_program()?;
        ok_gl(clear_color(0.0, 0.0, 0.0, 1.0))?;

        ok_gl(pixel_storei(gl::UNPACK_ALIGNMENT, 1))?;

        self.load_frame_textures(render_profile)?;

        if self.frame.is_palette_format {
            self.load_palette_ntsc(render_profile)?;
        }

        self.init_geometry()?;
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
        self.osd.osd_texture = names[0];
        if upload_texture_bytes(
            0,
            self.osd.osd_texture,
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
        self.osd.osd_size = (1, 1);
        let Some(shader) = compile_osd_program(is_gles) else {
            return;
        };
        self.osd.osd_matrix = shader.uniform_opt("unif_matrix");
        // Optional sampler resolved once here: a dropped `osd_texture`
        // defaults to unit 0, so the composite skips the upload instead of failing.
        self.osd.osd_texture_loc = shader.uniform_opt("osd_texture");
        self.osd.last_osd_matrix = None;
        self.osd.last_osd_sampler = None;
        self.osd.osd_shader = Some(shader);
    }
}

impl GlView {
    /// Override the upload retry budget. Equal layers skip staging;
    /// new content resets the failure budget.
    pub fn set_osd_retry_max(&mut self, max: u8) {
        self.osd.osd_retry = RetryBudget::new(max);
    }

    pub fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        if OsdLayer::same_option(self.osd.osd_layer.as_ref(), layer.as_ref()) {
            return;
        }
        self.osd.osd_layer = layer;
        self.osd.osd_dirty = true;
        self.osd.osd_retry.reset();
    }

    /// Use the OSD program, skipping the GL call when already current.
    /// Exactly one program is current at a time; the guard restores it after.
    fn use_osd_program(&mut self) -> Result<(), ViewError> {
        let Some(shader) = self.osd.osd_shader.as_ref() else {
            return Err(ViewError::NoProgram { program: "osd" });
        };
        let id = shader.id();
        if self.frame.current_program != Some(id) {
            ok_gl(shader.use_program())?;
            self.frame.current_program = Some(id);
        }
        Ok(())
    }

    /// Composite the staged layer over the game image, if any (best-effort).
    /// A GL failure drops the overlay, never the frame.
    pub fn draw_osd(&mut self, surface_w: i32, surface_h: i32) -> Result<OsdOutcome, ViewError> {
        if self.osd.osd_dirty {
            match self.sync_osd_texture(surface_w, surface_h) {
                Ok(()) => {
                    self.osd.osd_dirty = false;
                    self.osd.osd_retry.reset();
                }
                Err(e) => {
                    if matches!(self.note_osd_upload_failure(), RetryDecision::DropLayer) {
                        return Ok(OsdOutcome::Dropped);
                    }
                    return Err(e);
                }
            }
        }
        let has_layer = self.osd.osd_layer.is_some();
        let has_shader = self.osd.osd_shader.is_some();
        if !(has_layer && has_shader) {
            return Ok(OsdOutcome::Skipped);
        }
        self.use_osd_program()?;
        // Cached OSD matrix: skip the upload when unchanged.
        {
            let loc = self.osd.osd_matrix;
            let last = self.osd.last_osd_matrix;
            let identity = Mat4::identity();
            let mat: [f32; 16] = identity.as_array();
            if last != Some(mat) {
                if let Some(sampler_loc) = loc {
                    ok_gl(uniform_matrix_4fv(
                        sampler_loc,
                        1,
                        gl::FALSE,
                        identity.as_ptr(),
                    ))?;
                    self.osd.last_osd_matrix = Some(mat);
                } else if self.osd.osd_shader.is_some() {
                    // Structural matrix missing from the OSD program:
                    // fail typed with program identity instead of -1.
                    return Err(ViewError::MissingUniform {
                        name: "unif_matrix",
                        program: "osd",
                    });
                }
            }
        }
        // Optional sampler resolved once in `init_osd`: dropped means unit 0,
        // so skip the upload instead of failing; only the first composite uploads.
        if let Some(sampler) = self.osd.osd_texture_loc
            && self.osd.last_osd_sampler != Some(0)
        {
            ok_gl(uniform_1i(sampler, 0))?;
            self.osd.last_osd_sampler = Some(0);
        }
        use_texture(0, self.osd.osd_texture)?;
        if self.frame.use_vao {
            let Some(vba) = self.frame.vba.as_ref() else {
                return Err(ViewError::NoVertexArray { pipeline: "game" });
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.frame.vbo.as_ref() else {
                return Err(ViewError::NoVertexBuffer { pipeline: "game" });
            };
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, vbo.id))?;
        }
        // Fullscreen composite under the guard: all state restores on drop.
        // The memoized viewport skips the per-frame `GetIntegerv`.
        let _composite =
            CompositeGuard::fullscreen(surface_w, surface_h, self.surface.game_viewport)?;
        self.surface.game_viewport = Some(_composite.saved_viewport());
        ok_gl(draw_arrays(gl::TRIANGLE_STRIP, 0, 4))?;
        // Guard drops here restoring game state; mark program unknown so
        // the next game upload re-binds explicitly.
        drop(_composite);
        self.frame.current_program = None;
        // Explicit restore: rebind the game texture so the next frame
        // never samples the overlay.
        if self.frame.frame_texture != 0 {
            use_texture(0, self.frame.frame_texture)?;
        }
        self.use_game_program().ok();
        Ok(OsdOutcome::Presented)
    }

    /// Record one failed OSD upload (pure state; GL stays out).
    /// Returns the retry decision directly; the caller matches it.
    fn note_osd_upload_failure(&mut self) -> RetryDecision {
        let Some(_) = self.osd.osd_layer.as_ref() else {
            return RetryDecision::Retry;
        };
        match self.osd.osd_retry.record_failure() {
            RetryDecision::Retry => RetryDecision::Retry,
            RetryDecision::DropLayer => {
                // No ghost composite, and the next identical push
                // re-stages via the equality gate.
                self.osd.osd_layer = None;
                self.osd.osd_dirty = false;
                RetryDecision::DropLayer
            }
        }
    }

    /// Upload the staged layer into the full-surface texture (draw path only binds).
    /// Best-effort: failures keep the previous texture, never panic.
    fn sync_osd_texture(&mut self, surface_w: i32, surface_h: i32) -> Result<(), ViewError> {
        if self.osd.osd_layer.is_none() {
            if let Some((px, py, pw, ph)) = self.osd.osd_prev {
                let texture = self.osd.osd_texture;
                self.clear_rect(texture, surface_h, px, py, pw, ph)?;
                self.osd.osd_prev = None;
            }
            return Ok(());
        }
        if self.osd.osd_texture == 0 {
            return Ok(());
        }
        if self.osd.osd_size != (surface_w, surface_h) {
            let w = usize::try_from(surface_w.max(1)).map_err(|_| ViewError::InvalidSurface {
                w: surface_w,
                h: surface_h,
            })?;
            let h = usize::try_from(surface_h.max(1)).map_err(|_| ViewError::InvalidSurface {
                w: surface_w,
                h: surface_h,
            })?;
            use_texture(0, self.osd.osd_texture)?;
            ok_gl(tex_image_2d(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                surface_w,
                surface_h,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                ptr::null(),
            ))?;
            let _ = (w, h);
            self.osd.osd_size = (surface_w, surface_h);
        }
        let staged = match self.osd.osd_layer.as_ref() {
            Some(layer) => match checked_osd_rect(layer, surface_w, surface_h) {
                Ok(rect) => rect,
                Err(reason) => {
                    log::warn!("GlView: dropping invalid OSD layer ({reason:?})");
                    self.osd.osd_layer = None;
                    self.osd.osd_dirty = false;
                    return Ok(());
                }
            },
            None => return Ok(()),
        };
        let OSDRect {
            x: lx,
            y: ly,
            w: lw,
            h: lh,
            stride,
            need,
        } = staged;
        let rect = (lx, ly, lw, lh);
        if self.osd.osd_prev != Some(rect) {
            if let Some((px, py, pw, ph)) = self.osd.osd_prev {
                let texture = self.osd.osd_texture;
                self.clear_rect(texture, surface_h, px, py, pw, ph)?;
            }
            self.osd.osd_prev = Some(rect);
        }
        if self.osd.osd_upload_scratch.len() < need {
            self.osd.osd_upload_scratch.resize(need, 0);
        }
        let Some(layer) = self.osd.osd_layer.as_ref() else {
            return Ok(());
        };
        let texture = self.osd.osd_texture;
        if layer.rgba.len() == need {
            Self::blit_bytes(texture, surface_h, lx, ly, lw, lh, &layer.rgba[..need])?;
            return Ok(());
        }
        {
            let scratch = &mut self.osd.osd_upload_scratch[..need];
            scratch.fill(0);
            for (index, row) in layer.rgba.chunks_exact(stride).enumerate() {
                scratch[index * stride..index * stride + stride].copy_from_slice(row);
            }
        }
        Self::blit_bytes(
            texture,
            surface_h,
            lx,
            ly,
            lw,
            lh,
            &self.osd.osd_upload_scratch[..need],
        )?;
        Ok(())
    }

    /// Clear a subrect to transparent using the reusable zero scratch.
    /// `surface_h` flips the top-left OSD origin to GL bottom-left.
    fn clear_rect(
        &mut self,
        texture: u32,
        surface_h: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<(), ViewError> {
        let need = checked_clear_need(w, h).map_err(|_| ViewError::InvalidOsdClear { w, h })?;
        if self.osd.osd_clear_scratch.len() < need {
            self.osd.osd_clear_scratch.resize(need, 0);
        } else {
            self.osd.osd_clear_scratch[..need].fill(0);
        }
        let gl_y = checked_osd_gl_y(surface_h, y, h).map_err(|_| ViewError::InvalidOsdFlip {
            surface_h,
            y,
            h,
        })?;
        let rect = (x, y, w, h);
        use_texture(0, texture)?;
        ok_osd_gl(
            tex_sub_image_2d(
                gl::TEXTURE_2D,
                0,
                x,
                gl_y,
                w,
                h,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                self.osd.osd_clear_scratch.as_ptr().cast(),
            ),
            rect,
            "clear",
        )?;
        Ok(())
    }

    /// Upload RGBA rows into a texture subrect (best-effort).
    /// `surface_h` flips the top-left OSD origin to GL bottom-left.
    fn blit_bytes(
        texture: u32,
        surface_h: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        bytes: &[u8],
    ) -> Result<(), ViewError> {
        let gl_y = checked_osd_gl_y(surface_h, y, h).map_err(|_| ViewError::InvalidOsdFlip {
            surface_h,
            y,
            h,
        })?;
        let rect = (x, y, w, h);
        use_texture(0, texture)?;
        ok_osd_gl(
            tex_sub_image_2d(
                gl::TEXTURE_2D,
                0,
                x,
                gl_y,
                w,
                h,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                bytes.as_ptr().cast(),
            ),
            rect,
            "blit",
        )?;
        Ok(())
    }

    pub fn update_palette_texture(&self, rgba8: &[u8; 256]) -> Result<(), ViewError> {
        if !self.frame.is_palette_format {
            return Ok(());
        }
        use_texture(1, self.frame.palette_texture)?;
        ok_gl(tex_sub_image_2d(
            gl::TEXTURE_2D,
            0,
            0,
            0,
            self.frame.palette_width,
            self.frame.palette_height,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            rgba8.as_ptr().cast(),
        ))?;
        // Explicit restore: unit 0 + frame texture, so the game upload
        // below never samples the palette on unit 1.
        if self.frame.frame_texture != 0 {
            use_texture(0, self.frame.frame_texture)?;
        } else {
            ok_gl(active_texture(gl::TEXTURE0))?;
        }
        Ok(())
    }

    pub fn on_update(&mut self, screen: &[u8], row_length: i32) -> Result<(), ViewError> {
        let screen_ptr = screen.as_ptr().cast();
        self.use_game_program()?;

        use_texture(0, self.frame.frame_texture)?;

        let _row_guard = if row_length != self.frame.logical_width {
            PixelStoreGuard::set(row_length)?
        } else {
            PixelStoreGuard::none()
        };

        if self.frame.is_palette_format {
            use_texture(1, self.frame.palette_texture)?;
            use_texture(2, self.frame.ntsc_texture)?;
            use_texture(0, self.frame.frame_texture)?;
            ok_gl(tex_sub_image_2d(
                gl::TEXTURE_2D,
                0,
                0,
                0,
                self.frame.logical_width,
                self.frame.logical_height,
                gl::RED,
                gl::UNSIGNED_BYTE,
                screen_ptr,
            ))?;
        }

        if self.frame.use_vao {
            let Some(vba) = self.frame.vba.as_ref() else {
                return Err(ViewError::NoVertexArray { pipeline: "game" });
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.frame.vbo.as_ref() else {
                return Err(ViewError::NoVertexBuffer { pipeline: "game" });
            };
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, vbo.id))?;
        }

        ok_gl(clear(gl::COLOR_BUFFER_BIT))?;

        if !self.frame.is_palette_format {
            ok_gl(tex_sub_image_2d(
                gl::TEXTURE_2D,
                0,
                0,
                0,
                self.frame.logical_width,
                self.frame.logical_height,
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
    pub fn supports_unpack_row_length(&self) -> bool {
        self.frame.unpack_row_length
    }

    /// Fit the game into the surface minus reserved OSD strips.
    /// GL viewports start at the bottom-left.
    pub fn on_resize_with_strip(
        &mut self,
        viewport_width: i32,
        viewport_height: i32,
        strip: nerust_render_traits::ReservedStrip,
    ) -> Result<(), ViewError> {
        // Zero/negative viewports: nothing to draw; skip GL work.
        if viewport_width <= 0 || viewport_height <= 0 {
            return Ok(());
        }
        if self.surface.last_resize == Some((viewport_width, viewport_height, strip)) {
            return Ok(());
        }
        // Pure fit (f64 ratios, tested in `layout`): the view only
        // uploads the resulting box and matrix.
        let (game_box, arr) = crate::layout::fit_viewport(
            viewport_width,
            viewport_height,
            strip,
            self.frame.logical_width,
            self.frame.logical_height,
        )?;
        let [_, bottom, area_width, area_height] = game_box;

        self.use_game_program()?;
        if self.frame.use_vao {
            let Some(vba) = self.frame.vba.as_ref() else {
                return Err(ViewError::NoVertexArray { pipeline: "game" });
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.frame.vbo.as_ref() else {
                return Err(ViewError::NoVertexBuffer { pipeline: "game" });
            };
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, vbo.id))?;
        }
        ok_gl(viewport(0, bottom, area_width, area_height))?;
        // Resize always records: force the upload even if the cached
        // matrix matches, so a stale GPU value cannot survive.
        self.frame.last_matrix = None;
        self.set_game_matrix(&arr)?;
        self.surface.last_resize = Some((viewport_width, viewport_height, strip));
        self.surface.game_viewport = Some(game_box);
        Ok(())
    }

    /// Explicit teardown. `None` (no current context) skips GL calls
    /// and reports each live name as failed instead of deleting blind.
    pub fn on_close(&mut self, current: Option<CurrentContext>) -> CloseReport {
        let mut report = CloseReport::default();
        let has_context = current.is_some();
        let mut ids = Vec::new();
        if self.frame.frame_texture != 0 {
            ids.push(self.frame.frame_texture);
        }
        if self.frame.palette_texture != 0 {
            ids.push(self.frame.palette_texture);
        }
        if self.frame.ntsc_texture != 0 {
            ids.push(self.frame.ntsc_texture);
        }
        if self.osd.osd_texture != 0 {
            ids.push(self.osd.osd_texture);
        }
        if !ids.is_empty() {
            if !has_context {
                let count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
                debug_assert!(u32::try_from(ids.len()).is_ok());
                report.failed_textures += count;
                log::debug!("GL view teardown skipped: no current context");
            } else if let Err(e) = delete_textures(ids.len() as i32, ids.as_ptr()) {
                log::debug!("GL view teardown failed: {e}");
                report.failed_textures += 1;
            }
        }
        self.frame.frame_texture = 0;
        self.frame.palette_texture = 0;
        self.frame.ntsc_texture = 0;
        self.osd.osd_texture = 0;
        self.osd.osd_size = (0, 0);
        self.osd.osd_prev = None;
        self.surface.last_resize = None;
        self.surface.game_viewport = None;
        self.frame.last_matrix = None;
        self.osd.last_osd_matrix = None;
        self.osd.last_osd_sampler = None;
        self.frame.current_program = None;
        if let Some(mut shader) = self.frame.shader.take()
            && shader.destroy_if_current(current)
        {
            report.failed_programs += 1;
        }
        if let Some(mut shader) = self.osd.osd_shader.take()
            && shader.destroy_if_current(current)
        {
            report.failed_programs += 1;
        }
        if let Some(mut vba) = self.frame.vba.take()
            && vba.destroy_if_current(current)
        {
            report.failed_arrays += 1;
        }
        if let Some(vbo) = self.frame.vbo.take()
            && vbo.destroy_if_current(current)
        {
            report.failed_buffers += 1;
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::{GlView, OsdOutcome, RetryBudget, RetryDecision, ok_gl};
    use crate::{
        osd_overlay::{OSDRect, OsdReject, checked_osd_rect, osd_gl_y},
        shader_compiler::{
            build_glsl_source, compose_glsl_source, is_gles_context, supports_unpack_row_length,
        },
    };
    use nerust_render_traits::renderer::OsdLayer;

    #[test]
    fn checked_osd_rect_accepts_inside_and_rejects_rest() {
        let inside = OsdLayer {
            x: 4,
            y: 6,
            width: 2,
            height: 1,
            rgba: vec![1; 8],
        };
        assert_eq!(
            checked_osd_rect(&inside, 64, 64),
            Ok(OSDRect {
                x: 4,
                y: 6,
                w: 2,
                h: 1,
                stride: 8,
                need: 8,
            })
        );
        let short = OsdLayer {
            rgba: vec![1; 7],
            ..inside.clone()
        };
        assert_eq!(
            checked_osd_rect(&short, 64, 64),
            Err(OsdReject::Short { need: 8, have: 7 })
        );
        let flat = OsdLayer {
            height: 0,
            rgba: vec![],
            ..inside.clone()
        };
        assert_eq!(checked_osd_rect(&flat, 64, 64), Err(OsdReject::Empty));
        let outside = OsdLayer {
            x: 63,
            ..inside.clone()
        };
        assert_eq!(
            checked_osd_rect(&outside, 64, 64),
            Err(OsdReject::OutOfBounds)
        );
        let huge = OsdLayer {
            width: u32::MAX,
            height: u32::MAX,
            rgba: vec![],
            ..inside.clone()
        };
        assert_eq!(
            checked_osd_rect(&huge, i32::MAX, i32::MAX),
            Err(OsdReject::Empty)
        );
    }

    #[test]
    fn checked_osd_rect_rejects_i32_overflow_as_out_of_bounds() {
        // u32 coords above i32::MAX refuse instead of wrapping negative.
        let wide = OsdLayer {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        };
        assert_eq!(
            checked_osd_rect(&wide, i32::MAX, i32::MAX),
            Err(OsdReject::OutOfBounds)
        );
    }

    #[test]
    fn osd_gl_y_flips_top_left_to_bottom_left() {
        // 64px surface: top row (y=0,h=1) uploads at GL row 63.
        assert_eq!(osd_gl_y(64, 0, 1), 63);
        assert_eq!(osd_gl_y(64, 6, 1), 57);
        assert_eq!(osd_gl_y(64, 63, 1), 0);
    }

    #[test]
    fn set_osd_layer_skips_equal_upload() {
        let mut view = GlView::default();
        let layer = OsdLayer {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![255; 8],
        };
        view.set_osd_layer(Some(layer.clone()));
        assert!(view.osd.osd_dirty);
        view.osd.osd_dirty = false;
        view.set_osd_layer(Some(layer));
        assert!(!view.osd.osd_dirty);
        view.set_osd_layer(None);
        assert!(view.osd.osd_dirty);
    }

    #[test]
    fn osd_equality_gate_checks_header_before_bytes() {
        let base = OsdLayer {
            x: 1,
            y: 2,
            width: 2,
            height: 1,
            rgba: vec![9; 8],
        };
        assert!(OsdLayer::same_option(Some(&base), Some(&base.clone())));
        assert!(!OsdLayer::same_option(
            Some(&base),
            Some(&OsdLayer {
                x: 7,
                ..base.clone()
            })
        ));
        assert!(!OsdLayer::same_option(
            Some(&base),
            Some(&OsdLayer {
                rgba: vec![8; 8],
                ..base.clone()
            })
        ));
        assert!(!OsdLayer::same_option(
            Some(&base),
            Some(&OsdLayer {
                rgba: vec![9; 4],
                width: 1,
                ..base.clone()
            })
        ));
        assert!(OsdLayer::same_option(None, None));
        assert!(!OsdLayer::same_option(Some(&base), None));
        assert!(!OsdLayer::same_option(None, Some(&base)));
    }

    #[test]
    fn retry_budget_pins_boundaries() {
        let mut budget = RetryBudget::new(3);
        assert_eq!(budget.record_failure(), RetryDecision::Retry);
        assert_eq!(budget.record_failure(), RetryDecision::Retry);
        assert_eq!(budget.record_failure(), RetryDecision::DropLayer);
        assert_eq!(budget.record_failure(), RetryDecision::DropLayer);
        budget.reset();
        assert_eq!(budget.record_failure(), RetryDecision::Retry);
    }

    #[test]
    fn frame_error_carries_sizes() {
        let err = super::ViewError::from(crate::layout::FrameError::TooShort { need: 10, have: 4 });
        assert!(err.to_string().contains("10"));
        let internal = super::ViewError::NoProgram { program: "game" };
        assert!(internal.to_string().contains("shader"));
    }

    #[test]
    fn missing_identities_name_program() {
        let err = super::ViewError::MissingUniform {
            name: "unif_matrix",
            program: "game",
        };
        assert!(err.to_string().contains("game"));
        let err = super::ViewError::MissingAttribute {
            name: "position",
            program: "game",
        };
        assert!(err.to_string().contains("position"));
        let err = super::ViewError::OsdUpload {
            rect: (0, 0, 2, 1),
            stage: "blit",
            source: nerust_glwrap::GlError::new(vec![gl::INVALID_VALUE.into()]),
        };
        assert!(err.to_string().contains("blit"));
    }

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

    fn staged_view() -> GlView {
        let mut view = GlView::default();
        view.set_osd_layer(Some(OsdLayer {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![255; 8],
        }));
        view
    }

    #[test]
    fn upload_failures_retry_then_drop_layer() {
        let mut view = staged_view();
        assert_eq!(view.note_osd_upload_failure(), RetryDecision::Retry);
        assert!(view.osd.osd_layer.is_some());
        assert!(view.osd.osd_dirty);
        assert_eq!(view.note_osd_upload_failure(), RetryDecision::Retry);
        assert!(view.osd.osd_layer.is_some());
        assert_eq!(view.note_osd_upload_failure(), RetryDecision::DropLayer);
        assert!(view.osd.osd_layer.is_none());
        assert!(!view.osd.osd_dirty);
        assert_eq!(view.note_osd_upload_failure(), RetryDecision::Retry);
        assert!(view.osd.osd_layer.is_none());
        assert!(!view.osd.osd_dirty);
    }

    #[test]
    fn upload_failure_counter_saturates() {
        let mut view = staged_view();
        view.osd.osd_retry.fails = u8::MAX;
        assert_eq!(view.note_osd_upload_failure(), RetryDecision::DropLayer);
        assert_eq!(view.osd.osd_retry.fails, u8::MAX);
        assert!(view.osd.osd_layer.is_none());
    }

    #[test]
    fn dropped_outcome_is_copyable_fact() {
        let outcome = OsdOutcome::Dropped;
        assert_eq!(outcome, OsdOutcome::Dropped);
        assert_ne!(outcome, OsdOutcome::Presented);
    }

    #[test]
    fn ok_gl_passes_values_and_maps_errors() {
        assert_eq!(ok_gl::<u32>(Ok(7)).unwrap(), 7);
        let mapped = ok_gl::<u32>(Err(nerust_glwrap::GlError::new(vec![
            gl::INVALID_VALUE.into(),
        ])));
        assert!(matches!(mapped, Err(super::ViewError::Gl(_))));
    }

    #[test]
    fn close_report_without_context_counts_names() {
        use nerust_glwrap::CurrentContext;
        let mut view = GlView::default();
        view.frame.frame_texture = 7;
        let report = view.on_close(CurrentContext::assert_current(false));
        assert!(!report.is_ok());
        assert_eq!(report.failed_textures, 1);
        assert_eq!(report.failed_names(), 1);
        assert_eq!(view.frame.frame_texture, 0);
    }

    #[cfg(unix)]
    fn gl_test_profile(
        format: nerust_render_traits::VideoFrameFormat,
        ntsc: Option<Box<[u8]>>,
    ) -> nerust_render_traits::VideoRenderProfile {
        use nerust_render_traits::{logical::LogicalSize, physical::PhysicalSize};
        nerust_render_traits::VideoRenderProfile {
            frame_format: format,
            source_logical_size: LogicalSize {
                width: 64,
                height: 64,
            },
            logical_size: LogicalSize {
                width: 64,
                height: 64,
            },
            physical_size: PhysicalSize {
                width: 64.0,
                height: 64.0,
            },
            ntsc_packed_rgba8: ntsc,
        }
    }

    #[cfg(unix)]
    fn gl_test_layer() -> OsdLayer {
        OsdLayer {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            rgba: vec![255; 8],
        }
    }

    /// Full view drive on a real context (Xvfb + llvmpipe in CI).
    /// Skips without a display so headless runs stay green.
    #[cfg(unix)]
    #[test]
    fn gl_view_full_drive() {
        use glutin::display::GlDisplay as _;
        use nerust_glwrap::CurrentContext;

        let _serial = crate::test_display::lock_display();
        let Some(display) = crate::test_display::TestDisplay::connect() else {
            eprintln!("SKIP GL context tests: no GL display");
            return;
        };
        let Some(_current) = display.make_pbuffer_current(64, 64) else {
            eprintln!("SKIP GL context tests: no GL context");
            return;
        };
        GlView::load_with(|name| {
            let proc_name = std::ffi::CString::new(name)
                .unwrap_or_else(|_| std::ffi::CString::new("").unwrap());
            display.display.get_proc_address(&proc_name)
        });
        {
            let use_vao = true;
            let profile = gl_test_profile(nerust_render_traits::VideoFrameFormat::Rgba, None);
            let mut view = GlView::new();
            view.use_vao(use_vao);
            view.on_load(&profile).unwrap();
            view.on_load(&profile).unwrap();
            view.on_update(&vec![0x80; 64 * 64 * 4], 64).unwrap();
            assert_eq!(view.draw_osd(64, 64).unwrap(), OsdOutcome::Skipped);
            view.set_osd_layer(Some(gl_test_layer()));
            assert_eq!(view.draw_osd(64, 64).unwrap(), OsdOutcome::Presented);
            assert_eq!(view.draw_osd(64, 64).unwrap(), OsdOutcome::Presented);
            view.on_update(&vec![0x40; 65 * 64 * 4], 65).unwrap();
            view.on_resize_with_strip(64, 64, nerust_render_traits::ReservedStrip::default())
                .unwrap();
            view.on_resize_with_strip(64, 64, nerust_render_traits::ReservedStrip::default())
                .unwrap();
            let strip = nerust_render_traits::ReservedStrip { top: 8, bottom: 8 };
            view.on_resize_with_strip(64, 64, strip).unwrap();
            let mut before = [0; 4];
            crate::test_display::TestDisplay::viewport_of(&mut before);
            assert_eq!(before[1], 8);
            view.set_osd_layer(Some(gl_test_layer()));
            assert_eq!(view.draw_osd(64, 64).unwrap(), OsdOutcome::Presented);
            let mut after = [0; 4];
            crate::test_display::TestDisplay::viewport_of(&mut after);
            assert_eq!(before, after);
            view.update_palette_texture(&[0x12; 256]).unwrap();
            view.set_osd_layer(None);
            assert_eq!(view.draw_osd(64, 64).unwrap(), OsdOutcome::Skipped);
            // Zero viewport skips without touching GL.
            view.on_resize_with_strip(0, 64, nerust_render_traits::ReservedStrip::default())
                .unwrap();
            let current = CurrentContext::assert_current(true);
            assert!(view.on_close(current).is_ok());
            assert!(view.on_close(current).is_ok());
            drop(view);
            let ntsc = vec![0xAA; 64 * 42 * 4].into_boxed_slice();
            let pal = gl_test_profile(nerust_render_traits::VideoFrameFormat::Palette, Some(ntsc));
            let mut pview = GlView::new();
            pview.use_vao(use_vao);
            pview.on_load(&pal).unwrap();
            pview.on_update(&vec![7; 64 * 64], 64).unwrap();
            assert_eq!(pview.draw_osd(64, 64).unwrap(), OsdOutcome::Skipped);
            assert!(pview.on_close(current).is_ok());
            assert!(pview.on_close(current).is_ok());
        }
    }

    /// OSD flip + NTSC byte order, proved by readback (Xvfb + llvmpipe
    /// in CI). Skips without a display so headless runs stay green.
    #[cfg(unix)]
    #[test]
    fn osd_flip_and_ntsc_readback() {
        use glutin::display::GlDisplay as _;
        use nerust_glwrap::raw::{delete_textures, gen_textures, get_tex_image};

        use crate::frame_textures::{configure_ntsc_texture, upload_texture_bytes, use_texture};

        let _serial = crate::test_display::lock_display();
        let Some(display) = crate::test_display::TestDisplay::connect() else {
            eprintln!("SKIP GL context tests: no GL display");
            return;
        };
        let Some(_current) = display.make_pbuffer_current(64, 64) else {
            eprintln!("SKIP GL context tests: no GL context");
            return;
        };
        GlView::load_with(|name| {
            let proc_name = std::ffi::CString::new(name)
                .unwrap_or_else(|_| std::ffi::CString::new("").unwrap());
            display.display.get_proc_address(&proc_name)
        });
        // 2x2 RGBA texture, zeroed.
        let mut names = [0u32; 2];
        gen_textures(2, names.as_mut_ptr()).unwrap();
        let osd_tex = names[0];
        upload_texture_bytes(0, osd_tex, 2, 2, gl::RGBA as i32, gl::RGBA, &[0; 16]).unwrap();
        // OSD top row (y=0) red, bottom row (y=1) green.
        let red = [255u8, 0, 0, 255, 255, 0, 0, 255];
        let green = [0u8, 255, 0, 255, 0, 255, 0, 255];
        GlView::blit_bytes(osd_tex, 2, 0, 0, 2, 1, &red).unwrap();
        GlView::blit_bytes(osd_tex, 2, 0, 1, 2, 1, &green).unwrap();
        let mut out = [0u8; 16];
        get_tex_image(
            gl::TEXTURE_2D,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            out.as_mut_ptr().cast(),
        )
        .unwrap();
        // GL row 0 is bottom-first: the flip put OSD-bottom (green) at row 0;
        // without it both rows would sit swapped.
        assert_eq!(&out[..8], &green);
        assert_eq!(&out[8..], &red);
        // Checked math agrees with the upload sites above.
        assert_eq!(crate::osd_overlay::checked_osd_gl_y(2, 0, 1).unwrap(), 1);
        assert_eq!(crate::osd_overlay::checked_osd_gl_y(2, 1, 1).unwrap(), 0);
        // NTSC kernel: GPU-upload bytes equal the CPU pack order
        // byte-for-byte (big-endian words `[R, G, B, A]`).
        let ntsc_tex = names[1];
        let words: Vec<u8> = (0u8..16).collect();
        let mut scratch = Vec::new();
        configure_ntsc_texture(1, ntsc_tex, 2, 2, &words, &mut scratch).unwrap();
        use_texture(1, ntsc_tex).unwrap();
        let mut ntsc_out = [0u8; 16];
        get_tex_image(
            gl::TEXTURE_2D,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            ntsc_out.as_mut_ptr().cast(),
        )
        .unwrap();
        assert_eq!(&ntsc_out, &words[..]);
        delete_textures(2, names.as_ptr()).unwrap();
    }
}
