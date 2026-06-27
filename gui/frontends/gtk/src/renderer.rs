#[cfg(feature = "opengl")]
use nerust_backend_opengl::GlFactory as Factory;
#[cfg(feature = "wgpu")]
use nerust_backend_wgpu::WgpuFactory as Factory;
use nerust_screen_video::{
    FrameBuffer, GpuFactory, GpuRenderer, RendererConfig, RendererError, SurfaceSize,
    VideoRenderProfile,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub(crate) struct GtkRenderer {
    renderer: Option<Box<dyn GpuRenderer>>,
    last_size: SurfaceSize,
}

impl GtkRenderer {
    pub(crate) fn new() -> Self {
        Self {
            renderer: None,
            last_size: SurfaceSize::new(0, 0),
        }
    }

    pub(crate) fn realize(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        app_size: SurfaceSize,
        physical_size: SurfaceSize,
        profile: &VideoRenderProfile,
    ) {
        self.last_size = physical_size;
        drop(self.renderer.take());
        let config = RendererConfig {
            initial_size: app_size,
            render_profile: profile.clone(),
            vsync: true,
        };
        let factory = Factory::default();
        match factory.create_renderer(&config, display_handle) {
            Ok(mut r) => {
                if let Err(e) = r.attach(window_handle, display_handle, physical_size) {
                    log::error!("GtkRenderer: attach failed: {e}");
                }
                self.renderer = Some(r);
            }
            Err(e) => log::error!("GtkRenderer: create failed: {e}"),
        }
    }

    pub(crate) fn resize(&mut self, size: SurfaceSize) {
        self.last_size = size;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size);
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
                Box::new(nerust_screen_video::OpaqueError("no renderer".to_string())),
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
