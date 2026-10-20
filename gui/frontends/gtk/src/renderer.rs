use std::rc::Rc;

use nerust_render_traits::{
    FrameBuffer, SurfaceSize, VideoRenderProfile,
    renderer::{GpuFactory, GpuRenderer, OpaqueError, RendererConfig, RendererError},
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub(crate) struct GtkRenderer {
    factory: Rc<dyn GpuFactory>,
    renderer: Option<Box<dyn GpuRenderer>>,
    last_size: SurfaceSize,
    /// Set only by a successful attach. `render` must check this instead of
    /// `renderer.is_some()`: rendering unattached segfaults in native code.
    attached: bool,
}

impl GtkRenderer {
    pub(crate) fn new(factory: Rc<dyn GpuFactory>) -> Self {
        Self {
            factory,
            renderer: None,
            last_size: SurfaceSize::new(0, 0),
            attached: false,
        }
    }

    pub(crate) fn realize(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        physical_size: SurfaceSize,
        profile: &VideoRenderProfile,
    ) {
        self.last_size = physical_size;
        drop(self.renderer.take());
        let config = RendererConfig {
            render_profile: profile.clone(),
            vsync: true,
            max_osd_upload_retries: nerust_render_traits::renderer::DEFAULT_OSD_UPLOAD_RETRIES,
        };
        match self.factory.create_renderer(&config, display_handle) {
            Ok(mut r) => {
                match r.attach(window_handle, display_handle, physical_size) {
                    Ok(()) => {
                        self.attached = true;
                    }
                    Err(e) => {
                        self.attached = false;
                        log::error!("GtkRenderer: attach failed: {e}");
                    }
                }
                self.renderer = Some(r);
            }
            Err(e) => {
                self.attached = false;
                log::error!("GtkRenderer: create failed: {e}");
            }
        }
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
                let result = r.reattach(window_handle, display_handle, size);
                self.attached = result.is_ok();
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

    pub(crate) fn render(&mut self, frame_buffer: &FrameBuffer, window_size: SurfaceSize) {
        if !self.attached {
            return;
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if self.last_size != window_size {
            renderer.resize(window_size);
            self.last_size = window_size;
        }
        renderer.render(frame_buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::GtkRenderer;
    use nerust_render_traits::{
        FrameBuffer, SurfaceSize,
        renderer::{
            GpuFactory, GpuRenderer, OsdLayer, OsdSink, Present, RenderResult, RendererConfig,
            RendererError, StripSink,
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
    }

    impl Present for StubRenderer {
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
        fn render(&mut self, _frame: &FrameBuffer) -> RenderResult {
            self.renders.set(self.renders.get() + 1);
            RenderResult::Presented
        }
    }

    impl OsdSink for StubRenderer {
        fn set_osd_layer(&mut self, _layer: Option<OsdLayer>) {}
    }

    impl StripSink for StubRenderer {}

    impl GpuRenderer for StubRenderer {
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

    impl GpuFactory for StubFactory {
        fn create_renderer(
            &self,
            _config: &RendererConfig,
            _display: RawDisplayHandle,
        ) -> Result<Box<dyn GpuRenderer>, RendererError> {
            if self.fail {
                return Err(RendererError::new(
                    "test",
                    Box::new(std::io::Error::other("no renderer")),
                ));
            }
            Ok(Box::new(StubRenderer {
                renders: Rc::clone(&self.renders),
                fail_attach: self.fail_attach,
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
        let factory_obj: Rc<dyn GpuFactory> = factory.clone();
        let mut gtk = GtkRenderer::new(factory_obj);
        let (wh, dh) = dummy_handles();
        gtk.realize(wh, dh, SurfaceSize::new(64, 64), &test_profile());
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
        let factory_obj: Rc<dyn GpuFactory> = Rc::new(StubFactory {
            renders: Rc::clone(&renders),
            fail: false,
            fail_attach: false,
        });
        let mut gtk = GtkRenderer::new(factory_obj);
        let (wh, dh) = dummy_handles();
        gtk.realize(wh, dh, SurfaceSize::new(64, 64), &test_profile());
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
        let factory_obj: Rc<dyn GpuFactory> = Rc::new(StubFactory {
            renders: Rc::clone(&renders),
            fail: false,
            fail_attach: true,
        });
        let mut gtk = GtkRenderer::new(factory_obj);
        let (wh, dh) = dummy_handles();
        gtk.realize(wh, dh, SurfaceSize::new(64, 64), &test_profile());
        // Attach refused: renders stay inert and reattach reports it.
        gtk.render(&test_frame(), SurfaceSize::new(64, 64));
        assert_eq!(renders.get(), 0);
        assert!(gtk.reattach(wh, dh, SurfaceSize::new(64, 64)).is_err());
    }
}
