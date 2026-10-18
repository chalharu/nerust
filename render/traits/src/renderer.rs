use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::{FrameBuffer, ReservedStrip, SurfaceSize, VideoRenderProfile};

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

/// In-frame OSD layer: cropped straight-alpha RGBA composited
/// over the game image at surface size. Origin is top-left.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsdLayer {
    /// Top-left corner in physical surface pixels.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Row-major straight-alpha RGBA, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// OSD anchor corner in render space. This is the rasterizer's own
/// vocabulary: settings positions map to it once at the shell
/// boundary, so the render crates never depend on settings types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OsdAnchor {
    TopLeft,
    TopRight,
    #[default]
    BottomLeft,
    BottomRight,
}

impl OsdAnchor {
    /// Top edge (top-left/top-right) vs bottom edge.
    pub fn is_top(self) -> bool {
        matches!(self, OsdAnchor::TopLeft | OsdAnchor::TopRight)
    }

    /// Left side (top-left/bottom-left): the value pill pins here.
    pub fn is_left(self) -> bool {
        matches!(self, OsdAnchor::TopLeft | OsdAnchor::BottomLeft)
    }

    /// Same horizontal band (both top or both bottom): entries share
    /// one edge, so opposite sides can share a single row.
    pub fn same_edge(self, other: Self) -> bool {
        self.is_top() == other.is_top()
    }
}

/// One raster pill row: tag plus body in a single color. Plain data;
/// shaping lives in the rasterizer crate.
#[derive(Debug, Clone)]
pub struct OsdRow {
    pub tag: String,
    pub text: String,
    pub rgb: (u8, u8, u8),
}

/// Status readout: label plus right-aligned value in a fixed-width
/// pill so digits never shift.
#[derive(Debug, Clone)]
pub struct OsdStatus {
    pub label: String,
    pub value: String,
    pub rgb: (u8, u8, u8),
}

/// Raster input: transient rows plus optional status, with corners.
#[derive(Debug, Clone, Default)]
pub struct OsdContent {
    pub rows: Vec<OsdRow>,
    pub status: Option<OsdStatus>,
    pub position: OsdAnchor,
    pub status_position: OsdAnchor,
}

/// Extra physical-pixel padding inside the surface edges, on top of
/// the layout margin. Outer layers set it for rounded corners,
/// notches, or gesture bars the rasterizer cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SafeInsets {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
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

    /// Reserve edge strips for OSD labels. The game viewport fits into
    /// the remaining rect on the next resize/render; strips stay clear.
    /// Default ignores the reservation (full-surface game).
    fn set_reserved_strip(&mut self, _strip: ReservedStrip) {}

    /// Overlay an OSD layer over the game image on the next render.
    /// Backends composite after filters at surface size; `None` clears.
    fn set_osd_layer(&mut self, layer: Option<OsdLayer>);

    /// Update the render pipeline for a new render profile.
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError>;

    /// Render a frame.  attach() must have been called.
    fn render(&mut self, frame: &FrameBuffer) -> RenderResult;
}

/// Common parameters for [`GpuFactory::create_renderer`].
pub struct RendererConfig {
    pub render_profile: VideoRenderProfile,
    pub vsync: bool,
}

/// Abstract factory: creates a [`GpuRenderer`].
///
/// `display_handle` is required by OpenGL (glutin::Display::new).
/// wgpu ignores it and creates a headless device.
/// Call `attach()` on the result before rendering.
pub trait GpuFactory: std::fmt::Debug {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError>;
}
