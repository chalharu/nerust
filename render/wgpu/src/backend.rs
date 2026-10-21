use nerust_render_traits::{
    FrameBuffer, FrameLayout, ReservedStrip, SurfaceSize, VideoFrameSpec, VideoPresentation,
    VideoRenderProfile,
    renderer::{
        FrameSink, OpaqueError, OsdLayer, OsdSink, OsdUploadPolicy, ProfileSwitch, RenderResult,
        RendererConfig, RendererError, StripSink, SurfaceLifecycle, UploadBudget, note_osd_upload,
    },
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::{
    renderer::{DeviceLimitProfile, PresentationOptions, RenderOutcome, RenderPipeline},
    surface,
};

// WgpuRenderer.
// ---------------------------------------------------------------------------

/// Wgpu device + pipeline.  Surface is attached/detached dynamically.
pub struct WgpuRenderer {
    instance: wgpu::Instance,
    render_profile: VideoRenderProfile,
    pipeline: Option<RenderPipeline>,
    surface: Option<wgpu::Surface<'static>>,
    size: SurfaceSize,
    last_render_error: Option<String>,
    osd: Option<OsdLayer>,
    osd_dirty: bool,
    /// Shared OSD upload budget: repeated rejections drop the layer.
    osd_policy: UploadBudget,
    reserved: ReservedStrip,
    vsync: bool,
}

impl std::fmt::Debug for WgpuRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuRenderer")
            .field("size", &self.size)
            .field("last_render_error", &self.last_render_error)
            .finish_non_exhaustive()
    }
}

impl WgpuRenderer {
    fn build_pipeline(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        size: SurfaceSize,
        profile: &VideoRenderProfile,
        vsync: bool,
    ) -> Result<RenderPipeline, RendererError> {
        let presentation = VideoPresentation::new(VideoFrameSpec::new(
            profile.frame_format,
            profile.source_logical_size,
            profile.logical_size,
            profile.physical_size,
        ));
        pollster::block_on(RenderPipeline::new(
            instance,
            surface,
            size,
            &presentation,
            profile.ntsc_packed_rgba8.as_deref(),
            PresentationOptions { vsync },
            WgpuFactory::device_limit_profile(),
        ))
        .map_err(|e| {
            RendererError::attach(
                "wgpu",
                "pipeline",
                Box::new(OpaqueError(format!("pipeline: {e}"))),
            )
        })
    }
}

impl WgpuRenderer {
    fn report_outcome(&mut self, outcome: Result<RenderOutcome, String>) -> RenderResult {
        match outcome {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                RenderResult::Presented
            }
            Ok(RenderOutcome::Skipped) => RenderResult::Skipped,
            Ok(RenderOutcome::RecreateSurface) => {
                self.last_render_error = Some("wgpu surface lost".into());
                RenderResult::Error
            }
            Err(error) => {
                if self.last_render_error.as_deref() != Some(error.as_str()) {
                    log::error!("wgpu: {error}");
                }
                self.last_render_error = Some(error);
                RenderResult::Error
            }
        }
    }

    /// Push the staged layer into a fresh pipeline. Rebuilds drop
    /// GPU objects, so attach/profile paths must re-apply.
    fn reapply_osd(&mut self) {
        if self.osd.is_some() {
            self.osd_dirty = true;
        }
    }
}

impl SurfaceLifecycle for WgpuRenderer {
    fn size(&self) -> SurfaceSize {
        self.size
    }

    fn attach(
        &mut self,
        wh: RawWindowHandle,
        dh: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        let wgpu_surface = surface::create_wgpu_surface(&self.instance, wh, dh)
            .map_err(|e| RendererError::attach("wgpu", "surface", Box::new(OpaqueError(e))))?;
        let mut pipeline = Self::build_pipeline(
            &self.instance,
            &wgpu_surface,
            size,
            &self.render_profile,
            self.vsync,
        )?;
        pipeline.set_reserved_strip(self.reserved);
        self.size = size;
        self.surface = Some(wgpu_surface);
        self.pipeline = Some(pipeline);
        self.reapply_osd();
        Ok(())
    }

    fn detach(&mut self) {
        self.surface = None;
        self.pipeline = None;
    }

