//! Content-area handle seam. wakui exposes the drawable region's native
//! handle; the render engine attaches to it. Nothing more.
//!
//! Deliberate non-goal: pipeline, frame, and OSD vocabulary
//! (`PipelineFactory`, `FrameBuffer`, `VideoRenderProfile`, ...) stays in
//! the render engine (`nerust_render_traits`). A UI framework owns windows
//! and controls, never GPU pipelines — same split as winit versus wgpu.
//! `raw-window-handle` is the only window-system vocabulary wakui names.

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Drawable content area of one viewport: native handles plus pixel extent.
/// Produced by the native surface owner (frontend), consumed by the render
/// engine selected by the composition root. Plain data; no render types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportRegion {
    pub window: RawWindowHandle,
    pub display: RawDisplayHandle,
    pub width: u32,
    pub height: u32,
}

/// Provision contract for native surfaces (GTK `Picture` container, iced
/// window, ...). Implementors stay alive while the region is attached;
/// handles never outlive the native surface (see `SurfaceLifecycle` docs
/// on the render side for the attach/detach rules).
pub trait ViewportRegionProvider {
    fn viewport_region(&self) -> ViewportRegion;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeSurface {
        region: ViewportRegion,
    }

    fn test_region() -> ViewportRegion {
        // SAFETY: test-only constant handles; never attached to a real surface.
        let window: RawWindowHandle = unsafe { std::mem::zeroed() };
        let display: RawDisplayHandle = unsafe { std::mem::zeroed() };
        ViewportRegion {
            window,
            display,
            width: 960,
            height: 720,
        }
    }

    impl ViewportRegionProvider for FakeSurface {
        fn viewport_region(&self) -> ViewportRegion {
            self.region
        }
    }

    #[test]
    fn region_carries_handles_and_extent() {
        use std::mem::discriminant;
        let surface = FakeSurface {
            region: test_region(),
        };
        let region = surface.viewport_region();
        assert_eq!((region.width, region.height), (960, 720));
        // Zeroed test handles carry no identity; only the selected
        // variant must round-trip, never the pointer value.
        assert_eq!(
            discriminant(&region.window),
            discriminant(&test_region().window)
        );
        assert_eq!(
            discriminant(&region.display),
            discriminant(&test_region().display)
        );
    }
}
