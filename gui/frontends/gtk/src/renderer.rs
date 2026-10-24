use std::rc::Rc;

use nerust_render_traits::{
    FrameBuffer, RenderedImage, SurfaceSize, VideoRenderProfile,
    renderer::{PipelineFactory, RenderPipeline, RendererConfig, RendererError},
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub(crate) struct GtkRenderer {
    factory: Rc<dyn PipelineFactory>,
    renderer: Option<Box<dyn RenderPipeline>>,
    /// Set only by a successful setup. Image renders must check this
    /// instead of `renderer.is_some()`.
    attached: bool,
    output_generation: u64,
}

impl GtkRenderer {
    pub(crate) fn new(factory: Rc<dyn PipelineFactory>) -> Self {
        Self {
            factory,
            renderer: None,
            attached: false,
            output_generation: 0,
        }
    }

    pub(crate) fn attached(&self) -> bool {
        self.attached
    }

    pub(crate) fn output_generation(&self) -> u64 {
        self.output_generation
    }

    /// Ensure a pipeline exists and its (unused) surface is attached. The
    /// swapchain is never presented: frames leave through
    /// [`Self::render_image`] for the GTK picture to composite, so no
    /// native pixels here can bury the in-window menubar.
    pub(crate) fn ensure_ready(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        profile: &VideoRenderProfile,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.attached = false;
        drop(self.renderer.take());
        let config = RendererConfig {
            render_profile: profile.clone(),
            vsync: true,
            max_osd_upload_retries: nerust_render_traits::renderer::DEFAULT_OSD_UPLOAD_RETRIES,
        };
        let mut renderer = self.factory.create_pipeline(&config, display_handle)?;
        renderer.attach(window_handle, display_handle, size)?;
        self.renderer = Some(renderer);
        self.attached = true;
        self.output_generation = self.output_generation.wrapping_add(1);
        Ok(())
    }

    /// Render one frame into memory; `None` skips the frame without error.
    /// The caller hands the bytes to the GTK picture as a texture.
    pub(crate) fn render_image(
        &mut self,
        frame: &FrameBuffer,
        size: SurfaceSize,
    ) -> Option<RenderedImage> {
        if !self.attached {
            return None;
        }
        let renderer = self.renderer.as_mut()?;
        renderer.render_image(frame, size)
    }

    pub(crate) fn set_osd_layer(
        &mut self,
        layer: Option<nerust_render_traits::renderer::OsdLayer>,
    ) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_osd_layer(layer);
        }
    }

    pub(crate) fn set_reserved_strip(&mut self, strip: nerust_render_traits::ReservedStrip) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_reserved_strip(strip);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GtkRenderer;
    use nerust_render_traits::{
        FrameBuffer, RenderedImage, SurfaceSize,
        renderer::{
            FrameSink, OsdLayer, OsdSink, PipelineFactory, ProfileSwitch, RenderPipeline,
            RendererConfig, RendererError, StripSink, SurfaceLifecycle,
        },
    };
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
    use std::{cell::Cell, num::NonZeroIsize, rc::Rc};

    fn dummy_handles() -> (RawWindowHandle, RawDisplayHandle) {
        (
            RawWindowHandle::Win32(raw_window_handle::Win32WindowHandle::new(
                NonZeroIsize::new(1).unwrap(),
            )),
            RawDisplayHandle::Xlib(raw_window_handle::XlibDisplayHandle::new(None, 0)),
        )
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

    #[derive(Debug)]
    struct StubRenderer {
        images: Rc<Cell<usize>>,
        blank: bool,
        fail_attach: bool,
    }

    impl SurfaceLifecycle for StubRenderer {
        fn size(&self) -> SurfaceSize {
            SurfaceSize::new(64, 64)
        }
        fn attach(
            &mut self,
            _wh: RawWindowHandle,
            _dh: RawDisplayHandle,
            _size: SurfaceSize,
        ) -> Result<(), RendererError> {
            if self.fail_attach {
                return Err(RendererError::new(
                    "test",
                    Box::new(std::io::Error::other("attach refused")),
                ));
            }
            Ok(())
        }
        fn detach(&mut self) {}
        fn resize(&mut self, _size: SurfaceSize) {}
    }

    impl FrameSink for StubRenderer {
        fn render(&mut self, _frame: &FrameBuffer) -> nerust_render_traits::renderer::RenderResult {
            panic!("surface presents are retired: use render_image");
        }

        fn render_image(
            &mut self,
            _frame: &FrameBuffer,
            size: SurfaceSize,
        ) -> Option<RenderedImage> {
            self.images.set(self.images.get() + 1);
            if self.blank {
                return None;
            }
            Some(RenderedImage {
                width: size.width,
                height: size.height,
                stride_bytes: size.width as usize * 4,
                pixels: vec![255u8; size.width as usize * size.height as usize * 4],
            })
        }
    }

    impl OsdSink for StubRenderer {
        fn set_osd_layer(&mut self, _layer: Option<OsdLayer>) {}
    }

    impl StripSink for StubRenderer {}

    impl ProfileSwitch for StubRenderer {
        fn update_render_profile(
            &mut self,
            _profile: &nerust_render_traits::VideoRenderProfile,
        ) -> Result<(), RendererError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct StubFactory {
        images: Rc<Cell<usize>>,
        blank: bool,
        fail: bool,
        fail_attach: bool,
    }

    impl PipelineFactory for StubFactory {
        fn create_pipeline(
            &self,
            _config: &RendererConfig,
            _display: RawDisplayHandle,
        ) -> Result<Box<dyn RenderPipeline>, RendererError> {
            if self.fail {
                return Err(RendererError::new(
                    "test",
                    Box::new(std::io::Error::other("no renderer")),
                ));
            }
            Ok(Box::new(StubRenderer {
                images: Rc::clone(&self.images),
                blank: self.blank,
                fail_attach: self.fail_attach,
            }))
        }
    }

    fn test_frame() -> FrameBuffer {
        FrameBuffer::with_capacity(64, 64, nerust_render_traits::PixelFormat::Rgba)
    }

    fn stub_factory(
        images: &Rc<Cell<usize>>,
        blank: bool,
        fail: bool,
        fail_attach: bool,
    ) -> Rc<dyn PipelineFactory> {
        Rc::new(StubFactory {
            images: Rc::clone(images),
            blank,
            fail,
            fail_attach,
        })
    }

    #[test]
    fn failed_create_leaves_renderer_detached() {
        let images = Rc::new(Cell::new(0));
        let mut gtk = GtkRenderer::new(stub_factory(&images, false, true, false));
        let (wh, dh) = dummy_handles();
        assert!(
            gtk.ensure_ready(wh, dh, &test_profile(), SurfaceSize::new(64, 64))
                .is_err()
        );
        // Create failed: images refuse without touching a stub.
        assert!(
            gtk.render_image(&test_frame(), SurfaceSize::new(64, 64))
                .is_none()
        );
        assert_eq!(images.get(), 0);
        gtk.set_osd_layer(None);
        gtk.set_reserved_strip(nerust_render_traits::ReservedStrip::default());
    }

    #[test]
    fn successful_ensure_renders_sized_images() {
        let images = Rc::new(Cell::new(0));
        let mut gtk = GtkRenderer::new(stub_factory(&images, false, false, false));
        let (wh, dh) = dummy_handles();
        assert!(
            gtk.ensure_ready(wh, dh, &test_profile(), SurfaceSize::new(64, 64))
                .is_ok()
        );
        assert!(gtk.attached());
        let image = gtk
            .render_image(&test_frame(), SurfaceSize::new(8, 8))
            .expect("stub always renders");
        assert_eq!((image.width, image.height), (8, 8));
        assert_eq!(image.pixels.len(), 8 * 8 * 4);
        assert_eq!(images.get(), 1);
        gtk.set_osd_layer(Some(OsdLayer::new(0, 0, 2, 1, vec![255; 8])));
        gtk.set_reserved_strip(nerust_render_traits::ReservedStrip::default());
    }

    #[test]
    fn failed_attach_leaves_renderer_detached() {
        let images = Rc::new(Cell::new(0));
        let mut gtk = GtkRenderer::new(stub_factory(&images, false, false, true));
        let (wh, dh) = dummy_handles();
        assert!(
            gtk.ensure_ready(wh, dh, &test_profile(), SurfaceSize::new(64, 64))
                .is_err()
        );
        // Attach refused: images stay inert.
        assert!(
            gtk.render_image(&test_frame(), SurfaceSize::new(64, 64))
                .is_none()
        );
        assert_eq!(images.get(), 0);
    }

    #[test]
    fn blank_backend_frame_skips_without_detaching() {
        let images = Rc::new(Cell::new(0));
        let mut gtk = GtkRenderer::new(stub_factory(&images, true, false, false));
        let (wh, dh) = dummy_handles();
        gtk.ensure_ready(wh, dh, &test_profile(), SurfaceSize::new(64, 64))
            .unwrap();
        // A skipped backend frame is transient: the pipeline stays attached
        // and no generation bump happens.
        assert!(
            gtk.render_image(&test_frame(), SurfaceSize::new(64, 64))
                .is_none()
        );
        assert!(gtk.attached());
        assert_eq!(gtk.output_generation(), 1);
    }

    #[test]
    fn create_and_attach_failures_recover_on_retry() {
        #[derive(Debug)]
        struct RecoveringFactory {
            attempts: Cell<usize>,
            images: Rc<Cell<usize>>,
        }
        impl PipelineFactory for RecoveringFactory {
            fn create_pipeline(
                &self,
                config: &RendererConfig,
                display: RawDisplayHandle,
            ) -> Result<Box<dyn RenderPipeline>, RendererError> {
                let attempt = self.attempts.get();
                self.attempts.set(attempt + 1);
                StubFactory {
                    images: self.images.clone(),
                    blank: false,
                    fail: attempt == 0,
                    fail_attach: attempt == 1,
                }
                .create_pipeline(config, display)
            }
        }
        let images = Rc::new(Cell::new(0));
        let mut renderer = GtkRenderer::new(Rc::new(RecoveringFactory {
            attempts: Cell::new(0),
            images: images.clone(),
        }));
        let (wh, dh) = dummy_handles();
        let size = SurfaceSize::new(64, 64);
        for _ in 0..2 {
            assert!(
                renderer
                    .ensure_ready(wh, dh, &test_profile(), size)
                    .is_err()
            );
            assert!(!renderer.attached());
            assert_eq!(renderer.output_generation(), 0);
            assert!(renderer.render_image(&test_frame(), size).is_none());
            assert_eq!(images.get(), 0);
        }
        renderer
            .ensure_ready(wh, dh, &test_profile(), size)
            .unwrap();
        assert!(renderer.attached());
        assert_eq!(renderer.output_generation(), 1);
        assert!(renderer.render_image(&test_frame(), size).is_some());
        assert_eq!(images.get(), 1);
    }
}
