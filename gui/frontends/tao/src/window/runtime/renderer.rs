use crate::surface::SurfaceTarget;
use nerust_backend_wgpu::{RenderResult, WgpuBackend};
use nerust_gui_shell::session::NesSession;
use nerust_screen_wgpu::surface::SurfaceSize;
use std::sync::Arc;
use tao::window::Window as TaoWindow;

pub(crate) struct WgpuRenderer {
    backend: WgpuBackend<SurfaceTarget>,
}

impl WgpuRenderer {
    pub(crate) fn new(window: Arc<TaoWindow>, session: &NesSession) -> Self {
        let video = session.video();
        let backend = WgpuBackend::new(
            SurfaceTarget::new(window.clone(), session.window_size()),
            SurfaceSize::new(window.inner_size().width, window.inner_size().height),
            video.presentation(),
            video
                .console_video_assets()
                .expect("NES session always has video assets"),
        )
        .unwrap();
        Self { backend }
    }

    pub(crate) fn reconfigure(&mut self, window_size: SurfaceSize) {
        self.backend.reconfigure(window_size);
    }

    pub(crate) fn render(
        &mut self,
        session: &NesSession,
        window_size: SurfaceSize,
    ) -> RenderResult {
        session.with_frame_buffer(|frame_buffer| self.backend.render(frame_buffer, window_size))
    }
}