    fn resize(&mut self, size: SurfaceSize) {
        self.size = size;
        self.reapply_osd();
        let Some(ref surface) = self.surface else {
            return;
        };
        let Some(ref mut pipeline) = self.pipeline else {
            return;
        };
        pipeline.reconfigure_surface(surface, size);
    }
}

impl FrameSink for WgpuRenderer {
    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
        let Some(ref surface) = self.surface else {
            return RenderResult::Skipped;
        };
        let Some(ref mut pipeline) = self.pipeline else {
            return RenderResult::Skipped;
        };

        if self.size.width == 0 || self.size.height == 0 {
            return RenderResult::Skipped;
        }
        if let Err(error) = FrameLayout::validate(
            frame,
            self.render_profile.input_size(),
            self.render_profile.frame_format,
        ) {
            log::warn!("wgpu: {error}");
            return RenderResult::Skipped;
        }

        if let Some(p) = frame.palette_as_rgba8() {
            pipeline.update_palette_texture(&p);
        }
        if self.osd_dirty {
            let accepted = crate::renderer::setup::set_osd_layer(pipeline, self.osd.as_ref());
            if note_osd_upload(accepted, &mut self.osd, &mut self.osd_policy) {
                log::warn!("wgpu: dropping OSD layer after repeated upload failures");
            }
            // Retry while a layer remains; a drop clears the slot.
            self.osd_dirty = self.osd.is_some();
        }

        let outcome = pipeline.render(surface, self.size, frame.as_ref(), frame.stride());
        // Hosts already back off and rebuild on Error. Never hide a
        // permanently lost native surface behind an endless skip.
        self.report_outcome(outcome)
    }
}

impl OsdSink for WgpuRenderer {
    fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        if OsdLayer::same_option(self.osd.as_ref(), layer.as_ref()) {
            return;
        }
        self.osd = layer;
        self.osd_dirty = true;
        self.osd_policy.reset();
    }
}

impl StripSink for WgpuRenderer {
    fn set_reserved_strip(&mut self, strip: ReservedStrip) {
        self.reserved = strip;
        if let Some(ref mut pipeline) = self.pipeline {
            pipeline.set_reserved_strip(strip);
        }
    }
}

impl ProfileSwitch for WgpuRenderer {
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        if let Some(ref surface) = self.surface {
            let mut pipeline =
                Self::build_pipeline(&self.instance, surface, self.size, profile, self.vsync)?;
            pipeline.set_reserved_strip(self.reserved);
            self.pipeline = Some(pipeline);
        }
        // Commit only after a successful rebuild; detached updates stage for attach.
        self.render_profile = profile.clone();
        self.reapply_osd();
        Ok(())
    }
}

// WgpuFactory.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct WgpuFactory;

impl WgpuFactory {
    fn device_limit_profile() -> DeviceLimitProfile {
        #[cfg(target_os = "android")]
        {
            DeviceLimitProfile::DownlevelWebGl2
        }
        #[cfg(not(target_os = "android"))]
        {
            DeviceLimitProfile::Default
        }
    }
}

impl WgpuFactory {
    /// Shared construction for the narrow factory.
    fn build(config: &RendererConfig) -> Result<WgpuRenderer, RendererError> {
        let instance = surface::default_instance();

        Ok(WgpuRenderer {
            instance,
            render_profile: config.render_profile.clone(),
            pipeline: None,
            surface: None,
            size: SurfaceSize::new(0, 0),
            last_render_error: None,
            osd: None,
            osd_dirty: false,
            osd_policy: UploadBudget::new(config.upload_budget()),
            reserved: ReservedStrip::default(),
            vsync: config.vsync,
        })
    }
}

