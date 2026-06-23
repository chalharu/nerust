use nerust_screen_opengl::GlView;
use nerust_screen_video::{
    FrameBuffer, RenderResult, Renderer, SurfaceSize, VideoFrameFormat, VideoRenderProfile,
};
use std::os::raw::c_void;

/// App-facing OpenGL render backend.
///
/// This is the composition unit consumed by OpenGL-capable frontend hosts
/// (currently `gui/frontends/gtk`). It owns the [`GlView`] lifecycle and keeps
/// hosts free from any direct dependency on `nerust_screen_opengl`.
#[derive(Debug)]
pub struct GlBackend {
    view: GlView,
    expected_frame_len: usize,
}

impl GlBackend {
    /// Load OpenGL function pointers.
    ///
    /// Must be called with the GL context current, before the first
    /// [`on_load`](Self::on_load).
    pub fn load_with<F: FnMut(&'static str) -> *const c_void>(get_proc_address: F) {
        GlView::load_with(get_proc_address);
    }

    /// Create a new backend.
    ///
    /// GPU resources are not allocated until [`on_load`](Self::on_load).
    pub fn new() -> Self {
        Self {
            view: GlView::new(),
            expected_frame_len: usize::MAX,
        }
    }

    /// Enable or disable vertex array objects.
    ///
    /// Must be called before [`on_load`](Self::on_load).
    pub fn use_vao(&mut self, value: bool) {
        self.view.use_vao(value);
    }

    /// Allocate GPU resources for the given render profile.
    pub fn on_load(&mut self, render_profile: &VideoRenderProfile) -> Result<(), String> {
        self.view.on_load(render_profile)?;
        let frame_size = match render_profile.frame_format {
            VideoFrameFormat::Rgba => render_profile.logical_size,
            // Palette モードでは frame data は source_logical_size の 1bpp データ
            VideoFrameFormat::Palette => render_profile.source_logical_size,
        };
        let bpp = render_profile.frame_format.bytes_per_pixel();
        self.expected_frame_len = frame_size.width * frame_size.height * bpp;
        Ok(())
    }

    /// Release GPU resources.
    ///
    /// Must be called while the GL context is still current.
    pub fn on_close(&mut self) {
        self.view.on_close();
    }
}

impl Renderer for GlBackend {
    fn render(&mut self, frame_buffer: &FrameBuffer) -> RenderResult {
        if let Some(palette_rgba8) = frame_buffer.palette_as_rgba8() {
            self.view.update_palette_texture(&palette_rgba8);
        }
        let bytes = frame_buffer.as_ref();
        let bytes = bytes
            .get(..self.expected_frame_len)
            .expect("OpenGL backend expected a loaded frame buffer of the configured size");
        self.view.on_update(bytes.as_ptr());
        RenderResult::Presented
    }

    fn reconfigure(&mut self, size: SurfaceSize) {
        self.view
            .on_resize(1.0, 1.0, size.width as i32, size.height as i32);
    }
}

impl Default for GlBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::GlBackend;

    #[test]
    fn default_constructs_without_panic() {
        let _backend = GlBackend::default();
    }
}
