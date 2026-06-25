use nerust_backend_opengl::GlRendererFactory as Factory;
use nerust_screen_video::{
    FrameBuffer, Renderer, RendererConfig, RendererFactory, Surface, SurfaceSize,
    VideoRenderProfile,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub(crate) struct GtkRenderer {
    renderer: Option<Box<dyn Renderer>>,
    surface: Option<Box<dyn Surface>>,
}

impl GtkRenderer {
    pub(crate) fn new() -> Self {
        Self {
            renderer: None,
            surface: None,
        }
    }

    pub(crate) fn realize(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        app_size: SurfaceSize,
        profile: &VideoRenderProfile,
    ) {
        // Drop old state: wgpu requires surface before renderer.
        drop(self.surface.take());
        drop(self.renderer.take());

        let config = RendererConfig {
            initial_size: app_size,
            render_profile: profile.clone(),
            vsync: true,
        };
        let factory = Factory::default();
        match factory.create_renderer(&config, window_handle, display_handle) {
            Ok(r) => {
                let s = factory.create_surface(r.as_ref(), window_handle, display_handle, app_size);
                match s {
                    Ok(s) => {
                        self.renderer = Some(r);
                        self.surface = Some(s);
                    }
                    Err(e) => log::error!("GtkRenderer: surface creation failed: {e}"),
                }
            }
            Err(e) => log::error!("GtkRenderer: renderer creation failed: {e}"),
        }
    }

    pub(crate) fn render(&mut self, frame_buffer: &FrameBuffer, window_size: SurfaceSize) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        // Keep the output surface sized to the current window.
        if surface.size() != window_size {
            surface.configure(window_size);
        }
        renderer.render(surface.as_ref(), frame_buffer);
    }
}
