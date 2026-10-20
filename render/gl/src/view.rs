//! GL view facade: frame, overlay, and surface views composed.
//! Orchestration only; per-domain state and GL calls live in the views.

use std::os::raw::c_void;

use nerust_glwrap::CurrentContext;
use nerust_render_traits::{ReservedStrip, VideoRenderProfile, renderer::OsdLayer};

pub(crate) use crate::error::ViewError;
pub use crate::osd_view::OsdOutcome;
pub(crate) use crate::surface_view::CloseReport;

use crate::{frame_view::FrameView, osd_view::OsdView, surface_view::SurfaceView};

#[derive(Debug)]
pub struct GlView {
    frame: FrameView,
    osd: OsdView,
    surface: SurfaceView,
}

impl Default for GlView {
    fn default() -> Self {
        Self::new()
    }
}

impl GlView {
    pub fn new() -> Self {
        Self {
            frame: FrameView::new(),
            osd: OsdView::new(),
            surface: SurfaceView::new(),
        }
    }

    pub fn use_vao(&mut self, value: bool) {
        self.frame.set_use_vao(value);
    }

    pub fn load_with<F: FnMut(&'static str) -> *const c_void>(get_proc_address: F) {
        FrameView::load_with(get_proc_address);
    }

    pub fn on_load(&mut self, render_profile: &VideoRenderProfile) -> Result<(), ViewError> {
        // Reuse must not leak names: tear down while current first.
        let _ = self.on_close(CurrentContext::assert_current(true));
        debug_assert_eq!(self.frame.frame_texture, 0);
        self.surface.reset();
        let is_gles = self.frame.load_frame(render_profile)?;
        self.osd.reset_for_load();
        self.osd.init_osd(is_gles);
        Ok(())
    }

    /// Override the upload retry budget. Equal layers skip staging;
    /// new content resets the failure budget.
    pub fn set_osd_retry_max(&mut self, max: u8) {
        self.osd.set_osd_retry_max(max);
    }

    pub fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        self.osd.set_osd_layer(layer);
    }

    /// Composite the staged layer over the game image, if any (best-effort).
    /// A GL failure drops the overlay, never the frame.
    pub fn draw_osd(&mut self, surface_w: i32, surface_h: i32) -> Result<OsdOutcome, ViewError> {
        // Memoized viewport skips the per-frame `GetIntegerv`;
        // empty stages skip all GL work inside the overlay view.
        let viewport = self.surface.game_viewport();
        let (outcome, applied) =
            self.osd
                .draw_osd(surface_w, surface_h, &mut self.frame, viewport)?;
        if let Some(viewport) = applied {
            self.surface.set_game_viewport(viewport);
        }
        Ok(outcome)
    }

    pub fn update_palette_texture(&self, rgba8: &[u8; 256]) -> Result<(), ViewError> {
        self.frame.update_palette_texture(rgba8)
    }

    pub fn on_update(&mut self, screen: &[u8], row_length: i32) -> Result<(), ViewError> {
        self.frame.on_update(screen, row_length)
    }

    /// Whether the context accepts UNPACK_ROW_LENGTH (no CPU repack).
    pub fn supports_unpack_row_length(&self) -> bool {
        self.frame.supports_unpack_row_length()
    }

    /// Fit the game into the surface minus reserved OSD strips (memoized).
    pub fn on_resize_with_strip(
        &mut self,
        viewport_width: i32,
        viewport_height: i32,
        strip: ReservedStrip,
    ) -> Result<(), ViewError> {
        // Zero/negative viewports: nothing to draw; skip GL work.
        if viewport_width <= 0 || viewport_height <= 0 {
            return Ok(());
        }
        if self
            .surface
            .resize_hit(viewport_width, viewport_height, strip)
        {
            return Ok(());
        }
        let game_box = self
            .frame
            .apply_viewport(viewport_width, viewport_height, strip)?;
        self.surface
            .record_resize(viewport_width, viewport_height, strip, game_box);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn blit_bytes(
        texture: u32,
        surface_h: i32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        bytes: &[u8],
    ) -> Result<(), ViewError> {
        OsdView::blit_bytes(texture, surface_h, x, y, w, h, bytes)
    }

    /// Explicit teardown. `None` (no current context) skips GL calls
    /// and reports each live name as leaked instead of deleting blind.
    pub fn on_close(&mut self, current: Option<CurrentContext>) -> CloseReport {
        let mut report = CloseReport::default();
        for name in self.frame.close(current) {
            report.leak(name);
        }
        for name in self.osd.close(current) {
            report.leak(name);
        }
        self.surface.reset();
        report
    }
}

