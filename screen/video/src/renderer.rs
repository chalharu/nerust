use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::{FrameBuffer, SurfaceSize, VideoRenderProfile};

/// Wraps a static or formatted message as an `std::error::Error`.
#[derive(Debug)]
pub struct OpaqueError(pub String);

impl std::fmt::Display for OpaqueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for OpaqueError {}

/// Renderer-related error.
#[derive(Debug)]
pub struct RendererError {
    context: &'static str,
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl RendererError {
    pub fn new(context: &'static str, source: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self { context, source }
    }
}

impl std::fmt::Display for RendererError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.context, self.source)
    }
}

impl std::error::Error for RendererError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

/// Outcome reported by [`GpuRenderer::render`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderResult {
    Presented,
    Skipped,
    Error,
}

/// GPU device + pipeline + surface.  Single unified trait.
///
/// Lifecycle: GpuFactory::create_renderer() → attach() → render() → detach() → drop
pub trait GpuRenderer: std::fmt::Debug {
    /// Current output size.
    fn size(&self) -> SurfaceSize;

    /// Attach a native window.  Must be called before render().
    fn attach(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError>;

    /// Detach from the native window (device/pipeline are kept).
    fn detach(&mut self);

    /// detach + attach (handy for Android surface loss or Wayland).
    fn reattach(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.detach();
        self.attach(window_handle, display_handle, size)
    }

    /// Resize the output (swapchain for wgpu; stored for GL).
    fn resize(&mut self, size: SurfaceSize);

    /// Update the render pipeline for a new render profile.
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError>;

    /// Render a frame.  attach() must have been called.
    fn render(&mut self, frame: &FrameBuffer) -> RenderResult;
}

/// Common parameters for [`GpuFactory::create_renderer`].
pub struct RendererConfig {
    pub initial_size: SurfaceSize,
    pub render_profile: VideoRenderProfile,
    pub vsync: bool,
}

/// Abstract factory: creates a [`GpuRenderer`].
///
/// `display_handle` is required by OpenGL (glutin::Display::new).
/// wgpu ignores it and creates a headless device.
/// Call `attach()` on the result before rendering.
pub trait GpuFactory {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError>;
}
