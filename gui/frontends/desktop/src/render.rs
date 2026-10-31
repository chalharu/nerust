use nerust_gui_shell::{
    frame_composer::FrameComposer,
    session::osd::{OsdManager, OsdSyncInput},
};
use nerust_render_traits::{FrameBuffer, SurfaceSize, renderer::RenderPipeline};
use nerust_render_traits::{
    VideoRenderProfile,
    renderer::{DEFAULT_OSD_UPLOAD_RETRIES, PipelineFactory, RendererConfig},
};
use panechord::{ImageFrame, WindowSurface};
use std::{cell::Cell, rc::Rc};

pub(crate) struct RenderAdapter {
    pipeline: Option<Box<dyn RenderPipeline>>,
    profile: Option<(VideoRenderProfile, bool)>,
    window: Option<Rc<dyn WindowSurface>>,
    factory: Rc<dyn PipelineFactory>,
    viewport: Rc<Cell<SurfaceSize>>,
    logical_bounds: (f32, f32),
    composer: FrameComposer,
}

impl RenderAdapter {
    pub(crate) fn new(factory: Rc<dyn PipelineFactory>) -> Self {
        Self {
            pipeline: None,
            profile: None,
            window: None,
            factory,
            viewport: Rc::new(Cell::new(SurfaceSize::new(0, 0))),
            logical_bounds: (1.0, 1.0),
            composer: FrameComposer::new(),
        }
    }

    pub(crate) fn set_window(&mut self, window: Rc<dyn WindowSurface>) {
        self.close();
        self.window = Some(window);
    }

    pub(crate) fn toggle_fullscreen(&self) {
        if let Some(window) = &self.window {
            window.toggle_fullscreen();
        }
    }

    pub(crate) fn set_logical_bounds(&mut self, width: f32, height: f32) {
        self.logical_bounds = (width, height);
        let scale = self
            .window
            .as_ref()
            .map_or(1.0, |window| window.scale_factor());
        self.viewport.set(SurfaceSize::new(
            (f64::from(width) * scale).round() as u32,
            (f64::from(height) * scale).round() as u32,
        ));
    }

    pub(crate) fn capture(
        &mut self,
        frame: &FrameBuffer,
        profile: &VideoRenderProfile,
        vsync: bool,
        osd: &OsdManager,
        sync: OsdSyncInput,
    ) -> Result<Option<ImageFrame>, String> {
        let Some(window) = self.window.clone() else {
            return Ok(None);
        };
        let size = self.viewport.get();
        if size.width == 0 || size.height == 0 {
            return Ok(None);
        }
        let unchanged = self.profile.as_ref().is_some_and(|(known, known_vsync)| {
            *known_vsync == vsync && same_profile(known, profile)
        });
        if !unchanged {
            self.detach_pipeline();
            let display = window
                .display_handle()
                .map_err(|error| error.to_string())?
                .as_raw();
            let native = window
                .window_handle()
                .map_err(|error| error.to_string())?
                .as_raw();
            let mut pipeline = self
                .factory
                .create_pipeline(
                    &RendererConfig {
                        render_profile: profile.clone(),
                        vsync,
                        max_osd_upload_retries: DEFAULT_OSD_UPLOAD_RETRIES,
                    },
                    display,
                )
                .map_err(|error| error.to_string())?;
            if let Err(error) = pipeline.attach(native, display, size) {
                pipeline.detach();
                return Err(error.to_string());
            }
            self.pipeline = Some(pipeline);
            self.profile = Some((profile.clone(), vsync));
        }
        let Some(pipeline) = self.pipeline.as_mut() else {
            return Ok(None);
        };
        self.composer
            .render(pipeline.as_mut(), frame, size, osd, sync)?
            .map(|image| {
                ImageFrame::new(image.width, image.height, image.pixels.into())
                    .map_err(str::to_owned)
            })
            .transpose()
    }

    fn detach_pipeline(&mut self) {
        self.composer.invalidate();
        if let Some(mut pipeline) = self.pipeline.take() {
            pipeline.detach();
        }
        self.profile = None;
    }

    pub(crate) fn close(&mut self) {
        self.detach_pipeline();
        self.viewport.set(SurfaceSize::new(0, 0));
        self.window = None;
    }
}

fn same_profile(left: &VideoRenderProfile, right: &VideoRenderProfile) -> bool {
    left.source_logical_size == right.source_logical_size
        && left.logical_size == right.logical_size
        && left.physical_size.width == right.physical_size.width
        && left.physical_size.height == right.physical_size.height
        && left.frame_format == right.frame_format
        && left.ntsc_packed_rgba8 == right.ntsc_packed_rgba8
}

impl Drop for RenderAdapter {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_shell::session::osd::{OsdChrome, TickMetrics, ViewInsets};
    use nerust_render_traits::renderer::{
        DEFAULT_OSD_UPLOAD_RETRIES, PipelineFactory, RendererConfig,
    };
    use nerust_render_traits::{
        PixelFormat, VideoFrameFormat, VideoRenderProfile, logical::LogicalSize,
        physical::PhysicalSize,
    };
    use nerust_settings_core::model::{language::AppLanguage, local::NotificationSettings};
    use std::time::Instant;

