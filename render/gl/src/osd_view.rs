//! OSD overlay view: layer staging, upload, and composite.
//! Owns the overlay domain; the frame pipeline only lends binding helpers.

use gl::types::GLint;
use nerust_glwrap::bindings as gl;
use nerust_glwrap::{
    Current, DeletionPolicy, Shader,
    raw::{
        active_texture, bind_buffer, bind_vertex_array, blend_func_separate, check_frame,
        delete_textures, disable, draw_arrays, enable, gen_textures, get_integer_u32, get_viewport,
        is_enabled, tex_sub_image_2d, uniform_1i, uniform_matrix_4fv, use_program, viewport,
    },
};
use nerust_render_traits::renderer::{OsdLayer, UploadBudget};

use crate::upload::StagingBuf;

use crate::error::ViewError;
use crate::frame_textures::use_texture;
use crate::frame_view::{FrameView, ok_gl};
use crate::mat4::Mat4;
use crate::osd_overlay::{OSDRect, checked_clear_need, checked_osd_rect, checked_osd_y};
use crate::surface_view::LeakedName;

/// Outcome of one OSD draw. Transient failures propagate as `Err` (retried);
/// only the give-up transition reports `Dropped`; `Skipped` means nothing staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsdOutcome {
    Presented,
    Skipped,
    Dropped,
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

impl ViewportGuard {
    /// Explicit restore: reports failure instead of debug-logging it.
    /// Consumed, so the backstop drop below never double-restores.
    fn restore(self) -> Result<(), ViewError> {
        let [x, y, w, h] = self.saved;
        let result = ok_gl(viewport(x, y, w, h));
        std::mem::forget(self);
        result
    }
}

