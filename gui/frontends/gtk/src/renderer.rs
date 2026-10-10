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
    /// Set only by a successful attach. Rendering without an attached
    /// surface segfaults in native code (Xvfb has no working backend),
    /// so `render` must check this instead of `renderer.is_some()`.
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