impl nerust_render_traits::renderer::PipelineFactory for WgpuFactory {
    /// Per-use-site composition; same renderer, narrow object.
    fn create_pipeline(
        &self,
        config: &RendererConfig,
        _display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn nerust_render_traits::renderer::RenderPipeline>, RendererError> {
        Ok(Box::new(Self::build(config)?))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn surface_loss_requests_host_recovery_and_success_clears_error() {
        use crate::renderer::RenderOutcome;
        use nerust_render_traits::renderer::RenderResult;
        let mut renderer = detached_renderer();
        assert_eq!(
            renderer.report_outcome(Ok(RenderOutcome::RecreateSurface)),
            RenderResult::Error
        );
        assert!(
            renderer
                .last_render_error
                .as_deref()
                .unwrap()
                .contains("lost")
        );
        assert_eq!(
            renderer.report_outcome(Err("device lost".into())),
            RenderResult::Error
        );
        assert_eq!(
            renderer.report_outcome(Ok(RenderOutcome::Skipped)),
            RenderResult::Skipped
        );
        assert_eq!(
            renderer.report_outcome(Ok(RenderOutcome::Presented)),
            RenderResult::Presented
        );
        assert!(renderer.last_render_error.is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn device_attach_rebuild_preserves_profile_strips_vsync_and_osd() {
        use nerust_render_traits::{
            FrameBuffer, PixelFormat, ReservedStrip, SurfaceSize,
            renderer::{
                FrameSink, OsdLayer, OsdSink, ProfileSwitch, RenderResult, StripSink,
                SurfaceLifecycle,
            },
        };
        let _guard = crate::test_window::lock();
        let Some(window) = crate::test_window::TestWindow::open() else {
            return;
        };
        let mut renderer = detached_renderer();
        renderer.instance = crate::surface::instance_with_display(window.clone());
        let (wh, dh) = window.handles();
        let mut profile = test_profile();
        profile.source_logical_size.width = 2;
        profile.source_logical_size.height = 2;
        profile.logical_size.width = 4;
        profile.logical_size.height = 4;
        renderer.update_render_profile(&profile).unwrap();
        let strip = ReservedStrip { top: 5, bottom: 7 };
        renderer.set_reserved_strip(strip);
        renderer.set_osd_layer(Some(OsdLayer::new(1, 1, 2, 1, vec![255; 8])));
        renderer.attach(wh, dh, SurfaceSize::new(64, 64)).unwrap();
        assert_eq!(renderer.pipeline.as_ref().unwrap().reserved_strip(), strip);
        assert!(renderer.osd_dirty);
        let mut frame = FrameBuffer::with_capacity(8, 8, PixelFormat::Rgba);
        frame.resize(4, 4);
        // No padding after the final row is required.
        frame.resize_data(3 * 256 + 16);
        assert_eq!(renderer.render(&frame), RenderResult::Presented);
        renderer.resize(SurfaceSize::new(80, 80));
        assert!(renderer.osd_dirty);
        assert_eq!(renderer.render(&frame), RenderResult::Presented);
        assert!(!renderer.osd_dirty);
        // A malformed frame and a format mismatch skip before GPU work.
        frame.resize_data(2);
        assert_eq!(renderer.render(&frame), RenderResult::Skipped);
        frame.set_format(PixelFormat::PaletteIndex {
            palette: vec![0; 64].into_boxed_slice(),
        });
        assert_eq!(renderer.render(&frame), RenderResult::Skipped);
        // Update while attached, then reattach: the updated profile must win.
        profile.logical_size.width = 8;
        renderer.update_render_profile(&profile).unwrap();
        assert_eq!(renderer.pipeline.as_ref().unwrap().reserved_strip(), strip);
        renderer.detach();
        renderer.attach(wh, dh, SurfaceSize::new(80, 80)).unwrap();
        assert_eq!(renderer.render_profile.input_size().width, 8);
        assert_eq!(renderer.pipeline.as_ref().unwrap().reserved_strip(), strip);
        assert!(!renderer.vsync);
        let modes = renderer
            .surface
            .as_ref()
            .unwrap()
            .get_capabilities(
                &pollster::block_on(renderer.instance.request_adapter(
                    &wgpu::RequestAdapterOptions {
                        compatible_surface: renderer.surface.as_ref(),
                        ..Default::default()
                    },
                ))
                .unwrap(),
            )
            .present_modes;
        assert_eq!(
            renderer
                .pipeline
                .as_ref()
                .unwrap()
                .surface_config()
                .present_mode,
            crate::renderer::RenderPipeline::select_present_mode(&modes, false)
        );
        frame.set_format(PixelFormat::Rgba);
        frame.resize(8, 4);
        assert_eq!(renderer.render(&frame), RenderResult::Presented);
        // Surplus rows must not overflow the fixed-size GPU upload buffer.
        frame.resize(8, 8);
        assert_eq!(renderer.render(&frame), RenderResult::Presented);
        // Validate profile geometry and NTSC payload before resource creation.
        let previous_width = renderer.render_profile.logical_size.width;
        for width in [0, usize::MAX] {
            let mut invalid = profile.clone();
            invalid.logical_size.width = width;
            assert!(renderer.update_render_profile(&invalid).is_err());
            assert_eq!(renderer.render_profile.logical_size.width, previous_width);
        }
        let mut invalid = profile.clone();
        invalid.ntsc_packed_rgba8 = Some(vec![1, 2, 3].into_boxed_slice());
        assert!(renderer.update_render_profile(&invalid).is_err());
        // Both index pipelines must accept original, not filtered dimensions.
        profile.frame_format = nerust_render_traits::VideoFrameFormat::Palette;
        renderer.update_render_profile(&profile).unwrap();
        frame.set_format(PixelFormat::PaletteIndex {
            palette: vec![0xFF00_00FF; 64].into_boxed_slice(),
        });
        frame.resize(2, 2);
        assert_eq!(renderer.render(&frame), RenderResult::Presented);
        use nerust_render_filters::FilterTypeExt;
        let assets =
            nerust_render_traits::filter::FilterType::NtscRGB.palette_console_video_assets();
        profile.ntsc_packed_rgba8 =
            Some(assets.as_nes().unwrap().packed_ntsc_rgba8().unwrap().into());
        renderer.update_render_profile(&profile).unwrap();
        assert_eq!(renderer.render(&frame), RenderResult::Presented);
        renderer.detach();
    }

    #[test]
    fn lifecycle_split_seams_hold() {
        use nerust_render_traits::renderer::{
            FrameSink, PipelineFactory, ProfileSwitch, RenderPipeline, SurfaceLifecycle,
        };
        fn assert_seams<T: SurfaceLifecycle + FrameSink + ProfileSwitch>() {}
        assert_seams::<super::WgpuRenderer>();
        // Per-use-site composition: the same renderer behind narrow traits,
        // minted by the narrow factory entry point.
        fn assert_pipeline<T: RenderPipeline>() {}
        assert_pipeline::<super::WgpuRenderer>();
        fn assert_factory<T: PipelineFactory>() {}
        assert_factory::<super::WgpuFactory>();
    }

    #[test]
    fn attach_errors_name_backend_and_stage() {
        use nerust_render_traits::renderer::{OpaqueError, RendererError, RendererErrorKind};
        for (stage, backend, detail) in [
            ("adapter", "wgpu", "no adapter"),
            ("pipeline", "wgpu", "compile failed"),
            ("surface", "wgpu", "bad surface"),
            ("device", "wgpu", "lost"),
            ("context", "wgpu", "gone"),
        ] {
            let err =
                RendererError::attach(backend, stage, Box::new(OpaqueError(detail.to_string())));
            assert_eq!(err.kind(), RendererErrorKind::Attach { backend, stage });
            // Display carries context plus detail; backend lives on the kind.
            assert!(err.to_string().contains(detail), "{err}");
        }
    }

    fn detached_renderer() -> super::WgpuRenderer {
        super::WgpuRenderer {
            instance: crate::surface::default_instance(),
            render_profile: test_profile(),
            pipeline: None,
            surface: None,
            size: nerust_render_traits::SurfaceSize::new(0, 0),
            last_render_error: None,
            osd: None,
            osd_dirty: false,
            osd_policy: super::UploadBudget::new(3),
            reserved: nerust_render_traits::ReservedStrip::default(),
            vsync: false,
        }
    }

    fn test_profile() -> nerust_render_traits::VideoRenderProfile {
        use nerust_render_traits::{logical::LogicalSize, physical::PhysicalSize};
        nerust_render_traits::VideoRenderProfile {
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

    #[test]
    fn detached_renderer_skips_and_stages_profile_switch() {
        use nerust_render_traits::renderer::{
            FrameSink, OsdLayer, OsdSink, ProfileSwitch, StripSink, SurfaceLifecycle,
        };
        let mut renderer = detached_renderer();
        assert_eq!(
            renderer.size(),
            nerust_render_traits::SurfaceSize::new(0, 0)
        );
        // No pipeline: strip and layer stage without touching the device.
        renderer.set_reserved_strip(nerust_render_traits::ReservedStrip::default());
        renderer.set_osd_layer(Some(OsdLayer::new(0, 0, 2, 1, vec![255; 8])));
        assert!(renderer.osd_dirty);
        let frame = nerust_render_traits::FrameBuffer::with_capacity(
            64,
            64,
            nerust_render_traits::PixelFormat::Rgba,
        );
        assert_eq!(
            renderer.render(&frame),
            nerust_render_traits::renderer::RenderResult::Skipped
        );
        // OSD stays staged: the skip consumed nothing.
        assert!(renderer.osd_dirty);
        let mut profile = test_profile();
        profile.logical_size.width = 128;
        renderer.update_render_profile(&profile).unwrap();
        assert_eq!(renderer.render_profile.logical_size.width, 128);
        renderer.set_reserved_strip(nerust_render_traits::ReservedStrip { top: 3, bottom: 5 });
        renderer.detach();
        assert_eq!(renderer.reserved.top, 3);
        assert!(!renderer.vsync);
        // Resize and detach on a detached renderer are pure state ops.
        renderer.resize(nerust_render_traits::SurfaceSize::new(64, 64));
        assert_eq!(
            renderer.size(),
            nerust_render_traits::SurfaceSize::new(64, 64)
        );
        renderer.detach();
        assert!(renderer.pipeline.is_none());
    }

    #[test]
    fn error_dedup_arms_cover_all_attach_stages() {
        use nerust_render_traits::renderer::{OpaqueError, RendererError, RendererErrorKind};
        for (stage, kind) in [
            (
                "adapter",
                RendererErrorKind::Attach {
                    backend: "wgpu",
                    stage: "adapter",
                },
            ),
            (
                "device",
                RendererErrorKind::Attach {
                    backend: "wgpu",
                    stage: "device",
                },
            ),
            (
                "surface",
                RendererErrorKind::Attach {
                    backend: "wgpu",
                    stage: "surface",
                },
            ),
            (
                "pipeline",
                RendererErrorKind::Attach {
                    backend: "wgpu",
                    stage: "pipeline",
                },
            ),
        ] {
            let wrapped =
                RendererError::attach("wgpu", stage, Box::new(OpaqueError("x".to_string())));
            assert_eq!(wrapped.kind(), kind);
            assert!(wrapped.to_string().contains("attach"), "{wrapped}");
        }
        assert!(
            RendererError::not_attached("update")
                .to_string()
                .contains("update")
        );
    }

    #[test]
    fn factory_mints_shared_budget_with_floor() {
        use nerust_render_traits::renderer::{OsdUploadPolicy, RendererConfig};
        let mut config = RendererConfig {
            render_profile: test_profile(),
            vsync: false,
            max_osd_upload_retries: 0,
        };
        let mut floored = super::WgpuFactory::build(&config).unwrap();
        assert!(floored.osd_policy.record_failure());
        config.max_osd_upload_retries = 3;
        let mut renderer = super::WgpuFactory::build(&config).unwrap();
        assert!(!renderer.osd_policy.record_failure());
        assert!(!renderer.osd_policy.record_failure());
        assert!(renderer.osd_policy.record_failure());
    }

    #[test]
    fn new_layer_rearms_exhausted_budget() {
        use nerust_render_traits::renderer::{OsdLayer, OsdSink, OsdUploadPolicy};
        let mut renderer = detached_renderer();
        assert!(!renderer.osd_policy.record_failure());
        assert!(!renderer.osd_policy.record_failure());
        assert!(renderer.osd_policy.record_failure());
        renderer.set_osd_layer(Some(OsdLayer::new(0, 0, 2, 1, vec![255; 8])));
        assert!(renderer.osd_dirty);
        assert!(!renderer.osd_policy.record_failure());
    }

    #[test]
    fn device_limit_profile_defaults_off_android() {
        #[cfg(target_os = "android")]
        assert!(matches!(
            super::WgpuFactory::device_limit_profile(),
            crate::renderer::DeviceLimitProfile::DownlevelWebGl2
        ));
        #[cfg(not(target_os = "android"))]
        assert!(matches!(
            super::WgpuFactory::device_limit_profile(),
            crate::renderer::DeviceLimitProfile::Default
        ));
    }
}
