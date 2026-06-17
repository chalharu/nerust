use nerust_console::video::VideoRenderProfile;
use nerust_screen_opengl::GlView;
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
        let logical_size = render_profile.logical_size;
        let bpp = render_profile.frame_format.bytes_per_pixel();
        self.expected_frame_len = logical_size.width * logical_size.height * bpp;
        Ok(())
    }

    /// Upload `frame_buffer` to the GPU and draw a frame.
    pub fn on_update(&self, frame_buffer: &[u8]) {
        let frame_buffer = frame_buffer
            .get(..self.expected_frame_len)
            .expect("OpenGL backend expected a loaded frame buffer of the configured size");
        self.view.on_update(frame_buffer.as_ptr());
    }

    /// Handle a viewport resize.
    pub fn on_resize(&mut self, scale_x: f32, scale_y: f32, width: i32, height: i32) {
        self.view.on_resize(scale_x, scale_y, width, height);
    }

    /// Release GPU resources.
    ///
    /// Must be called while the GL context is still current.
    pub fn on_close(&mut self) {
        self.view.on_close();
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
        // Verify that constructing GlBackend without a GL context does not
        // immediately panic (GPU resources are deferred to on_load).
        let _backend = GlBackend::default();
    }
}
