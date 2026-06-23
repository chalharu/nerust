use nerust_backend_opengl::GlRenderer;
use nerust_screen_video::{FrameBuffer, Renderer, SurfaceSize, VideoRenderProfile};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

#[derive(Debug)]
pub(crate) struct GtkGlRenderer {
    view: Option<GlRenderer>,
}

impl GtkGlRenderer {
    pub(crate) fn new() -> Self {
        Self { view: None }
    }

    pub(crate) fn realize(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
        profile: &VideoRenderProfile,
    ) {
        match GlRenderer::new(window_handle, display_handle, size, profile) {
            Ok(view) => self.view = Some(view),
            Err(e) => log::error!("GtkGlRenderer: failed to create GlRenderer: {e}"),
        }
    }

    pub(crate) fn render(&mut self, frame_buffer: &FrameBuffer) {
        if let Some(view) = self.view.as_mut() {
            view.render(frame_buffer);
        }
    }
}
