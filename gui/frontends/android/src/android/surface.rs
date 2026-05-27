use nerust_backend_wgpu::RenderSurfaceTarget;
use nerust_screen_wgpu::surface::SurfaceSize;
use std::sync::Arc;
use winit::raw_window_handle::{
    HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::window::Window;

pub(crate) struct SurfaceTarget {
    window: Arc<Window>,
}

impl SurfaceTarget {
    pub(crate) fn new(window: Arc<Window>) -> Self {
        Self { window }
    }
}

// Safety: `SurfaceTarget` owns an `Arc<Window>` and keeps the backing native
// objects alive for the lifetime of the render surface built from it.
unsafe impl RenderSurfaceTarget for SurfaceTarget {
    fn prepare(&self) {}

    fn surface_size(&self, fallback: SurfaceSize) -> SurfaceSize {
        let size = self.window.inner_size();
        if size.width > 0 && size.height > 0 {
            SurfaceSize::new(size.width, size.height)
        } else {
            SurfaceSize::new(fallback.width, fallback.height)
        }
    }

    fn raw_window_handle(&self) -> Result<RawWindowHandle, HandleError> {
        self.window.window_handle().map(|handle| handle.as_raw())
    }

    fn raw_display_handle(&self) -> Result<Option<RawDisplayHandle>, HandleError> {
        self.window
            .display_handle()
            .map(|handle| Some(handle.as_raw()))
    }
}
