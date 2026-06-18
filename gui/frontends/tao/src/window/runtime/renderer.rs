use crate::surface::SurfaceTarget;
use nerust_backend_wgpu::{RenderResult, WgpuBackend};
use nerust_gui_shell::session::SessionHandle;
use nerust_screen_wgpu::{renderer::PresentationOptions, surface::SurfaceSize};
use std::sync::Arc;
use tao::window::Window as TaoWindow;

pub(crate) struct WgpuRenderer {
    backend: WgpuBackend<SurfaceTarget>,
}

impl WgpuRenderer {
    pub(crate) fn new(window: Arc<TaoWindow>, session: &SessionHandle) -> Self {
        let snapshot = session.snapshot();
        let profile = snapshot
            .video_profile
            .expect("session should publish a render profile");
        let backend = WgpuBackend::new(
            SurfaceTarget::new(window.clone(), session.window_size()),
            SurfaceSize::new(window.inner_size().width, window.inner_size().height),
            &profile,
            PresentationOptions {
                vsync: session.settings_snapshot().local.video.presentation.vsync,
            },
        )
        .unwrap();
        Self { backend }
    }

    pub(crate) fn reconfigure(&mut self, window_size: SurfaceSize) {
        self.backend.reconfigure(window_size);
    }

    pub(crate) fn render(
        &mut self,
        session: &mut SessionHandle,
        window_size: SurfaceSize,
    ) -> RenderResult {
        session.swap_frame_buffer();
        self.backend.render(session.frame_buffer(), window_size)
    }
}