impl Drop for ViewportGuard {
    fn drop(&mut self) {
        // Panic backstop only: every normal path restores explicitly.
        if std::thread::panicking() {
            let [x, y, w, h] = self.saved;
            if viewport(x, y, w, h).is_err() {
                log::debug!("GlView: viewport restore failed");
            }
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
        ok_gl(blend_func_separate(
            gl::SRC_ALPHA,
            gl::ONE_MINUS_SRC_ALPHA,
            gl::ONE,
            gl::ONE_MINUS_SRC_ALPHA,
        ))?;
        Ok(Self {
            was_enabled,
            src_rgb,
            dst_rgb,
            src_alpha,
            dst_alpha,
        })
    }
}

impl BlendGuard {
    /// Explicit restore: factors first, then the enable flag, reporting failure.
    fn restore(self) -> Result<(), ViewError> {
        // Restore factors first, then the enable flag: pre-enabled BLEND
        // survives, pre-disabled never leaks on.
        let result = (|| {
            ok_gl(blend_func_separate(
                self.src_rgb,
                self.dst_rgb,
                self.src_alpha,
                self.dst_alpha,
            ))?;
            if !self.was_enabled {
                ok_gl(disable(gl::BLEND))?;
            }
            Ok(())
        })();
        std::mem::forget(self);
        result
    }
}

impl Drop for BlendGuard {
    fn drop(&mut self) {
        // Panic backstop only: every normal path restores explicitly.
        if std::thread::panicking() {
            if blend_func_separate(self.src_rgb, self.dst_rgb, self.src_alpha, self.dst_alpha)
                .is_err()
            {
                log::debug!("GlView: blend factor restore failed");
            }
            if !self.was_enabled && disable(gl::BLEND).is_err() {
                log::debug!("GlView: blend disable restore failed");
            }
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

impl ProgramGuard {
    /// Explicit restore: reports failure instead of debug-logging it.
    fn restore(self) -> Result<(), ViewError> {
        // No raw `gl::` calls: wrappers drain per call and report the
        // GL error chain on failure.
        let result = (|| {
            ok_gl(active_texture(self.active_texture))?;
            ok_gl(bind_buffer(gl::ARRAY_BUFFER, self.array_buffer))?;
            ok_gl(bind_vertex_array(self.vao_binding))?;
            ok_gl(use_program(self.program))?;
            Ok(())
        })();
        std::mem::forget(self);
        result
    }
}

impl Drop for ProgramGuard {
    fn drop(&mut self) {
        // Panic backstop only: every normal path restores explicitly.
        if std::thread::panicking() {
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
}

/// Viewport + blend + program save for the OSD composite.
/// Explicit `restore` reports failure; drops are panic backstops only.
struct CompositeGuard {
    viewport: ViewportGuard,
    blend: BlendGuard,
    program: ProgramGuard,
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
        // Staged build: a later failure rolls back the earlier guards, so a
        // half-built composite never leaks viewport or blend state.
        let blend = match BlendGuard::enable_src_alpha() {
            Ok(blend) => blend,
            Err(error) => {
                if let Err(rollback) = viewport.restore() {
                    log::debug!("GlView: OSD viewport rollback failed: {rollback:?}");
                }
                return Err(error);
            }
        };
        let program = match ProgramGuard::save() {
            Ok(program) => program,
            Err(error) => {
                if let Err(rollback) = blend.restore() {
                    log::debug!("GlView: OSD blend rollback failed: {rollback:?}");
                }
                if let Err(rollback) = viewport.restore() {
                    log::debug!("GlView: OSD viewport rollback failed: {rollback:?}");
                }
                return Err(error);
            }
        };
        Ok(Self {
            viewport,
            blend,
            program,
        })
    }

    /// Restore game state element-wise: every restore runs even when an
    /// earlier one fails; the first failure is reported.
    fn restore(self) -> Result<(), ViewError> {
        let Self {
            viewport,
            blend,
            program,
        } = self;
        program
            .restore()
            .and(blend.restore())
            .and(viewport.restore())
    }

    fn saved_viewport(&self) -> [i32; 4] {
        self.viewport.saved_viewport()
    }
}

/// OSD pipeline resources: layer, program, retry, staging. One owner for
/// the overlay domain; the frame pipeline never touches these.
#[derive(Debug)]
pub(crate) struct OsdView {
    pub(crate) osd_texture: u32,
    pub(crate) osd_size: (i32, i32),
    pub(crate) osd_prev: Option<(i32, i32, i32, i32)>,
    pub(crate) osd_dirty: bool,
    /// Bounded retry for the staged layer (shared `UploadBudget` value).
    pub(crate) osd_retry: UploadBudget,
    pub(crate) osd_shader: Option<Shader>,
    pub(crate) osd_layer: Option<OsdLayer>,
    /// Grow-only zeros reused for OSD clears (no steady allocs).
    pub(crate) osd_clear_scratch: StagingBuf,
    /// Null-allocated surface store is undefined until the first full clear.
    /// Set on resize; the next staged composite clears once, then sets it.
    /// Resizes with no staged layer never pay the fullscreen memset.
    osd_surface_clean: bool,
    pub(crate) osd_matrix: Option<GLint>,
    pub(crate) last_osd_matrix: Option<[f32; 16]>,
    /// Sampler uniform resolved once in `init_osd` (not per frame).
    pub(crate) osd_texture_loc: Option<GLint>,
    /// Last uploaded sampler unit; unit 0 is the only value ever
    /// bound, so the second and later composites skip the upload.
    pub(crate) last_osd_sampler: Option<i32>,
}

impl Default for OsdView {
    fn default() -> Self {
        Self {
            osd_texture: 0,
            osd_size: (0, 0),
            osd_prev: None,
            osd_dirty: false,
            osd_retry: UploadBudget::new(
                nerust_render_traits::renderer::DEFAULT_OSD_UPLOAD_RETRIES,
            ),
            osd_shader: None,
            osd_layer: None,
            osd_clear_scratch: StagingBuf::new(),
            osd_surface_clean: true,
            osd_matrix: None,
            last_osd_matrix: None,
            osd_texture_loc: None,
            last_osd_sampler: None,
        }
    }
}

impl OsdView {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Replace the upload budget (tests, custom backoff).
    pub fn set_osd_policy(&mut self, policy: UploadBudget) {
        self.osd_retry = policy;
    }

    pub fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        if OsdLayer::same_option(self.osd_layer.as_ref(), layer.as_ref()) {
            return;
        }
        self.osd_layer = layer;
        self.osd_dirty = true;
        self.osd_retry.reset();
    }

    /// Use the OSD program, skipping the GL call when already current.
    /// Exactly one program is current at a time; the guard restores it after.
    fn use_osd_program(&mut self, frame: &mut FrameView) -> Result<(), ViewError> {
        let Some(shader) = self.osd_shader.as_ref() else {
            return Err(ViewError::NoProgram { program: "osd" });
        };
        let id = shader.id();
        if frame.current_program() != Some(id) {
            frame.set_current_program(None);
            ok_gl(shader.use_program())?;
            ok_gl(check_frame())?;
            frame.set_current_program(Some(id));
        }
        Ok(())
    }

    /// Overlay composite over the game image (best-effort; never drops the frame).
    /// `frame` lends binding; the applied viewport returns for the surface memo.
    pub(crate) fn draw_osd(
        &mut self,
        surface_w: i32,
        surface_h: i32,
        frame: &mut FrameView,
        game_viewport: Option<[i32; 4]>,
    ) -> Result<(OsdOutcome, Option<[i32; 4]>), ViewError> {
        if self.osd_layer.is_some() && self.osd_size != (surface_w, surface_h) {
            self.osd_dirty = true;
        }
        if self.osd_dirty {
            match self.sync_osd_texture(surface_w, surface_h) {
                Ok(()) => {
                    self.osd_dirty = false;
                    self.osd_retry.reset();
                }
                Err(e) => {
                    if self.note_osd_upload_rejected() {
                        return Ok((OsdOutcome::Dropped, game_viewport));
                    }
                    return Err(e);
                }
            }
        }
        let has_layer = self.osd_layer.is_some();
        let has_shader = self.osd_shader.is_some();
        if !(has_layer && has_shader) {
            return Ok((OsdOutcome::Skipped, game_viewport));
        }
        self.use_osd_program(frame)?;
        // Cached OSD matrix: skip the upload when unchanged.
        {
            let loc = self.osd_matrix;
            let last = self.last_osd_matrix;
            let identity = Mat4::identity();
            let mat: [f32; 16] = identity.as_array();
            if last != Some(mat) {
                if let Some(sampler_loc) = loc {
                    ok_gl(uniform_matrix_4fv(sampler_loc, 1, gl::FALSE, &mat))?;
                    ok_gl(check_frame())?;
                    self.last_osd_matrix = Some(mat);
                } else if self.osd_shader.is_some() {
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
        if let Some(sampler) = self.osd_texture_loc
            && self.last_osd_sampler != Some(0)
        {
            ok_gl(uniform_1i(sampler, 0))?;
            ok_gl(check_frame())?;
            self.last_osd_sampler = Some(0);
        }
        use_texture(0, self.osd_texture)?;
        frame.bind_geometry()?;
        // Fullscreen composite with explicit restore: failures propagate
        // instead of debug-logging inside a drop.
        // The memoized viewport skips the per-frame `GetIntegerv`.
        let composite = CompositeGuard::fullscreen(surface_w, surface_h, game_viewport)?;
        let applied_viewport = Some(composite.saved_viewport());
        // Guard restoration must not leave a successful program memo on error.
        frame.set_current_program(None);
        // Shared quad: four vertices with identical position/UV locations.
        let draw = ok_gl(unsafe { draw_arrays(gl::TRIANGLE_STRIP, 0, 4) });
        // The composite restores even when the draw failed, so game GL state
        // never leaks; the draw error still reports first.
        let restored = composite.restore();
        draw?;
        restored?;
        frame.restore_game_binding()?;
        ok_gl(check_frame())?;
        Ok((OsdOutcome::Presented, applied_viewport))
    }

    /// Record one failed OSD upload; `true` means the layer was dropped.
    /// Shared budget owns the count; the next identical push re-stages.
    pub(crate) fn note_osd_upload_rejected(&mut self) -> bool {
        if self.osd_layer.is_none() {
            return false;
        }
        let dropped = self.osd_retry.charge(false);
        if dropped {
            self.osd_layer.take();
            self.osd_dirty = false;
        }
        dropped
    }

    /// Upload the staged layer into the full-surface texture (draw path only binds).
    /// Best-effort: failures keep the previous texture, never panic.
    fn sync_osd_texture(&mut self, surface_w: i32, surface_h: i32) -> Result<(), ViewError> {
        if self.osd_texture == 0 {
            return Ok(());
        }
        if self.osd_size != (surface_w, surface_h) {
            checked_clear_need(surface_w, surface_h).map_err(|_| ViewError::InvalidSurface {
                w: surface_w,
                h: surface_h,
            })?;
            let limit = ok_gl(nerust_glwrap::raw::get_integer(gl::MAX_TEXTURE_SIZE))?;
            if surface_w > limit || surface_h > limit {
                return Err(ViewError::InvalidSurface {
                    w: surface_w,
                    h: surface_h,
                });
            }
            // Defined store, undefined contents: null-alloc sizes the store
            // without a host memset or pixel transfer. The first staged
            // composite clears once; resizes with no OSD layer pay nothing more.
            crate::frame_textures::allocate_texture_store(
                0,
                self.osd_texture,
                surface_w as usize,
                surface_h as usize,
                gl::RGBA as i32,
                gl::RGBA,
            )?;
            ok_gl(check_frame())?;
            self.osd_size = (surface_w, surface_h);
            self.osd_prev = None;
            self.osd_surface_clean = false;
        }
        if self.osd_layer.is_none() {
            if let Some((px, py, pw, ph)) = self.osd_prev {
                let texture = self.osd_texture;
                self.clear_rect(texture, surface_h, px, py, pw, ph)?;
                self.osd_prev = None;
            }
            return Ok(());
        }
        let staged = match self.osd_layer.as_ref() {
            Some(layer) => match checked_osd_rect(layer, surface_w, surface_h) {
                Ok(rect) => rect,
                Err(reason) => {
                    log::warn!("GlView: dropping invalid OSD layer ({reason:?})");
                    self.osd_layer = None;
                    self.osd_dirty = false;
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
            need,
            ..
        } = staged;
        let rect = (lx, ly, lw, lh);
        if !self.osd_surface_clean {
            // First staged composite after a resize: the null store becomes
            // transparent once, before any subrect blit.
            let texture = self.osd_texture;
            self.clear_rect(texture, surface_h, 0, 0, surface_w, surface_h)?;
            self.osd_surface_clean = true;
        }
        if self.osd_prev != Some(rect)
            && let Some((px, py, pw, ph)) = self.osd_prev
        {
            let texture = self.osd_texture;
            self.clear_rect(texture, surface_h, px, py, pw, ph)?;
        }
        let Some(layer) = self.osd_layer.as_ref() else {
            return Ok(());
        };
        let texture = self.osd_texture;
        // Validated RGBA is already tight; ignore any trailing bytes.
        Self::blit_bytes(texture, surface_h, lx, ly, lw, lh, &layer.rgba()[..need])?;
        self.osd_prev = Some(rect);
        Ok(())
    }

    /// Clear a subrect to transparent using the reusable zero scratch.
    /// Texture rows and the shared quad's UVs are both top-first.
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
        self.osd_clear_scratch.staging(need).fill(0);
        let gl_y = checked_osd_y(surface_h, y, h).map_err(|_| ViewError::InvalidOsdFlip {
            surface_h,
            y,
            h,
        })?;
        let rect = (x, y, w, h);
        crate::frame_textures::prepare_cpu_unpack()?;
        use_texture(0, texture)?;
        ok_osd_gl(
            unsafe {
                tex_sub_image_2d(
                    gl::TEXTURE_2D,
                    0,
                    x,
                    gl_y,
                    w,
                    h,
                    gl::RGBA,
                    gl::UNSIGNED_BYTE,
                    self.osd_clear_scratch.staging(need).as_ptr().cast(),
                )
            },
            rect,
            "clear",
        )?;
        ok_osd_gl(check_frame(), rect, "clear")?;
        Ok(())
    }

    /// Upload RGBA rows into a texture subrect (best-effort).
    /// Texture rows and the shared quad's UVs are both top-first.
    pub(crate) fn blit_bytes(
        texture: u32,
        surface_h: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        bytes: &[u8],
    ) -> Result<(), ViewError> {
        let need = checked_clear_need(w, h).map_err(|_| ViewError::InvalidOsdClear { w, h })?;
        if bytes.len() < need {
            return Err(crate::layout::FrameError::Shared(
                nerust_render_traits::FrameError::TooShort {
                    need,
                    have: bytes.len(),
                },
            )
            .into());
        }
        let gl_y = checked_osd_y(surface_h, y, h).map_err(|_| ViewError::InvalidOsdFlip {
            surface_h,
            y,
            h,
        })?;
        let rect = (x, y, w, h);
        crate::frame_textures::prepare_cpu_unpack()?;
        use_texture(0, texture)?;
        ok_osd_gl(
            unsafe {
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
                )
            },
            rect,
            "blit",
        )?;
        ok_osd_gl(check_frame(), rect, "blit")?;
        Ok(())
    }

    /// Reset overlay program state before a fresh load.
    /// `init_osd` overwrites the texture and shader below.
    pub(crate) fn reset_for_load(&mut self) {
        self.osd_matrix = None;
        self.last_osd_matrix = None;
        self.osd_texture_loc = None;
        self.last_osd_sampler = None;
    }

    /// Best-effort overlay program plus a placeholder texture.
    /// Unlike the game pipeline, failure only drops the OSD.
    pub(crate) fn init_osd(&mut self, is_gles: bool) {
        for name in self.close(Current::assert_current(true)) {
            log::debug!("OSD reinitialization could not delete {name:?}");
        }
        self.osd_dirty = self.osd_layer.is_some();
        self.osd_retry.reset();
        use crate::frame_textures::upload_texture_bytes;
        use crate::shader_compiler::compile_osd_program;
        let mut names = [0u32; 1];
        let generated = gen_textures(1, &mut names);
        self.osd_texture = names[0];
        if generated.is_err() {
            return;
        }
        if upload_texture_bytes(
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
        let Some(shader) = compile_osd_program(is_gles) else {
            return;
        };
        self.osd_matrix = shader.uniform_opt("unif_matrix");
        // Optional sampler resolved once here: a dropped `osd_texture`
        // defaults to unit 0, so the composite skips the upload instead of failing.
        self.osd_texture_loc = shader.uniform_opt("osd_texture");
        self.last_osd_matrix = None;
        self.last_osd_sampler = None;
        self.osd_shader = Some(shader);
    }

    /// Explicit teardown: drop the overlay texture plus program.
    /// `None` (no current context) reports live names instead of deleting blind.
    pub(crate) fn close(&mut self, current: Option<Current>) -> Vec<LeakedName> {
        let policy = DeletionPolicy::Count;
        let mut leaked = Vec::new();
        let has_context = current.is_some();
        if self.osd_texture != 0 {
            if !has_context {
                if policy.note(true) {
                    leaked.push(LeakedName::Texture);
                }
                log::debug!("OsdView teardown skipped: no current context");
            } else if delete_textures(1, &[self.osd_texture]).is_err() {
                log::debug!("OsdView teardown failed");
                if policy.note(true) {
                    leaked.push(LeakedName::Texture);
                }
            }
        }
        self.osd_texture = 0;
        self.osd_size = (0, 0);
        self.osd_prev = None;
        self.last_osd_matrix = None;
        self.last_osd_sampler = None;
        if let Some(mut shader) = self.osd_shader.take()
            && shader.destroy_if_current(current)
        {
            leaked.push(LeakedName::Program);
        }
        leaked
    }
}

#[cfg(test)]
mod tests {
    use super::{OsdOutcome, OsdView};
    use nerust_render_traits::renderer::{OsdLayer, UploadBudget};

    fn staged_view() -> OsdView {
        let mut view = OsdView::default();
        view.set_osd_layer(Some(OsdLayer::new(0, 0, 2, 1, vec![255; 8])));
        view
    }

    #[test]
    fn set_osd_layer_skips_equal_upload() {
        let mut view = OsdView::default();
        let layer = OsdLayer::new(0, 0, 2, 1, vec![255; 8]);
        view.set_osd_layer(Some(layer.clone()));
        assert!(view.osd_dirty);
        view.osd_dirty = false;
        view.set_osd_layer(Some(layer));
        assert!(!view.osd_dirty);
        view.set_osd_layer(None);
        assert!(view.osd_dirty);
    }

    #[test]
    fn upload_failures_retry_then_drop_layer() {
        let mut view = staged_view();
        assert!(!view.note_osd_upload_rejected());
        assert!(view.osd_layer.is_some());
        assert!(view.osd_dirty);
        assert!(!view.note_osd_upload_rejected());
        assert!(view.osd_layer.is_some());
        assert!(view.note_osd_upload_rejected());
        assert!(view.osd_layer.is_none());
        assert!(!view.osd_dirty);
        assert!(!view.note_osd_upload_rejected());
        assert!(view.osd_layer.is_none());
        assert!(!view.osd_dirty);
    }

    #[test]
    fn injected_policy_drives_drop_decision() {
        let mut view = staged_view();
        view.set_osd_policy(UploadBudget::new(1));
        assert!(view.note_osd_upload_rejected());
        assert!(view.osd_layer.is_none());
        assert!(!view.osd_dirty);
    }

    #[test]
    fn retry_budget_uses_single_shared_policy() {
        // Rebuilt shared budget: two failures then drop.
        let mut view = staged_view();
        view.set_osd_policy(UploadBudget::new(3));
        assert!(!view.note_osd_upload_rejected());
        assert!(!view.note_osd_upload_rejected());
        assert!(view.note_osd_upload_rejected());
        // Zero clamps to one attempt, never drops live layers at once.
        let mut view = staged_view();
        view.set_osd_policy(UploadBudget::new(0));
        assert!(view.note_osd_upload_rejected());
    }

    #[test]
    fn dropped_outcome_is_copyable_fact() {
        let outcome = super::OsdOutcome::Dropped;
        assert_eq!(outcome, OsdOutcome::Dropped);
        assert_ne!(outcome, OsdOutcome::Presented);
    }

    #[test]
    fn close_report_without_context_counts_names() {
        use nerust_glwrap::Current;
        let mut view = OsdView {
            osd_texture: 7,
            ..OsdView::default()
        };
        let leaked = view.close(Current::assert_current(false));
        assert_eq!(leaked.len(), 1);
        assert_eq!(view.osd_texture, 0);
    }

    #[test]
    fn program_gate_fails_typed_without_shader() {
        let mut view = OsdView::default();
        let mut frame = crate::frame_view::FrameView::new();
        assert!(matches!(
            view.draw_osd(64, 64, &mut frame, None),
            Ok((OsdOutcome::Skipped, None))
        ));
        // Shader-less program use fails typed before any GL call.
        assert!(matches!(
            view.use_osd_program(&mut frame),
            Err(crate::error::ViewError::NoProgram { program: "osd" })
        ));
    }

    #[test]
    fn invalid_layer_drops_without_gl_when_size_matches() {
        // Size already matches: no realloc, so the corrupt rect drops
        // before any GL call.
        let mut view = OsdView {
            osd_texture: 1,
            osd_size: (64, 64),
            ..OsdView::default()
        };
        view.set_osd_layer(Some(OsdLayer::new(63, 63, 2, 2, vec![0; 16])));
        assert!(view.sync_osd_texture(64, 64).is_ok());
        assert!(view.osd_layer.is_none());
        assert!(!view.osd_dirty);
    }

    #[test]
    fn clear_path_with_no_layer_is_noop() {
        let mut view = OsdView::default();
        assert!(view.sync_osd_texture(64, 64).is_ok());
        // Texture-less stage with a live layer skips before GL.
        view.set_osd_layer(Some(OsdLayer::new(0, 0, 2, 1, vec![255; 8])));
        assert!(view.sync_osd_texture(64, 64).is_ok());
        assert!(view.osd_layer.is_some());
    }
}