    fn composition_fixture() -> (Box<dyn RenderPipeline>, FrameBuffer, OsdManager) {
        let profile = VideoRenderProfile {
            source_logical_size: LogicalSize {
                width: 256,
                height: 240,
            },
            logical_size: LogicalSize {
                width: 256,
                height: 240,
            },
            physical_size: PhysicalSize {
                width: 256.0,
                height: 240.0,
            },
            frame_format: VideoFrameFormat::Rgba,
            ntsc_packed_rgba8: None,
        };
        let pipeline = nerust_render_softbuffer::SoftbufferFactory
            .create_pipeline(
                &RendererConfig {
                    render_profile: profile,
                    vsync: false,
                    max_osd_upload_retries: DEFAULT_OSD_UPLOAD_RETRIES,
                },
                raw_window_handle::RawDisplayHandle::Xlib(
                    raw_window_handle::XlibDisplayHandle::new(None, 0),
                ),
            )
            .unwrap();
        let mut frame = FrameBuffer::with_capacity(256, 240, PixelFormat::Rgba);
        frame.resize(256, 240);
        (pipeline, frame, OsdManager::new())
    }

    fn sync_input(now: Instant, status_enabled: bool) -> OsdSyncInput {
        OsdSyncInput {
            tick: TickMetrics {
                fps: 60.0,
                paused: true,
                loaded: true,
                ..TickMetrics::default()
            },
            chrome: OsdChrome::from(&NotificationSettings {
                status_enabled,
                ..NotificationSettings::default()
            }),
            language: AppLanguage::English,
            now,
            width: 1,
            height: 1,
            ui_scale: 1.0,
            safe_insets: ViewInsets::default(),
        }
    }

    fn has_osd_pixels(image: &nerust_render_traits::RenderedImage) -> bool {
        image
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[..3] != [0, 0, 0])
    }

    #[test]
    fn compositor_draws_osd_in_renderer_pixels_and_clears_it() {
        let (mut pipeline, frame, osd) = composition_fixture();
        let mut composer = FrameComposer::new();
        let size = SurfaceSize::new(660, 496);
        let now = Instant::now();
        let quiet = composer
            .render(
                pipeline.as_mut(),
                &frame,
                size,
                &osd,
                sync_input(now, false),
            )
            .unwrap()
            .unwrap();
        assert!(!has_osd_pixels(&quiet));
        let paused = composer
            .render(pipeline.as_mut(), &frame, size, &osd, sync_input(now, true))
            .unwrap()
            .unwrap();
        assert!(has_osd_pixels(&paused));
        let cleared = composer
            .render(
                pipeline.as_mut(),
                &frame,
                size,
                &osd,
                sync_input(now, false),
            )
            .unwrap()
            .unwrap();
        assert_eq!(cleared.pixels, quiet.pixels);
    }

    #[test]
    fn compositor_resizes_full_client_and_preserves_osd() {
        let (mut pipeline, frame, osd) = composition_fixture();
        let mut composer = FrameComposer::new();
        let now = Instant::now();
        for size in [SurfaceSize::new(660, 496), SurfaceSize::new(548, 411)] {
            let image = composer
                .render(pipeline.as_mut(), &frame, size, &osd, sync_input(now, true))
                .unwrap()
                .unwrap();
            assert_eq!(pipeline.size(), size);
            assert_eq!((image.width, image.height), (size.width, size.height));
            assert!(has_osd_pixels(&image));
        }
    }

    #[test]
    fn compositor_resends_osd_to_replacement_renderer() {
        let (mut pipeline, frame, osd) = composition_fixture();
        let mut composer = FrameComposer::new();
        let size = SurfaceSize::new(660, 496);
        let now = Instant::now();
        let initial = composer
            .render(pipeline.as_mut(), &frame, size, &osd, sync_input(now, true))
            .unwrap()
            .unwrap();
        let (mut replacement, _, _) = composition_fixture();
        composer.invalidate();
        let restored = composer
            .render(
                replacement.as_mut(),
                &frame,
                size,
                &osd,
                sync_input(now, true),
            )
            .unwrap()
            .unwrap();
        assert!(has_osd_pixels(&restored));
        assert_eq!(restored.pixels, initial.pixels);
    }

    #[test]
    fn native_client_bounds_set_surface_size_without_view_feedback() {
        let mut renderer = RenderAdapter::new(Rc::new(nerust_render_softbuffer::SoftbufferFactory));
        renderer.set_logical_bounds(660.0, 496.0);
        assert_eq!(renderer.viewport.get(), SurfaceSize::new(660, 496));
        renderer.set_logical_bounds(0.0, 0.0);
        assert_eq!(renderer.viewport.get(), SurfaceSize::new(0, 0));
    }
}
