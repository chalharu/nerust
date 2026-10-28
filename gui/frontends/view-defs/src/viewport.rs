//! Viewport render driver: pipeline ownership and frame driving live on the
//! view-definition side. Frontends provide only the content-area region
//! (`wakui::ViewportRegion`) and composite or present the outcome.
//!
//! The render engine (`nerust_render_traits`) is a peer asset; the UI
//! framework (`wakui`) never names it. This module is the single consumer
//! boundary: frontend crates must not name render types.

use std::{rc::Rc, time::Instant};

use nerust_render_traits::{
    FrameBuffer, ReservedStrip, SurfaceSize, VideoRenderProfile,
    renderer::{
        DEFAULT_OSD_UPLOAD_RETRIES, OsdLayer, OsdSink, PipelineFactory, ProfileSwitch,
        RenderPipeline, RenderResult, RendererConfig, RendererError, StripSink, SurfaceLifecycle,
    },
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use wakui::viewport::ViewportRegion;

use crate::{application::ApplicationHost, frame::FrameCtx, osd::OsdUpdate};

pub(crate) use wakui::viewport::{ViewportImage, ViewportPresent};

/// Pipeline ownership plus attach/generation bookkeeping. Profile, frames,
/// and OSD content arrive from `ApplicationHost`; the driver never touches
/// the session itself.
pub struct RenderDriver {
    factory: Option<Rc<dyn PipelineFactory>>,
    pipeline: Option<Box<dyn RenderPipeline>>,
    attached_profile: Option<VideoRenderProfile>,
    /// Handles of the current attach; presents reuse them so frontends
    /// supply the region once at sync time.
    region: Option<ViewportRegion>,
    attached: bool,
    output_generation: u64,
}

impl RenderDriver {
    pub(crate) fn new() -> Self {
        Self {
            factory: None,
            pipeline: None,
            attached_profile: None,
            region: None,
            attached: false,
            output_generation: 0,
        }
    }

    fn set_factory(&mut self, factory: Rc<dyn PipelineFactory>) {
        self.factory = Some(factory);
    }

    /// Ensure a pipeline exists and is attached for `profile`. Rebuilds when
    /// forced, unattached, or the profile moved. The old pipeline drops
    /// before the new attach makes its own context current.
    fn sync(
        &mut self,
        region: ViewportRegion,
        profile: &VideoRenderProfile,
        vsync: bool,
        force: bool,
    ) -> Result<bool, String> {
        if !force && self.attached && same_profile(self.attached_profile.as_ref(), Some(profile)) {
            return Ok(true);
        }
        let factory = self
            .factory
            .clone()
            .ok_or_else(|| "no GPU factory installed".to_string())?;
        // Drop first: a half-built pipeline never survives a failure below.
        self.attached = false;
        self.pipeline = None;
        self.attached_profile = None;
        let config = RendererConfig {
            render_profile: profile.clone(),
            vsync,
            max_osd_upload_retries: DEFAULT_OSD_UPLOAD_RETRIES,
        };
        let mut pipeline = factory
            .create_pipeline(&config, region.display)
            .map_err(|error| error.to_string())?;
        pipeline
            .attach(
                region.window,
                region.display,
                SurfaceSize::new(region.width, region.height),
            )
            .map_err(|error| error.to_string())?;
        self.pipeline = Some(pipeline);
        self.attached_profile = Some(profile.clone());
        self.region = Some(region);
        self.attached = true;
        self.output_generation = self.output_generation.wrapping_add(1);
        Ok(true)
    }

    fn render_direct(&mut self, frame: &FrameBuffer) -> RenderResult {
        match self.pipeline.as_mut() {
            Some(pipeline) => pipeline.render(frame),
            None => RenderResult::Skipped,
        }
    }

    fn render_composited(
        &mut self,
        frame: &FrameBuffer,
        width: u32,
        height: u32,
    ) -> Option<ViewportImage> {
        if !self.attached {
            return None;
        }
        let image = self
            .pipeline
            .as_mut()?
            .render_image(frame, SurfaceSize::new(width, height))?;
        Some(ViewportImage {
            width: image.width,
            height: image.height,
            stride_bytes: image.stride_bytes,
            pixels: image.pixels,
        })
    }

    fn apply_osd(&mut self, update: OsdUpdate) {
        if let Some(pipeline) = self.pipeline.as_mut() {
            update.apply(pipeline.as_mut());
        }
    }

    fn resize_if_needed(&mut self, width: u32, height: u32) {
        let size = SurfaceSize::new(width, height);
        if let Some(pipeline) = self.pipeline.as_mut()
            && pipeline.size() != size
        {
            pipeline.resize(size);
        }
    }

    /// Drop the pipeline without touching the factory: teardown and
    /// target-change paths use this so GPU resources release before
    /// the native owner they borrow.
    fn drop_pipeline(&mut self) {
        self.attached = false;
        self.pipeline = None;
        self.attached_profile = None;
        self.region = None;
    }

    /// Test seam: install a stub pipeline without a GPU factory.
    #[doc(hidden)]
    pub fn install_test_pipeline(&mut self, pipeline: Box<dyn RenderPipeline>) {
        self.pipeline = Some(pipeline);
        self.attached = true;
        self.output_generation = self.output_generation.wrapping_add(1);
    }
}

impl ApplicationHost {
    /// Install the GPU factory chosen by the composition root. Frontends
    /// forward the opaque handle; they never name the factory type.
    pub fn set_gpu_factory(&mut self, factory: Rc<dyn PipelineFactory>) {
        self.viewport.set_factory(factory);
    }

    /// Ensure the pipeline is attached for the current profile. Rebuilds
    /// when forced, unattached, or the profile moved. `Ok(true)` means a
    /// pipeline is attached; `Ok(false)` means no profile is loaded, which
    /// is a skip, never an error.
    pub fn viewport_sync(&mut self, region: ViewportRegion, force: bool) -> Result<bool, String> {
        let Some(profile) = self.render_profile() else {
            return Ok(false);
        };
        let vsync = self.settings_snapshot().local.video.presentation.vsync;
        let ready = self
            .viewport
            .sync(region, &profile, vsync, force)
            .map_err(|detail| format!("viewport attach failed: {detail}"))?;
        if ready {
            self.invalidate_osd();
        }
        Ok(ready)
    }

    /// Direct-present one frame on the native swapchain: resize, OSD,
    /// swap, render, and result bookkeeping, all inside the host borrow.
    /// Handles come from the last successful sync; sizes refresh per frame.
    pub fn viewport_present(
        &mut self,
        width: u32,
        height: u32,
        client_height: u32,
    ) -> ViewportPresent {
        if !self.viewport.attached {
            return ViewportPresent::NoPipeline;
        }
        self.viewport.resize_if_needed(width, height);
        let ctx = self.frame_ctx();
        let size = SurfaceSize::new(width, height);
        let update = self.prepare_osd(size, client_height, &ctx, Instant::now());
        if let Some(pipeline) = self.viewport.pipeline.as_mut() {
            update.apply(pipeline.as_mut());
        }
        self.swap_frame_buffer();
        // Field-split borrows: the session frame and the driver pipeline
        // are disjoint, so presenting stays zero-copy.
        let counter = ctx.tick().frame_counter;
        let Some(frame) = self.session.frame_buffer() else {
            return ViewportPresent::Skipped;
        };
        let result = self.viewport.render_direct(frame);
        self.on_viewport_result(result, counter)
    }

    fn on_viewport_result(&mut self, result: RenderResult, frame_counter: u64) -> ViewportPresent {
        match result {
            RenderResult::Presented => {
                self.render_succeeded();
                self.frame_presented(frame_counter);
                ViewportPresent::Presented
            }
            RenderResult::Error => {
                self.viewport.attached = false;
                self.viewport.pipeline = None;
                self.render_failed("Rendering failed; retrying with backoff".into());
                ViewportPresent::NoPipeline
            }
            RenderResult::Skipped => ViewportPresent::Skipped,
        }
    }

    /// Composited capture for frontends that blend themselves: render one
    /// frame into memory. `None` skips the frame without error. Field-split
    /// borrows keep this zero-copy: no per-frame buffer clone.
    pub fn viewport_capture(&mut self, width: u32, height: u32) -> Option<ViewportImage> {
        let frame = self.session.frame_buffer()?;
        self.viewport.render_composited(frame, width, height)
    }

    /// Apply an OSD update computed against `generation`; stale updates
    /// from a previous attach never touch the new pipeline.
    pub fn viewport_apply_osd(&mut self, update: OsdUpdate, generation: u64) {
        if generation == self.viewport.output_generation {
            self.viewport.apply_osd(update);
        }
    }

    /// OSD frame for a viewport of plain pixel extents. Same content as
    /// `prepare_osd`; the `SurfaceSize` conversion stays inside the host so
    /// frontends never name render types to poll OSD.
    pub fn prepare_osd_viewport(
        &mut self,
        width: u32,
        height: u32,
        client_height: u32,
        ctx: &FrameCtx,
        now: Instant,
    ) -> OsdUpdate {
        self.prepare_osd(SurfaceSize::new(width, height), client_height, ctx, now)
    }

    /// Test seam: install a stub pipeline without a GPU factory.
    /// Frontends use this with `testkit::NullPipeline`.
    #[doc(hidden)]
    pub fn viewport_install_test_pipeline(&mut self, pipeline: Box<dyn RenderPipeline>) {
        self.viewport.install_test_pipeline(pipeline);
    }

    /// Drop the attached pipeline, if any. Teardown and target-change
    /// paths call this so GPU resources release before their native owner.
    pub fn viewport_drop(&mut self) {
        self.viewport.drop_pipeline();
    }

    pub fn viewport_attached(&self) -> bool {
        self.viewport.attached
    }

    pub fn viewport_generation(&self) -> u64 {
        self.viewport.output_generation
    }
}

/// Profile identity for rebuild gating: geometry, format, and filter
/// payload. `VideoRenderProfile` carries an `Arc` blob, so equality is
/// structural, never derived.
fn same_profile(a: Option<&VideoRenderProfile>, b: Option<&VideoRenderProfile>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a.source_logical_size == b.source_logical_size
                && a.logical_size == b.logical_size
                && a.physical_size.width == b.physical_size.width
                && a.physical_size.height == b.physical_size.height
                && a.frame_format == b.frame_format
                && a.ntsc_packed_rgba8 == b.ntsc_packed_rgba8
        }
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        testkit::{NullFactory, NullPipeline},
        *,
    };
    use nerust_render_traits::PixelFormat;
    use std::num::NonZeroIsize;

    fn dummy_region() -> ViewportRegion {
        ViewportRegion {
            window: RawWindowHandle::Win32(raw_window_handle::Win32WindowHandle::new(
                NonZeroIsize::new(1).unwrap(),
            )),
            display: RawDisplayHandle::Xlib(raw_window_handle::XlibDisplayHandle::new(None, 0)),
            width: 64,
            height: 64,
        }
    }

    fn test_profile() -> VideoRenderProfile {
        use nerust_render_traits::{logical::LogicalSize, physical::PhysicalSize};
        VideoRenderProfile {
            frame_format: nerust_render_traits::VideoFrameFormat::Rgba,
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
            ntsc_packed_rgba8: None,
        }
    }

    fn test_frame() -> FrameBuffer {
        FrameBuffer::with_capacity(64, 64, PixelFormat::Rgba)
    }

    fn driver(factory: Rc<dyn PipelineFactory>) -> RenderDriver {
        let mut driver = RenderDriver::new();
        driver.set_factory(factory);
        driver
    }

    fn stub_factory(fail_create: bool, fail_attach: bool, blank: bool) -> Rc<dyn PipelineFactory> {
        Rc::new(NullFactory {
            fail_create,
            fail_attach,
            blank_images: blank,
            ..Default::default()
        })
    }

    #[test]
    fn failed_create_leaves_driver_detached() {
        let mut driver = driver(stub_factory(true, false, false));
        assert!(
            driver
                .sync(dummy_region(), &test_profile(), true, false)
                .is_err()
        );
        assert!(!driver.attached);
        assert!(driver.render_composited(&test_frame(), 64, 64).is_none());
        assert_eq!(driver.output_generation, 0);
        driver.apply_osd(OsdUpdate::default());
    }

    #[test]
    fn successful_sync_renders_sized_images() {
        let mut driver = driver(stub_factory(false, false, false));
        assert!(
            driver
                .sync(dummy_region(), &test_profile(), true, false)
                .unwrap()
        );
        assert!(driver.attached);
        let image = driver
            .render_composited(&test_frame(), 8, 8)
            .expect("stub always renders");
        assert_eq!((image.width, image.height), (8, 8));
        assert_eq!(image.pixels.len(), 8 * 8 * 4);
        driver.apply_osd(OsdUpdate::default());
    }

    #[test]
    fn failed_attach_leaves_driver_detached() {
        let mut driver = driver(stub_factory(false, true, false));
        assert!(
            driver
                .sync(dummy_region(), &test_profile(), true, false)
                .is_err()
        );
        assert!(!driver.attached);
        assert!(driver.render_composited(&test_frame(), 64, 64).is_none());
        assert_eq!(driver.output_generation, 0);
    }

    #[test]
    fn blank_backend_frame_skips_without_detaching() {
        let mut driver = driver(stub_factory(false, false, true));
        driver
            .sync(dummy_region(), &test_profile(), true, false)
            .unwrap();
        // A skipped backend frame is transient: the pipeline stays attached
        // and no generation bump happens.
        assert!(driver.render_composited(&test_frame(), 64, 64).is_none());
        assert!(driver.attached);
        assert_eq!(driver.output_generation, 1);
    }

    #[test]
    fn same_profile_sync_skips_rebuild() {
        let mut driver = driver(stub_factory(false, false, false));
        driver
            .sync(dummy_region(), &test_profile(), true, false)
            .unwrap();
        assert_eq!(driver.output_generation, 1);
        // Same profile, no force: no rebuild, no generation bump.
        assert!(
            driver
                .sync(dummy_region(), &test_profile(), true, false)
                .unwrap()
        );
        assert_eq!(driver.output_generation, 1);
        // Force rebuilds even with the same profile.
        assert!(
            driver
                .sync(dummy_region(), &test_profile(), true, true)
                .unwrap()
        );
        assert_eq!(driver.output_generation, 2);
    }

    #[test]
    fn direct_render_paths_report_status() {
        // No pipeline: skip without touching the frame.
        let mut driver = RenderDriver::new();
        assert_eq!(driver.render_direct(&test_frame()), RenderResult::Skipped);
        // Installed stub presents.
        driver.install_test_pipeline(Box::new(NullPipeline::default()));
        assert_eq!(driver.render_direct(&test_frame()), RenderResult::Presented);
        driver.resize_if_needed(64, 64);
    }
}
/// observable call counts. Frontends name only this kit, never render types.
/// Test stubs shared by frontend test suites: a no-GPU pipeline with
/// observable call counts. Frontends name only this kit, never render types.
pub mod testkit {
    use super::*;
    use nerust_render_traits::{RenderedImage, renderer::FrameSink};

