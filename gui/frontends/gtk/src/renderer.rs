use std::rc::Rc;

use nerust_render_traits::{
    FrameBuffer, SurfaceSize, VideoRenderProfile,
    renderer::{
        OpaqueError, PipelineFactory, RenderPipeline, RenderResult, RendererConfig, RendererError,
        reattach_surface,
    },
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub(crate) struct GtkRenderer {
    factory: Rc<dyn PipelineFactory>,
    renderer: Option<Box<dyn RenderPipeline>>,
    last_size: SurfaceSize,
    /// Set only by a successful attach. `render` must check this instead of
    /// `renderer.is_some()`: rendering unattached segfaults in native code.
    attached: bool,
    output_generation: u64,
}

impl GtkRenderer {
    pub(crate) fn new(factory: Rc<dyn PipelineFactory>) -> Self {
        Self {
            factory,
            renderer: None,
            last_size: SurfaceSize::new(0, 0),
            attached: false,
            output_generation: 0,
        }
    }

    pub(crate) fn realize(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        physical_size: SurfaceSize,
        profile: &VideoRenderProfile,
    ) -> Result<(), RendererError> {
        self.last_size = physical_size;
        self.attached = false;
        drop(self.renderer.take());
        let config = RendererConfig {
            render_profile: profile.clone(),
            vsync: true,
            max_osd_upload_retries: nerust_render_traits::renderer::DEFAULT_OSD_UPLOAD_RETRIES,
        };
        let mut renderer = self.factory.create_pipeline(&config, display_handle)?;
        renderer.attach(window_handle, display_handle, physical_size)?;
        self.renderer = Some(renderer);
        self.attached = true;
        self.output_generation = self.output_generation.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn attached(&self) -> bool {
        self.attached
    }

    pub(crate) fn output_generation(&self) -> u64 {
        self.output_generation
    }

    pub(crate) fn reattach(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.last_size = size;
        match self.renderer.as_mut() {
            Some(r) => {
                let result = reattach_surface(r.as_mut(), window_handle, display_handle, size);
                self.attached = result.is_ok();
                if self.attached {
                    self.output_generation = self.output_generation.wrapping_add(1);
                }
                result
            }
            None => {
                self.attached = false;
                Err(RendererError::new(
                    "reattach",
                    Box::new(OpaqueError("no renderer".to_string())),
                ))
            }
        }
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

    pub(crate) fn render(
        &mut self,
        frame_buffer: &FrameBuffer,
        window_size: SurfaceSize,
    ) -> RenderResult {
        if !self.attached {
            return RenderResult::Skipped;
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return RenderResult::Skipped;
        };
        if self.last_size != window_size {
            renderer.resize(window_size);
            self.last_size = window_size;
        }
        let result = renderer.render(frame_buffer);
        if matches!(result, RenderResult::Error) {
            self.attached = false;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::GtkRenderer;
    use nerust_render_traits::{
        FrameBuffer, SurfaceSize,
        renderer::{
            FrameSink, OsdLayer, OsdSink, PipelineFactory, ProfileSwitch, RenderPipeline,
            RenderResult, RendererConfig, RendererError, StripSink, SurfaceLifecycle,
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
        renders: Rc<Cell<usize>>,
        fail_attach: bool,
        lose_first_frame: bool,
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
        fn render(&mut self, _frame: &FrameBuffer) -> RenderResult {
            self.renders.set(self.renders.get() + 1);
            if self.lose_first_frame && self.renders.get() == 1 {
                return RenderResult::Error;
            }
            RenderResult::Presented
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
        renders: Rc<Cell<usize>>,
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
                renders: Rc::clone(&self.renders),
                fail_attach: self.fail_attach,
                lose_first_frame: false,
            }))
        }
    }

    fn test_frame() -> FrameBuffer {
        FrameBuffer::with_capacity(64, 64, nerust_render_traits::PixelFormat::Rgba)
    }

    #[test]
    fn failed_create_leaves_renderer_detached() {
        let factory = Rc::new(StubFactory {
            renders: Rc::new(Cell::new(0)),
            fail: true,
            fail_attach: false,
        });
        let factory_obj: Rc<dyn PipelineFactory> = factory.clone();
        let mut gtk = GtkRenderer::new(factory_obj);
        let (wh, dh) = dummy_handles();
        assert!(
            gtk.realize(wh, dh, SurfaceSize::new(64, 64), &test_profile())
                .is_err()
        );
        // Create failed: render and reattach refuse without touching a stub.
        gtk.render(&test_frame(), SurfaceSize::new(64, 64));
        assert_eq!(factory.renders.get(), 0);
        assert!(gtk.reattach(wh, dh, SurfaceSize::new(64, 64)).is_err());
        gtk.set_osd_layer(None);
        gtk.set_reserved_strip(nerust_render_traits::ReservedStrip::default());
    }

    #[test]
    fn successful_realize_drives_renders_and_resizes() {
        let renders = Rc::new(Cell::new(0));
        let factory_obj: Rc<dyn PipelineFactory> = Rc::new(StubFactory {
            renders: Rc::clone(&renders),
            fail: false,
            fail_attach: false,
        });
        let mut gtk = GtkRenderer::new(factory_obj);
        let (wh, dh) = dummy_handles();
        assert!(
            gtk.realize(wh, dh, SurfaceSize::new(64, 64), &test_profile())
                .is_ok()
        );
        gtk.render(&test_frame(), SurfaceSize::new(64, 64));
        assert_eq!(renders.get(), 1);
        // Size change resizes before rendering.
        gtk.render(&test_frame(), SurfaceSize::new(128, 128));
        assert_eq!(renders.get(), 2);
        assert!(gtk.reattach(wh, dh, SurfaceSize::new(64, 64)).is_ok());
        gtk.set_osd_layer(Some(OsdLayer::new(0, 0, 2, 1, vec![255; 8])));
        gtk.set_reserved_strip(nerust_render_traits::ReservedStrip::default());
    }

    #[test]
    fn failed_attach_leaves_renderer_detached() {
        let renders = Rc::new(Cell::new(0));
        let factory_obj: Rc<dyn PipelineFactory> = Rc::new(StubFactory {
            renders: Rc::clone(&renders),
            fail: false,
            fail_attach: true,
        });
        let mut gtk = GtkRenderer::new(factory_obj);
        let (wh, dh) = dummy_handles();
        assert!(
            gtk.realize(wh, dh, SurfaceSize::new(64, 64), &test_profile())
                .is_err()
        );
        // Attach refused: renders stay inert and reattach reports it.
        gtk.render(&test_frame(), SurfaceSize::new(64, 64));
        assert_eq!(renders.get(), 0);
        assert!(gtk.reattach(wh, dh, SurfaceSize::new(64, 64)).is_err());
    }

    #[test]
    fn unchanged_size_recovers_after_create_and_attach_failures() {
        #[derive(Debug)]
        struct RecoveringFactory {
            attempts: Cell<usize>,
            renders: Rc<Cell<usize>>,
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
                    renders: self.renders.clone(),
                    fail: attempt == 0,
                    fail_attach: attempt == 1,
                }
                .create_pipeline(config, display)
            }
        }
        let renders = Rc::new(Cell::new(0));
        let mut renderer = GtkRenderer::new(Rc::new(RecoveringFactory {
            attempts: Cell::new(0),
            renders: renders.clone(),
        }));
        let (wh, dh) = dummy_handles();
        let size = SurfaceSize::new(64, 64);
        for _ in 0..2 {
            assert!(renderer.realize(wh, dh, size, &test_profile()).is_err());
            assert!(!renderer.attached());
            assert_eq!(renderer.output_generation(), 0);
            renderer.render(&test_frame(), size);
            assert_eq!(renders.get(), 0);
        }
        renderer.realize(wh, dh, size, &test_profile()).unwrap();
        assert!(renderer.attached());
        assert_eq!(renderer.output_generation(), 1);
        renderer.render(&test_frame(), size);
        assert_eq!(renders.get(), 1);
        renderer.reattach(wh, dh, size).unwrap();
        assert_eq!(renderer.output_generation(), 2);
        renderer.render(&test_frame(), size);
        assert_eq!(renders.get(), 2);
    }

    #[test]
    fn surface_loss_forces_real_attach_retry_at_unchanged_size() {
        #[derive(Debug)]
        struct LossFactory {
            creates: Rc<Cell<usize>>,
            renders: Rc<Cell<usize>>,
        }
        impl PipelineFactory for LossFactory {
            fn create_pipeline(
                &self,
                _: &RendererConfig,
                _: RawDisplayHandle,
            ) -> Result<Box<dyn RenderPipeline>, RendererError> {
                self.creates.set(self.creates.get() + 1);
                Ok(Box::new(StubRenderer {
                    renders: self.renders.clone(),
                    fail_attach: false,
                    lose_first_frame: true,
                }))
            }
        }
        let creates = Rc::new(Cell::new(0));
        let renders = Rc::new(Cell::new(0));
        let mut renderer = GtkRenderer::new(Rc::new(LossFactory {
            creates: creates.clone(),
            renders,
        }));
        let (wh, dh) = dummy_handles();
        let size = SurfaceSize::new(64, 64);
        renderer.realize(wh, dh, size, &test_profile()).unwrap();
        assert_eq!(renderer.render(&test_frame(), size), RenderResult::Error);
        assert!(!renderer.attached());
        assert_eq!(renderer.render(&test_frame(), size), RenderResult::Skipped);
        // Same condition as SurfaceCore::tick: detached triggers realize, even
        // without a resize or explicit settings reload.
        if !renderer.attached() {
            renderer.realize(wh, dh, size, &test_profile()).unwrap();
        }
        assert_eq!(creates.get(), 2);
        assert_eq!(renderer.output_generation(), 2);
        assert_eq!(
            renderer.render(&test_frame(), size),
            RenderResult::Presented
        );
    }
}
