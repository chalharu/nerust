use nerust_backend_opengl::GlRendererFactory;
use nerust_screen_video::{
    FrameBuffer, Renderer, RendererConfig, RendererError, RendererFactory, SurfaceSize,
    VideoRenderProfile,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub(crate) struct GtkRenderer {
    view: Option<Box<dyn Renderer>>,
}

impl GtkRenderer {
    pub(crate) fn new() -> Self {
        Self { view: None }
    }

    pub(crate) fn realize(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        app_size: SurfaceSize,
        profile: &VideoRenderProfile,
    ) {
        // Drop the old renderer BEFORE creating a new one, so that the
        // old renderer's VAO/VBO glDelete* calls run in the old context
        // (which is still current) instead of corrupting the new context.
        self.view = None;
        let config = RendererConfig {
            initial_size: app_size,
            render_profile: profile.clone(),
            vsync: true,
        };
        match GlRendererFactory.create_renderer(&config, window_handle, display_handle) {
            Ok(view) => self.view = Some(view),
            Err(e) => log::error!("GtkRenderer: failed to create GlRenderer: {e}"),
        }
    }

    pub(crate) fn render(&mut self, frame_buffer: &FrameBuffer, window_size: SurfaceSize) {
        if let Some(view) = self.view.as_mut() {
            view.render(frame_buffer, window_size);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn reconfigure(&mut self, size: SurfaceSize) {
        if let Some(view) = self.view.as_mut() {
            view.reconfigure(size);
        }
    }

    pub(crate) fn recreate_surface(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        match self.view.as_mut() {
            Some(view) => view.recreate_surface(window_handle, display_handle, size),
            None => Err(RendererError::new(
                "recreate surface",
                Box::new(nerust_screen_video::OpaqueError(
                    "no renderer initialized".to_string(),
                )),
            )),
        }
    }
}