#[cfg(test)]
mod tests {
    use super::{GlView, OsdOutcome};
    use crate::frame_view::ok_gl;
    use crate::osd_overlay::{RetryBudget, RetryDecision};
    use crate::{
        osd_overlay::{OSDRect, OsdReject, checked_osd_rect, osd_gl_y},
        shader_compiler::{
            build_glsl_source, compose_glsl_source, is_gles_context, supports_unpack_row_length,
        },
    };
    use nerust_render_traits::renderer::OsdLayer;

    #[test]
    fn checked_osd_rect_accepts_inside_and_rejects_rest() {
        let inside = OsdLayer::new(4, 6, 2, 1, vec![1; 8]);
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
        let short = OsdLayer::new(inside.x, inside.y, inside.width, inside.height, vec![1; 7]);
        assert_eq!(
            checked_osd_rect(&short, 64, 64),
            Err(OsdReject::Short { need: 8, have: 7 })
        );
        let flat = OsdLayer::new(inside.x, inside.y, inside.width, 0, vec![]);
        assert_eq!(checked_osd_rect(&flat, 64, 64), Err(OsdReject::Empty));
        let outside = OsdLayer::new(
            63,
            inside.y,
            inside.width,
            inside.height,
            inside.rgba.to_vec(),
        );
        assert_eq!(
            checked_osd_rect(&outside, 64, 64),
            Err(OsdReject::OutOfBounds)
        );
        let huge = OsdLayer::new(inside.x, inside.y, u32::MAX, u32::MAX, vec![]);
        assert_eq!(
            checked_osd_rect(&huge, i32::MAX, i32::MAX),
            Err(OsdReject::Empty)
        );
    }

    #[test]
    fn checked_osd_rect_rejects_i32_overflow_as_out_of_bounds() {
        // u32 coords above i32::MAX refuse instead of wrapping negative.
        let wide = OsdLayer::new(u32::MAX, 0, 2, 1, vec![0; 8]);
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
        let layer = OsdLayer::new(0, 0, 2, 1, vec![255; 8]);
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
        let base = OsdLayer::new(1, 2, 2, 1, vec![9; 8]);
        assert!(OsdLayer::same_option(Some(&base), Some(&base.clone())));
        assert!(!OsdLayer::same_option(
            Some(&base),
            Some(&OsdLayer::new(7, 2, 2, 1, base.rgba.to_vec()))
        ));
        assert!(!OsdLayer::same_option(
            Some(&base),
            Some(&OsdLayer::new(1, 2, 2, 1, vec![8; 8]))
        ));
        assert!(!OsdLayer::same_option(
            Some(&base),
            Some(&OsdLayer::new(1, 2, 1, 1, vec![9; 4]))
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
        view.set_osd_layer(Some(OsdLayer::new(0, 0, 2, 1, vec![255; 8])));
        view
    }

    #[test]
    fn upload_failures_retry_then_drop_layer() {
        let mut view = staged_view();
        assert_eq!(view.osd.note_osd_upload_failure(), RetryDecision::Retry);
        assert!(view.osd.osd_layer.is_some());
        assert!(view.osd.osd_dirty);
        assert_eq!(view.osd.note_osd_upload_failure(), RetryDecision::Retry);
        assert!(view.osd.osd_layer.is_some());
        assert_eq!(view.osd.note_osd_upload_failure(), RetryDecision::DropLayer);
        assert!(view.osd.osd_layer.is_none());
        assert!(!view.osd.osd_dirty);
        assert_eq!(view.osd.note_osd_upload_failure(), RetryDecision::Retry);
        assert!(view.osd.osd_layer.is_none());
        assert!(!view.osd.osd_dirty);
    }

    #[test]
    fn upload_failure_counter_saturates() {
        let mut view = staged_view();
        view.osd.osd_retry.fails = u8::MAX;
        assert_eq!(view.osd.note_osd_upload_failure(), RetryDecision::DropLayer);
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
        assert_eq!(report.failed_textures(), 1);
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
        OsdLayer::new(0, 0, 2, 1, vec![255; 8])
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