    /// Null pipeline: records calls, renders nothing.
    /// `blank` makes image captures skip (transient backend skip);
    /// `fail_attach` refuses attachment.
    #[derive(Debug, Default)]
    pub struct NullPipeline {
        pub renders: usize,
        pub images: usize,
        pub attaches: usize,
        pub fail_attach: bool,
        pub blank: bool,
    }
    impl SurfaceLifecycle for NullPipeline {
        fn size(&self) -> SurfaceSize {
            SurfaceSize::new(64, 64)
        }
        fn attach(
            &mut self,
            _window: RawWindowHandle,
            _display: RawDisplayHandle,
            _size: SurfaceSize,
        ) -> Result<(), RendererError> {
            self.attaches += 1;
            if self.fail_attach {
                return Err(RendererError::new(
                    "test",
                    Box::new(std::io::Error::other("boom")),
                ));
            }
            Ok(())
        }
        fn detach(&mut self) {}
        fn resize(&mut self, _size: SurfaceSize) {}
    }

    impl FrameSink for NullPipeline {
        fn render(&mut self, _frame: &FrameBuffer) -> RenderResult {
            self.renders += 1;
            RenderResult::Presented
        }
        fn render_image(
            &mut self,
            _frame: &FrameBuffer,
            size: SurfaceSize,
        ) -> Option<RenderedImage> {
            self.images += 1;
            if self.blank {
                return None;
            }
            Some(RenderedImage {
                width: size.width,
                height: size.height,
                stride_bytes: size.width as usize * 4,
                pixels: vec![0; size.width as usize * size.height as usize * 4],
            })
        }
    }

