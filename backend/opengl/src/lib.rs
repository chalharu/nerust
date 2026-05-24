// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_screen_filter::presentation::ConsoleVideoAssets;
use nerust_screen_opengl::GlView;
use nerust_screen_video::VideoPresentation;
use std::os::raw::c_void;

/// App-facing OpenGL render backend.
///
/// This is the composition unit consumed by OpenGL-based shells
/// (`gui/frontends/glutin`, `gui/frontends/gtk`). It owns the [`GlView`]
/// lifecycle and keeps shells free from any
/// direct dependency on `nerust_screen_opengl`.
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

    /// Allocate GPU resources for the given presentation and console-family assets.
    ///
    /// Branches on the console variant; currently only NES is supported.
    pub fn on_load(
        &mut self,
        presentation: &VideoPresentation,
        assets: &ConsoleVideoAssets,
    ) -> Result<(), String> {
        let ConsoleVideoAssets::Nes(nes_assets) = assets;
        self.view.on_load(presentation, nes_assets)?;
        let source_logical_size = presentation.source_logical_size();
        self.expected_frame_len = source_logical_size.width * source_logical_size.height;
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
