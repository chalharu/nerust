use super::surface::SurfaceTarget;
use nerust_backend_wgpu::{RenderResult, WgpuBackend};
use nerust_gui_shell::session::SessionHandle;
use nerust_screen_wgpu::{renderer::PresentationOptions, surface::SurfaceSize};
use std::sync::Arc;
use winit::window::Window;

pub(crate) struct WgpuRenderer {
    backend: WgpuBackend<SurfaceTarget>,
}

impl WgpuRenderer {
    pub(crate) fn new(window: Arc<Window>, session: &SessionHandle) -> Self {
        let snapshot = session.snapshot();
        let profile = snapshot
            .video_profile
            .expect("session should publish a render profile");
        let size = window.inner_size();
        let backend = WgpuBackend::new(
            SurfaceTarget::new(window.clone()),
            SurfaceSize::new(size.width, size.height),
            &profile,
            PresentationOptions {
                vsync: session.settings_snapshot().local.video.presentation.vsync,
            },
        )
        .expect("Android wgpu renderer should build");
        Self { backend }
    }

    pub(crate) fn reconfigure(&mut self, window_size: SurfaceSize) {
        self.backend.reconfigure(window_size);
    }

    pub(crate) fn render(
        &mut self,
        session: &SessionHandle,
        window_size: SurfaceSize,
    ) -> RenderResult {
        let snapshot = session.snapshot();
        let frame = snapshot
            .video_frame
            .expect("session should publish a video frame");
        self.backend.render(frame.bytes(), window_size)
    }
}