    impl ProfileSwitch for NullPipeline {
        fn update_render_profile(
            &mut self,
            _profile: &VideoRenderProfile,
        ) -> Result<(), RendererError> {
            Ok(())
        }
    }

    impl OsdSink for NullPipeline {
        fn set_osd_layer(&mut self, _layer: Option<OsdLayer>) {}
    }

    impl StripSink for NullPipeline {
        fn set_reserved_strip(&mut self, _strip: ReservedStrip) {}
    }

    /// Null factory: hands out `NullPipeline` without touching a GPU.
    /// Frontends install this in tests instead of a render backend.
    #[derive(Debug, Default)]
    pub struct NullFactory {
        pub pipelines: std::cell::Cell<usize>,
        pub fail_create: bool,
        pub fail_attach: bool,
        pub blank_images: bool,
    }

    impl PipelineFactory for NullFactory {
        fn create_pipeline(
            &self,
            _config: &RendererConfig,
            _display: RawDisplayHandle,
        ) -> Result<Box<dyn RenderPipeline>, RendererError> {
            if self.fail_create {
                return Err(RendererError::new(
                    "test",
                    Box::new(std::io::Error::other("boom")),
                ));
            }
            self.pipelines.set(self.pipelines.get() + 1);
            Ok(Box::new(NullPipeline {
                fail_attach: self.fail_attach,
                blank: self.blank_images,
                ..Default::default()
            }))
        }
    }
}
