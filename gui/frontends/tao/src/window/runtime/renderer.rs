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
        log::info!(
            "tao renderer init: profile logical={:?} physical={:?} window_inner={:?} session_window={:?}",
            profile.logical_size,
            profile.physical_size,
            window.inner_size(),
            session.window_size()
        );
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
        session: &SessionHandle,
        window_size: SurfaceSize,
    ) -> RenderResult {
        let snapshot = session.snapshot();
        let frame = snapshot
            .video_frame
            .expect("session should publish a video frame");
        let non_black_pixels = frame
            .bytes()
            .chunks_exact(4)
            .filter(|pixel| {
                let pixel = *pixel;
                pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 || pixel[3] != 255
            })
            .count();
        log::info!(
            "tao renderer update: frame_counter={} frame_len={} non_black_pixels={} window_size={:?}",
            snapshot.metrics.frame_counter,
            frame.bytes().len(),
            non_black_pixels,
            window_size
        );
        self.backend.render(frame.bytes(), window_size)
    }
}
