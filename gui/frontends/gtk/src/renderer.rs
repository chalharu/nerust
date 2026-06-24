use nerust_backend_opengl::GlRendererFactory;
use nerust_screen_video::{
    FrameBuffer, Renderer, RendererConfig, RendererFactory, SurfaceSize, VideoRenderProfile,
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
        physical_size: SurfaceSize,
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
            Ok(view) => {
                let mut view = view;
                view.reconfigure(physical_size);
                self.view = Some(view);
            }
            Err(e) => log::error!("GtkRenderer: failed to create GlRenderer: {e}"),
        }
    }

    pub(crate) fn render(&mut self, frame_buffer: &FrameBuffer) {
        if let Some(view) = self.view.as_mut() {
            view.render(frame_buffer);
        }
    }

    pub(crate) fn reconfigure(&mut self, size: SurfaceSize) {
        if let Some(view) = self.view.as_mut() {
            view.reconfigure(size);
        }
    }
}
