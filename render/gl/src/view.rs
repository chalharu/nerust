use std::{os::raw::c_void, ptr, rc::Rc};

use gl::types::GLint;
use nerust_glwrap::{Shader, raw::*, vertex::*};
use nerust_render_ntsc::NTSC_TEXTURE_HEIGHT;
use nerust_render_traits::{
    ReservedStrip, VideoFrameFormat, VideoRenderProfile, renderer::OsdLayer,
};

use crate::{mat4::Mat4, vec2d::Vec2D, vertex_data::VertexData};

/// View failures: GL errors, shader errors, invariant and frame faults.
/// Translated to a warned `RenderResult::Error` at the render boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ViewError {
    #[error(transparent)]
    Gl(#[from] nerust_glwrap::GlError),
    #[error(transparent)]
    Program(#[from] nerust_glwrap::ShaderError),
    #[error("GL view misses required uniform `{name}`")]
    MissingUniform { name: &'static str },
    #[error("GL view misses required attribute `{name}`")]
    MissingAttribute { name: &'static str },
    #[error("GL view has no shader program")]
    NoProgram,
    #[error("GL view has no vertex array")]
    NoVertexArray,
    #[error("GL view has no vertex buffer")]
    NoVertexBuffer,
    #[error("{what}: need {need}, have {have}")]
    Frame {
        what: &'static str,
        need: usize,
        have: usize,
    },
}

/// Outcome of one OSD draw. Transient upload failures propagate as
/// `Err` (retried next frame); only the give-up transition reports
/// `Dropped`, so the renderer logs the demotion exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsdOutcome {
    Presented,
    Dropped,
}

/// Bounded OSD retry: transient blips survive, dead drivers drop.
/// Pure counter; the renderer owns warn-once logging (see `OsdOutcome`).
#[derive(Debug)]
struct RetryBudget {
    max: u8,
    fails: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryDecision {
    Retry,
    DropLayer,
}

impl RetryBudget {
    fn new(max: u8) -> Self {
        Self { max, fails: 0 }
    }

    fn record_failure(&mut self) -> RetryDecision {
        self.fails = self.fails.saturating_add(1);
        if self.fails >= self.max {
            RetryDecision::DropLayer
        } else {
            RetryDecision::Retry
        }
    }

    fn reset(&mut self) {
        self.fails = 0;
    }
}

/// Shared OSD equality lives on [`OsdLayer`] (single source).
fn osd_layers_equal(a: Option<&OsdLayer>, b: Option<&OsdLayer>) -> bool {
    OsdLayer::same_option(a, b)
}

/// Convert a GL call result into a view error for `?` propagation.
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

/// Upload attempts before a failing OSD layer is dropped. Transient
/// blips survive; a dead driver stops costing per-frame work.
const MAX_OSD_UPLOAD_RETRIES: u8 = 3;

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
}

/// Surface memo: last viewport applied; skips redundant GL state writes.
#[derive(Debug, Default)]
struct SurfaceState {
    last_resize: Option<(i32, i32, ReservedStrip)>,
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
        }
    }
}

#[derive(Debug)]
pub struct GlView {
    frame: FrameResources,
    osd: OsdResources,
    surface: SurfaceState,
}

/// Validated OSD blit rect: position, size, row stride, and byte
/// need. Pure so the corrupt-layer policy stays unit-testable.
/// Required uniform location. Optimized-out names are a broken program
/// here (unlike optional sampler uploads): fail typed instead of `-1`.
fn required_uniform(shader: &nerust_glwrap::Shader, key: &'static str) -> Result<i32, ViewError> {
    shader
        .uniform_opt(key)
        .ok_or(ViewError::MissingUniform { name: key })
}

