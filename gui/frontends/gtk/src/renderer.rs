use std::rc::Rc;

use nerust_render_base::{
    FrameBuffer, SurfaceSize, VideoRenderProfile,
    renderer::{GpuFactory, GpuRenderer, OpaqueError, RendererConfig, RendererError},
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub(crate) struct GtkRenderer {
    factory: Rc<dyn GpuFactory>,
    renderer: Option<Box<dyn GpuRenderer>>,
    last_size: SurfaceSize,
}

impl GtkRenderer {
    pub(crate) fn new(factory: Rc<dyn GpuFactory>) -> Self {
        Self {
            factory,
            renderer: None,
            last_size: SurfaceSize::new(0, 0),
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
        };
        match self.factory.create_renderer(&config, display_handle) {
            Ok(mut r) => {
                if let Err(e) = r.attach(window_handle, display_handle, physical_size) {
                    log::error!("GtkRenderer: attach failed: {e}");
                }
                self.renderer = Some(r);
            }
            Err(e) => log::error!("GtkRenderer: create failed: {e}"),
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
            Some(r) => r.reattach(window_handle, display_handle, size),
            None => Err(RendererError::new(
                "reattach",
                Box::new(OpaqueError("no renderer".to_string())),
            )),
        }
    }

    pub(crate) fn render(&mut self, frame_buffer: &FrameBuffer, window_size: SurfaceSize) {
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
