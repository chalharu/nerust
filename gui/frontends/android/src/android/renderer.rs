use super::surface::SurfaceTarget;
use nerust_backend_wgpu::{RenderResult, WgpuBackend};
use nerust_gui_shell::session::SessionHandle;
use nerust_screen_wgpu::{
    renderer::{DeviceLimitProfile, PresentationOptions},
    surface::SurfaceSize,
};
use std::sync::Arc;
use winit::window::Window;

pub(crate) struct WgpuRenderer {
    backend: WgpuBackend<SurfaceTarget>,
}

impl WgpuRenderer {
    pub(crate) fn new(window: Arc<Window>, session: &SessionHandle) -> Result<Self, String> {
        let snapshot = session.snapshot();
        let profile = snapshot
            .video_profile
            .expect("session should publish a render profile");
        let size = window.inner_size();
        let backend = WgpuBackend::new_with_device_limit_profile(
            SurfaceTarget::new(window.clone()),
            SurfaceSize::new(size.width, size.height),
            &profile,
            DeviceLimitProfile::DownlevelWebGl2,
            PresentationOptions {
                vsync: session.settings_snapshot().local.video.presentation.vsync,
            },
        )?;
        Ok(Self { backend })
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
        if session.palette_updated() {
            session.with_palette_rgba8(|rgba| {
                self.backend.update_palette_texture(rgba);
            });
        }
        let mut result: Option<RenderResult> = None;
        session.with_frame_buffer(&mut |bytes| {
            result = Some(self.backend.render(bytes, window_size));
        });
        result.unwrap_or(RenderResult::Skipped)
    }
}