fn checked_osd_rect(
    layer: &OsdLayer,
    surface_w: i32,
    surface_h: i32,
) -> Option<(i32, i32, i32, i32, usize, usize)> {
    let (w, h) = (
        usize::try_from(layer.width).ok()?,
        usize::try_from(layer.height).ok()?,
    );
    let stride = w.checked_mul(4)?;
    let need = stride.checked_mul(h)?;
    if need == 0 || layer.rgba.len() < need {
        return None;
    }
    let (lx, ly, lw, lh) = (layer.x as i32, layer.y as i32, w as i32, h as i32);
    if lx < 0 || ly < 0 || lx + lw > surface_w || ly + lh > surface_h {
        return None;
    }
    Some((lx, ly, lw, lh, stride, need))
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

    /// Frame upload target plus the placeholder palette texture.
    /// NTSC kernels and vertex geometry load separately below.
    fn load_frame_textures(
        &mut self,
        render_profile: &VideoRenderProfile,
    ) -> Result<(), ViewError> {
        // Palette モードでは frame data は source_logical_size、RGBA では logical_size
        let frame_size = if self.frame.is_palette_format {
            render_profile.source_logical_size
        } else {
            render_profile.logical_size
        };
        self.frame.logical_width = frame_size.width as i32;
        self.frame.logical_height = frame_size.height as i32;
        let bpp: usize = if self.frame.is_palette_format { 1 } else { 4 };
        // frame texture (palette 時は R8、RGBA 時は RGBA)
        let (internal_fmt, data_fmt) = if self.frame.is_palette_format {
            (gl::R8 as GLint, gl::RED)
        } else {
            (gl::RGBA as GLint, gl::RGBA)
        };
        let mut tex_names = [0; 1];
        ok_gl(gen_textures(1, tex_names.as_mut_ptr()))?;
        self.frame.frame_texture = tex_names[0];
        configure_frame_texture(
            0,
            self.frame.frame_texture,
            frame_size.width,
            frame_size.height,
            internal_fmt,
            data_fmt,
            allocate(frame_size.width * frame_size.height * bpp).as_ref(),
        )?;

        // palette texture: 常に 64x1 RGBA8、ゼロ初期化。
        // 実データは render 時に FrameBuffer.palette_as_rgba8() から同期される。
        if self.frame.is_palette_format {
            self.frame.palette_width = 64;
            self.frame.palette_height = 1;
            let palette_data =
                vec![
                    0u8;
                    self.frame.palette_width as usize * self.frame.palette_height as usize * 4
                ];
            let mut pal_names = [0; 1];
            ok_gl(gen_textures(1, pal_names.as_mut_ptr()))?;
            self.frame.palette_texture = pal_names[0];
            configure_frame_texture(
                1,
                self.frame.palette_texture,
                self.frame.palette_width as usize,
                self.frame.palette_height as usize,
                gl::RGBA as GLint,
                gl::RGBA,
                &palette_data,
            )?;
        }
        Ok(())
    }

    /// Palette sampler binding plus the NTSC kernel texture and its
    /// uniforms. The textures themselves come from `load_frame_textures`.
    fn load_palette_ntsc(
        &mut self,
        shader: &nerust_glwrap::Shader,
        render_profile: &VideoRenderProfile,
    ) -> Result<(), ViewError> {
        ok_gl(uniform_1i(required_uniform(shader, "palette_texture")?, 1))?;

        // NTSC texture
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
            ok_gl(uniform_1i(required_uniform(shader, "ntsc_texture")?, 2))?;
        } else {
            // ダミー (sampler2D 未バインド防止)
            let mut ntsc_names = [0; 1];
            ok_gl(gen_textures(1, ntsc_names.as_mut_ptr()))?;
            self.frame.ntsc_texture = ntsc_names[0];
            let ntsc_height = NTSC_TEXTURE_HEIGHT as usize;
            let dummy = vec![0u8; 64 * ntsc_height * 4];
            configure_frame_texture(
                2,
                self.frame.ntsc_texture,
                64,
                ntsc_height,
                gl::RGBA as GLint,
                gl::RGBA,
                &dummy,
            )?;
            ok_gl(uniform_1i(required_uniform(shader, "ntsc_texture")?, 2))?;
        }

        ok_gl(uniform_2f(
            required_uniform(shader, "source_size")?,
            render_profile.source_logical_size.width as f32,
            render_profile.source_logical_size.height as f32,
        ))?;
        ok_gl(uniform_2f(
            required_uniform(shader, "output_size")?,
            ntsc_size.width as f32,
            ntsc_size.height as f32,
        ))?;
        ok_gl(uniform_1i(
            required_uniform(shader, "ntsc_enabled")?,
            self.frame.ntsc_enabled as i32,
        ))?;
        Ok(())
    }

    /// Quad geometry plus the game-pipeline uniforms.
    /// Attribute names are resolved before the GL-typed builder:
    /// a missing name is a broken program, and 0 would alias a valid
    /// attribute.
    fn init_geometry(&mut self, shader: &nerust_glwrap::Shader) -> Result<(), ViewError> {
        let vertex_data: [VertexData; 4] = [
            VertexData::new(Vec2D::new(-1.0, 1.0), Vec2D::new(0.0, 0.0)),
            VertexData::new(Vec2D::new(-1.0, -1.0), Vec2D::new(0.0, 1.0)),
            VertexData::new(Vec2D::new(1.0, 1.0), Vec2D::new(1.0, 0.0)),
            VertexData::new(Vec2D::new(1.0, -1.0), Vec2D::new(1.0, 1.0)),
        ];

        let vbo = Rc::new(ok_gl(VertexBuffer::from_slice(&vertex_data))?);
        // Resolve before the GL-typed builder closure: a missing name
        // is a broken program, and 0 would alias a valid attribute.
        let position = shader
            .attribute_opt("position")
            .ok_or(ViewError::MissingAttribute { name: "position" })?;
        let uv = shader
            .attribute_opt("uv")
            .ok_or(ViewError::MissingAttribute { name: "uv" })?;
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

        ok_gl(uniform_matrix_4fv(
            required_uniform(shader, "unif_matrix")?,
            1,
            gl::FALSE,
            Mat4::identity().as_ptr(),
        ))?;
        ok_gl(uniform_1i(required_uniform(shader, "frame_texture")?, 0))?;
        Ok(())
    }

    pub fn on_load(&mut self, render_profile: &VideoRenderProfile) -> Result<(), ViewError> {
        // Reuse must not leak names: tear down while current first.
        let _ = self.on_close();
        debug_assert_eq!(self.frame.frame_texture, 0);
        self.surface.last_resize = None;
        self.frame.is_palette_format = render_profile.frame_format == VideoFrameFormat::Palette;
        let version = gl_string(gl::VERSION);
        let is_gles = is_gles_context(version.as_deref());
        // Stride-padded rows upload as-is via UNPACK_ROW_LENGTH
        // (desktop GL, or GLES3+); older ES repacks instead.
        self.frame.unpack_row_length = supports_unpack_row_length(version.as_deref());
        let shader = compile_shader_program(self.frame.is_palette_format, is_gles)?;
        ok_gl(shader.use_program())?;
        ok_gl(clear_color(0.0, 0.0, 0.0, 1.0))?;

        ok_gl(pixel_storei(gl::UNPACK_ALIGNMENT, 1))?;

        self.load_frame_textures(render_profile)?;

        // Palette and NTSC kernels plus their uniforms; the textures
        // themselves come from `load_frame_textures`.
        if self.frame.is_palette_format {
            self.load_palette_ntsc(&shader, render_profile)?;
        }

        self.init_geometry(&shader)?;
        self.frame.shader = Some(shader);
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
        if configure_frame_texture(
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
                    self.osd.osd_shader = Some(shader);
                    return;
                }
                Err(err) => log::warn!("osd shader {name} unavailable: {err}"),
            }
        }
    }

    /// Stage an OSD layer for the next present. `None` clears.
    /// Equal layers skip; new content resets the failure budget.
    pub fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        if osd_layers_equal(self.osd.osd_layer.as_ref(), layer.as_ref()) {
            return;
        }
        self.osd.osd_layer = layer;
        self.osd.osd_dirty = true;
        self.osd.osd_retry.reset();
    }

    /// Composite the staged layer over the game image, if any (best-effort).
    /// A GL failure drops the overlay, never the frame.
    pub fn draw_osd(&mut self, surface_w: i32, surface_h: i32) -> Result<OsdOutcome, ViewError> {
        // Bounded retry: transient errors re-upload next frame, but a
        // dead driver gives up instead of re-allocating every frame.
        if self.osd.osd_dirty {
            match self.sync_osd_texture(surface_w, surface_h) {
                Ok(()) => {
                    self.osd.osd_dirty = false;
                    self.osd.osd_retry.reset();
                }
                Err(e) => {
                    if self.note_osd_upload_failure().is_some() {
                        return Ok(OsdOutcome::Dropped);
                    }
                    return Err(e);
                }
            }
        }
        let (Some(shader), Some(_)) = (self.osd.osd_shader.as_ref(), self.osd.osd_layer.as_ref())
        else {
            return Ok(OsdOutcome::Presented);
        };
        ok_gl(shader.use_program())?;
        ok_gl(uniform_matrix_4fv(
            required_uniform(shader, "unif_matrix")?,
            1,
            gl::FALSE,
            Mat4::identity().as_ptr(),
        ))?;
        ok_gl(uniform_1i(required_uniform(shader, "osd_texture")?, 0))?;
        use_texture(0, self.osd.osd_texture)?;
        if self.frame.use_vao {
            let Some(vba) = self.frame.vba.as_ref() else {
                return Err(ViewError::NoVertexArray);
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.frame.vbo.as_ref() else {
                return Err(ViewError::NoVertexBuffer);
            };
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, vbo.id))?;
        }
        // The game viewport (letterboxed by `on_resize_with_strip`) is
        // global GL state: save it, composite fullscreen, restore it so
        // the next game frame keeps its strips without a resize event.
        let mut saved = [0; 4];
        ok_gl(get_viewport(&mut saved))?;
        ok_gl(viewport(0, 0, surface_w, surface_h))?;
        ok_gl(enable(gl::BLEND))?;
        ok_gl(blend_func(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA))?;
        let drawn = ok_gl(draw_arrays(gl::TRIANGLE_STRIP, 0, 4));
        ok_gl(viewport(saved[0], saved[1], saved[2], saved[3]))?;
        ok_gl(disable(gl::BLEND))?;
        drawn?;
        Ok(OsdOutcome::Presented)
    }

    /// Record one failed OSD upload (pure state; GL stays out).
    /// Returns `Some` on the transition that drops the stale layer.
    fn note_osd_upload_failure(&mut self) -> Option<()> {
        self.osd.osd_layer.as_ref()?;
        match self.osd.osd_retry.record_failure() {
            RetryDecision::Retry => None,
            RetryDecision::DropLayer => {
                // No ghost composite, and the next identical push
                // re-stages via the equality gate.
                self.osd.osd_layer = None;
                self.osd.osd_dirty = false;
                Some(())
            }
        }
    }

    /// Upload the staged layer into the full-surface texture (draw path only binds).
    /// Best-effort: failures keep the previous texture, never panic.
    fn sync_osd_texture(&mut self, surface_w: i32, surface_h: i32) -> Result<(), ViewError> {
        if self.osd.osd_layer.is_none() {
            // Wipe the last rect so no ghost remains. Retain on failure:
            // the retry must know which rect still needs erasing.
            if let Some((px, py, pw, ph)) = self.osd.osd_prev {
                let texture = self.osd.osd_texture;
                self.clear_rect(texture, px, py, pw, ph)?;
                self.osd.osd_prev = None;
            }
            return Ok(());
        }
        if self.osd.osd_texture == 0 {
            return Ok(());
        }
        // Full-surface texture: placement rides on texture coordinates
        // instead of small viewports (driver-quirky on some backends).
        if self.osd.osd_size != (surface_w, surface_h) {
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
            self.osd.osd_size = (surface_w, surface_h);
        }
        // Validate before any allocation: a corrupt layer must drop
        // loudly, never OOM the renderer thread on unchecked math.
        // Pure check first, so no layer borrow crosses the mutations.
        let staged = match self.osd.osd_layer.as_ref() {
            Some(layer) => match checked_osd_rect(layer, surface_w, surface_h) {
                Some(rect) => rect,
                None => {
                    log::warn!("GlView: dropping invalid OSD layer");
                    self.osd.osd_layer = None;
                    self.osd.osd_dirty = false;
                    return Ok(());
                }
            },
            None => return Ok(()),
        };
        let (lx, ly, lw, lh, stride, need) = staged;
        // Clear the previous rect so moved/shrunk layers leave no ghosts.
        let rect = (lx, ly, lw, lh);
        if self.osd.osd_prev != Some(rect) {
            if let Some((px, py, pw, ph)) = self.osd.osd_prev {
                let texture = self.osd.osd_texture;
                self.clear_rect(texture, px, py, pw, ph)?;
            }
            self.osd.osd_prev = Some(rect);
        }
        if self.osd.osd_upload_scratch.len() < need {
            self.osd.osd_upload_scratch.resize(need, 0);
        }
        let Some(layer) = self.osd.osd_layer.as_ref() else {
            return Ok(());
        };
        {
            let scratch = &mut self.osd.osd_upload_scratch[..need];
            scratch.fill(0);
            for (index, row) in layer.rgba.chunks_exact(stride).enumerate() {
                scratch[index * stride..index * stride + stride].copy_from_slice(row);
            }
        }
        let texture = self.osd.osd_texture;
        Self::blit_bytes(
            texture,
            lx,
            ly,
            lw,
            lh,
            &self.osd.osd_upload_scratch[..need],
        )?;
        Ok(())
    }

    /// Clear a subrect to transparent using the reusable zero scratch.
    fn clear_rect(
        &mut self,
        texture: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Result<(), ViewError> {
        let need = w.max(1) as usize * h.max(1) as usize * 4;
        if self.osd.osd_clear_scratch.len() < need {
            self.osd.osd_clear_scratch.resize(need, 0);
        } else {
            self.osd.osd_clear_scratch[..need].fill(0);
        }
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
            self.osd.osd_clear_scratch.as_ptr().cast(),
        ))?;
        Ok(())
    }

    /// Upload RGBA rows into a texture subrect (best-effort).
    /// A GL failure degrades to a debug log instead of killing the frame.
    fn blit_bytes(
        texture: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        bytes: &[u8],
    ) -> Result<(), ViewError> {
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
        ok_gl(active_texture(gl::TEXTURE0))?;
        Ok(())
    }

    pub fn on_update(&self, screen: &[u8], row_length: i32) -> Result<(), ViewError> {
        let screen_ptr = screen.as_ptr().cast();
        let Some(shader) = self.frame.shader.as_ref() else {
            return Err(ViewError::NoProgram);
        };
        ok_gl(shader.use_program())?;

        use_texture(0, self.frame.frame_texture)?;

        // Padded rows upload as-is; row length is global pixel-store
        // state, so restore the default right after the upload.
        let padded = row_length != self.frame.logical_width;
        if padded {
            ok_gl(pixel_storei(gl::UNPACK_ROW_LENGTH, row_length))?;
        }

        if self.frame.is_palette_format {
            use_texture(1, self.frame.palette_texture)?;
            use_texture(2, self.frame.ntsc_texture)?;
            // frame texture (palette index を R8 → GL_RED で upload)
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
                return Err(ViewError::NoVertexArray);
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.frame.vbo.as_ref() else {
                return Err(ViewError::NoVertexBuffer);
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
        if padded {
            ok_gl(pixel_storei(gl::UNPACK_ROW_LENGTH, 0))?;
        }
        ok_gl(draw_arrays(gl::TRIANGLE_STRIP, 0, 4))?;
        Ok(())
    }

    /// Whether the context accepts UNPACK_ROW_LENGTH (no CPU repack).
    pub fn supports_unpack_row_length(&self) -> bool {
        self.frame.unpack_row_length
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
        // Skip redundant GL state writes; no program/VAO caching here.
        if self.surface.last_resize == Some((viewport_width, viewport_height, strip)) {
            return Ok(());
        }
        let height_i = viewport_height.max(0);
        let top = strip.top.min(height_i as u32) as i32;
        let bottom = strip.bottom.min(height_i as u32 - top as u32) as i32;
        let area_height = (height_i - top - bottom).max(1);
        let window_aspect = viewport_width as f32 / area_height as f32;
        let content_aspect = self.frame.logical_width as f32 / self.frame.logical_height as f32;

        let (scale_x, scale_y) = if window_aspect > content_aspect {
            // Window is wider than content → letterbox (black bars on sides)
            (content_aspect / window_aspect, 1.0)
        } else {
            // Window is taller than content → pillarbox (black bars on top/bottom)
            (1.0, window_aspect / content_aspect)
        };

        let Some(shader) = self.frame.shader.as_ref() else {
            return Err(ViewError::NoProgram);
        };
        ok_gl(shader.use_program())?;
        if self.frame.use_vao {
            let Some(vba) = self.frame.vba.as_ref() else {
                return Err(ViewError::NoVertexArray);
            };
            ok_gl(vba.bind_vao(|_vac| Ok(())))?;
        } else {
            let Some(vbo) = self.frame.vbo.as_ref() else {
                return Err(ViewError::NoVertexBuffer);
            };
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, vbo.id))?;
        }
        ok_gl(viewport(0, bottom, viewport_width, area_height))?;
        ok_gl(uniform_matrix_4fv(
            required_uniform(shader, "unif_matrix")?,
            1,
            gl::FALSE,
            Mat4::scale(scale_x, scale_y, 1.0).as_ptr(),
        ))?;
        self.surface.last_resize = Some((viewport_width, viewport_height, strip));
        Ok(())
    }

    /// Explicit teardown while the context is current (fact, no warn).
    /// Returns true when a GL delete failed; the renderer logs it.
    pub fn on_close(&mut self) -> bool {
        let mut failed = false;
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
        // Teardown best-effort: a failed delete must not panic the
        // profile rebuild that called here.
        if !ids.is_empty()
            && let Err(e) = delete_textures(ids.len() as i32, ids.as_ptr())
        {
            log::debug!("GL view teardown failed: {e}");
            failed = true;
        }
        // Zero what was deleted: a second `on_close` without an
        // intervening `on_load` must not double-delete names.
        self.frame.frame_texture = 0;
        self.frame.palette_texture = 0;
        self.frame.ntsc_texture = 0;
        self.osd.osd_texture = 0;
        self.osd.osd_size = (0, 0);
        self.osd.osd_prev = None;
        self.surface.last_resize = None;
        if let Some(mut shader) = self.frame.shader.take()
            && shader.destroy_if_current(true)
        {
            failed = true;
        }
        if let Some(mut shader) = self.osd.osd_shader.take()
            && shader.destroy_if_current(true)
        {
            failed = true;
        }
        if let Some(mut vba) = self.frame.vba.take()
            && vba.destroy_if_current(true)
        {
            failed = true;
        }
        if let Some(vbo) = self.frame.vbo.take()
            && vbo.destroy_if_current(true)
        {
            failed = true;
        }
        failed
    }
}

