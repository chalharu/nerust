#[cfg(feature = "opengl")]
use nerust_backend_opengl::GlFactory as Factory;
#[cfg(feature = "wgpu")]
use nerust_backend_wgpu::WgpuFactory as Factory;
use nerust_screen_video::{
    FrameBuffer, GpuFactory, GpuRenderer, RendererConfig, SurfaceSize, VideoRenderProfile,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub(crate) struct GtkRenderer {
    renderer: Option<Box<dyn GpuRenderer>>,
}

impl GtkRenderer {
    pub(crate) fn new() -> Self {
        Self { renderer: None }
    }

    pub(crate) fn realize(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        app_size: SurfaceSize,
        physical_size: SurfaceSize,
        profile: &VideoRenderProfile,
    ) {
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

    pub(crate) fn render(&mut self, frame_buffer: &FrameBuffer, window_size: SurfaceSize) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if renderer.size() != window_size {
            renderer.resize(window_size);
        }
        renderer.render(frame_buffer);
    }
}
