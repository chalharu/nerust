use nerust_render_traits::{
    FrameBuffer, ReservedStrip, SurfaceSize, VideoFrameSpec, VideoPresentation, VideoRenderProfile,
    renderer::{
        AttachError, FrameSink, GpuFactory, GpuRenderer, OsdLayer, OsdSink, Present, ProfileSwitch,
        RenderResult, RendererConfig, RendererError, StripSink, SurfaceLifecycle,
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
        #[cfg(target_os = "android")]
        let dl = DeviceLimitProfile::DownlevelWebGl2;
        #[cfg(not(target_os = "android"))]
        let dl = DeviceLimitProfile::Default;
        pollster::block_on(RenderPipeline::new(
            instance,
            surface,
            size,
            &presentation,
            profile.ntsc_packed_rgba8.as_deref(),
            PresentationOptions { vsync },
            dl,
        ))
        .map_err(|e| {
            RendererError::new(
                "pipeline",
                Box::new(AttachError::Pipeline {
                    backend: "wgpu",
                    detail: e.to_string(),
                }),
            )
        })
    }
}

impl WgpuRenderer {
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
        self.size = size;
        let wgpu_surface = surface::create_wgpu_surface(&self.instance, wh, dh).map_err(|e| {
            RendererError::new(
                "surface",
                Box::new(AttachError::Surface {
                    backend: "wgpu",
                    detail: e.to_string(),
                }),
            )
        })?;
        let pipeline = Self::build_pipeline(
            &self.instance,
            &wgpu_surface,
            size,
            &self.render_profile,
            true,
        )?;
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

        if let Some(p) = frame.palette_as_rgba8() {
            pipeline.update_palette_texture(&p);
        }
        if self.osd_dirty {
            crate::renderer::setup::set_osd_layer(pipeline, self.osd.clone());
            self.osd_dirty = false;
        }

        match pipeline.render(surface, self.size, frame.as_ref(), frame.stride()) {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                RenderResult::Presented
            }
            Ok(RenderOutcome::Skipped) => RenderResult::Skipped,
            Ok(RenderOutcome::RecreateSurface) => {
                self.last_render_error = None;
                RenderResult::Skipped
            }
            Err(e) => {
                if self.last_render_error.as_deref() != Some(e.as_str()) {
                    log::error!("wgpu: {e}");
                }
                self.last_render_error = Some(e.to_string());
                RenderResult::Error
            }
        }
    }
}

impl Present for WgpuRenderer {
    fn size(&self) -> SurfaceSize {
        SurfaceLifecycle::size(self)
    }

    fn attach(
        &mut self,
        wh: RawWindowHandle,
        dh: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        SurfaceLifecycle::attach(self, wh, dh, size)
    }

    fn detach(&mut self) {
        SurfaceLifecycle::detach(self);
    }

    fn resize(&mut self, size: SurfaceSize) {
        SurfaceLifecycle::resize(self, size);
    }

    fn render(&mut self, frame: &FrameBuffer) -> RenderResult {
        FrameSink::render(self, frame)
    }
}

impl OsdSink for WgpuRenderer {
    fn set_osd_layer(&mut self, layer: Option<OsdLayer>) {
        self.osd = layer;
        self.osd_dirty = true;
    }
}

impl StripSink for WgpuRenderer {
    fn set_reserved_strip(&mut self, strip: ReservedStrip) {
        if let Some(ref mut pipeline) = self.pipeline {
            pipeline.set_reserved_strip(strip);
        }
    }
}

impl ProfileSwitch for WgpuRenderer {
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        let Some(ref surface) = self.surface else {
            return Err(RendererError::not_attached("update"));
        };
        self.pipeline = Some(Self::build_pipeline(
            &self.instance,
            surface,
            self.size,
            profile,
            true,
        )?);
        self.reapply_osd();
        Ok(())
    }
}

impl GpuRenderer for WgpuRenderer {
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError> {
        ProfileSwitch::update_render_profile(self, profile)
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

impl GpuFactory for WgpuFactory {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        _display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError> {
        let instance = surface::default_instance();

        // Create a headless device + queue.  The pipeline and surface are
        // created in attach() where we have actual window handles.
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| {
            RendererError::new(
                "request adapter",
                Box::new(AttachError::Adapter {
                    backend: "wgpu",
                    detail: e.to_string(),
                }),
            )
        })?;
        let limits = Self::device_limit_profile().required_limits();
        let (_device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("nerust_wgpu_device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                ..Default::default()
            }))
            .map_err(|e| {
                RendererError::new(
                    "device",
                    Box::new(AttachError::Device {
                        backend: "wgpu",
                        detail: e.to_string(),
                    }),
                )
            })?;

        Ok(Box::new(WgpuRenderer {
            instance,
            render_profile: config.render_profile.clone(),
            pipeline: None,
            surface: None,
            size: SurfaceSize::new(0, 0),
            last_render_error: None,
            osd: None,
            osd_dirty: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lifecycle_split_seams_hold() {
        use nerust_render_traits::renderer::{
            FrameSink, GpuRenderer, Present, ProfileSwitch, SurfaceLifecycle,
        };
        fn assert_seams<T: SurfaceLifecycle + FrameSink + ProfileSwitch + Present + GpuRenderer>() {
        }
        assert_seams::<super::WgpuRenderer>();
    }

    #[test]
    fn attach_errors_name_backend_and_stage() {
        use nerust_render_traits::renderer::AttachError;
        let err = AttachError::Adapter {
            backend: "wgpu",
            detail: "no adapter".to_string(),
        };
        assert!(err.to_string().contains("wgpu"));
        assert!(err.to_string().contains("adapter"));
        let err = AttachError::Pipeline {
            backend: "wgpu",
            detail: "compile failed".to_string(),
        };
        assert!(err.to_string().contains("pipeline"));
        assert!(err.to_string().contains("compile failed"));
        let err = AttachError::Surface {
            backend: "wgpu",
            detail: "bad surface".to_string(),
        };
        assert!(err.to_string().contains("surface"));
        let err = AttachError::Device {
            backend: "wgpu",
            detail: "lost".to_string(),
        };
        assert!(err.to_string().contains("device"));
        let err = AttachError::Context {
            backend: "wgpu",
            detail: "gone".to_string(),
        };
        assert!(err.to_string().contains("context"));
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
    fn detached_renderer_skips_and_refuses_profile_switch() {
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
        // Profile switch without a surface names the missing attach.
        let profile = test_profile();
        let err = renderer.update_render_profile(&profile).unwrap_err();
        assert!(err.to_string().contains("not attached"), "{err}");
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
        use nerust_render_traits::renderer::{AttachError, RendererError};
        for (stage, err) in [
            (
                "request adapter",
                AttachError::Adapter {
                    backend: "wgpu",
                    detail: "x".to_string(),
                },
            ),
            (
                "device",
                AttachError::Device {
                    backend: "wgpu",
                    detail: "x".to_string(),
                },
            ),
            (
                "surface",
                AttachError::Surface {
                    backend: "wgpu",
                    detail: "x".to_string(),
                },
            ),
            (
                "pipeline",
                AttachError::Pipeline {
                    backend: "wgpu",
                    detail: "x".to_string(),
                },
            ),
        ] {
            let wrapped = RendererError::new(stage, Box::new(err));
            assert!(wrapped.to_string().contains(stage), "{wrapped}");
        }
        assert!(
            RendererError::not_attached("update")
                .to_string()
                .contains("update")
        );
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