/// NTSC kernel texture (RGBA8)。packed_ntsc_rgba8 は big-endian u32 を RGBA8 に変換。
/// 実際の kernel entry stride を data 長から計算して texture 高さとする。
fn configure_ntsc_texture(
    unit: u32,
    texture: u32,
    width: usize,
    height: usize,
    be_data: &[u8],
    scratch: &mut Vec<u8>,
) -> Result<(), ViewError> {
    // be_data は PALETTE_TEXTURE_WIDTH × entry_stride × 4 bytes
    let entry_count = be_data.len() / (width * 4);
    let actual_height = entry_count.max(height);
    let need = width * actual_height * 4;
    if scratch.len() < need {
        scratch.resize(need, 0);
    }
    let rgba = &mut scratch[..need];
    for row in 0..actual_height {
        for col in 0..width {
            let base = (row * width + col) * 4;
            let dst = (row * width + col) * 4;
            if base + 4 <= be_data.len() {
                rgba[dst..dst + 4].copy_from_slice(&be_data[base..base + 4]);
            } else {
                rgba[dst..dst + 4].copy_from_slice(&[0, 0, 0, 0]);
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

    Err(ViewError::NoProgram)
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
        GlView, OsdOutcome, RetryBudget, RetryDecision, build_glsl_source, checked_osd_rect,
        compose_glsl_source, is_gles_context, ok_gl, osd_layers_equal, supports_unpack_row_length,
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
        assert_eq!(checked_osd_rect(&inside, 64, 64), Some((4, 6, 2, 1, 8, 8)));
        // Short data, zero area, and out-of-bounds all refuse.
        let short = OsdLayer {
            rgba: vec![1; 7],
            ..inside.clone()
        };
        assert_eq!(checked_osd_rect(&short, 64, 64), None);
        let flat = OsdLayer {
            height: 0,
            rgba: vec![],
            ..inside.clone()
        };
        assert_eq!(checked_osd_rect(&flat, 64, 64), None);
        let outside = OsdLayer {
            x: 63,
            ..inside.clone()
        };
        assert_eq!(checked_osd_rect(&outside, 64, 64), None);
        // Overflow dimensions refuse instead of wrapping.
        let huge = OsdLayer {
            width: u32::MAX,
            height: u32::MAX,
            rgba: vec![],
            ..inside.clone()
        };
        assert_eq!(checked_osd_rect(&huge, i32::MAX, i32::MAX), None);
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
        // Simulated upload consumed the flag; re-staging the same
        // bytes must not re-dirty (no per-frame re-upload).
        view.osd.osd_dirty = false;
        view.set_osd_layer(Some(layer));
        assert!(!view.osd.osd_dirty);
        // Clearing is a change: the wipe must run.
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
        assert!(osd_layers_equal(Some(&base), Some(&base.clone())));
        assert!(!osd_layers_equal(
            Some(&base),
            Some(&OsdLayer {
                x: 7,
                ..base.clone()
            })
        ));
        assert!(!osd_layers_equal(
            Some(&base),
            Some(&OsdLayer {
                rgba: vec![8; 8],
                ..base.clone()
            })
        ));
        assert!(!osd_layers_equal(
            Some(&base),
            Some(&OsdLayer {
                rgba: vec![9; 4],
                width: 1,
                ..base.clone()
            })
        ));
        assert!(osd_layers_equal(None, None));
        assert!(!osd_layers_equal(Some(&base), None));
        assert!(!osd_layers_equal(None, Some(&base)));
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
        let err = super::ViewError::Frame {
            what: "frame buffer too small",
            need: 10,
            have: 4,
        };
        assert!(err.to_string().contains("10"));
        let internal = super::ViewError::NoProgram;
        assert!(internal.to_string().contains("shader"));
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
        assert!(view.note_osd_upload_failure().is_none());
        assert!(view.osd.osd_layer.is_some());
        assert!(view.osd.osd_dirty);
        assert!(view.note_osd_upload_failure().is_none());
        assert!(view.osd.osd_layer.is_some());
        // Third strike: the stale layer drops, no ghost composite.
        assert!(view.note_osd_upload_failure().is_some());
        assert!(view.osd.osd_layer.is_none());
        assert!(!view.osd.osd_dirty);
        // Saturated: further failures stay dropped without re-arming.
        assert!(view.note_osd_upload_failure().is_none());
        assert!(view.osd.osd_layer.is_none());
        assert!(!view.osd.osd_dirty);
    }

    #[test]
    fn upload_failure_counter_saturates() {
        let mut view = staged_view();
        view.osd.osd_retry.fails = u8::MAX;
        assert!(view.note_osd_upload_failure().is_some());
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
            let proc_name = std::ffi::CString::new(name).expect("GL proc name has no interior nul");
            display.display.get_proc_address(&proc_name)
        });
        // Production always drives the VAO path (`fresh_view` sets it);
        // the legacy client-array path needs no cover here.
        {
            let use_vao = true;
            // RGBA drive: upload, draw, OSD stage, padded rows, wipe, teardown.
            let profile = gl_test_profile(nerust_render_traits::VideoFrameFormat::Rgba, None);
            let mut view = GlView::new();
            view.use_vao(use_vao);
            view.on_load(&profile).unwrap();
            // Reuse after load must not leak: on_load tears down first.
            view.on_load(&profile).unwrap();
            view.on_update(&vec![0x80; 64 * 64 * 4], 64).unwrap();
            assert_eq!(view.draw_osd(64, 64).unwrap(), OsdOutcome::Presented);
            view.set_osd_layer(Some(gl_test_layer()));
            assert_eq!(view.draw_osd(64, 64).unwrap(), OsdOutcome::Presented);
            // Steady OSD upload reuses staging: identical second draw matches.
            assert_eq!(view.draw_osd(64, 64).unwrap(), OsdOutcome::Presented);
            view.on_update(&vec![0x40; 65 * 64 * 4], 65).unwrap();
            view.on_resize_with_strip(64, 64, nerust_render_traits::ReservedStrip::default())
                .unwrap();
            // Gated resize: identical second call skips GL work.
            view.on_resize_with_strip(64, 64, nerust_render_traits::ReservedStrip::default())
                .unwrap();
            // OSD composite widens the viewport but must restore the
            // letterboxed game viewport: the next game frame draws
            // without a resize event in between. A real strip makes the
            // game viewport differ from fullscreen, so the check bites.
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
            assert_eq!(view.draw_osd(64, 64).unwrap(), OsdOutcome::Presented);
            assert!(!view.on_close());
            // Double close is safe: second call finds no names.
            assert!(!view.on_close());
            drop(view);
            // Palette drive with the NTSC kernel path enabled.
            let ntsc = vec![0xAA; 64 * 42 * 4].into_boxed_slice();
            let pal = gl_test_profile(nerust_render_traits::VideoFrameFormat::Palette, Some(ntsc));
            let mut pview = GlView::new();
            pview.use_vao(use_vao);
            pview.on_load(&pal).unwrap();
            pview.on_update(&vec![7; 64 * 64], 64).unwrap();
            assert_eq!(pview.draw_osd(64, 64).unwrap(), OsdOutcome::Presented);
            assert!(!pview.on_close());
            assert!(!pview.on_close());
        }
    }
}
