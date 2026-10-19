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

    /// Renderer exists but no surface is attached yet.
    pub fn not_attached(op: &'static str) -> Self {
        Self::new(op, Box::new(RendererErrorKind::NotAttached))
    }

    /// Surface exists but its context is not current on this thread.
    pub fn not_current(op: &'static str) -> Self {
        Self::new(op, Box::new(RendererErrorKind::NotCurrent))
    }
}

/// Typed renderer faults. Replaces `OpaqueError("")` sentinels with
/// diagnosable variants; free-form messages stay on `OpaqueError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererErrorKind {
    NotAttached,
    NotCurrent,
    NoConfig,
}

impl std::fmt::Display for RendererErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAttached => write!(f, "renderer not attached"),
            Self::NotCurrent => write!(f, "GL context not current"),
            Self::NoConfig => write!(f, "no GL config"),
        }
    }
}

impl std::error::Error for RendererErrorKind {}

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

impl OsdLayer {
    /// Cheap content equality: header check before the byte memcmp.
    pub fn same_content(&self, other: Option<&Self>) -> bool {
        let Some(other) = other else {
            return false;
        };
        self.x == other.x
            && self.y == other.y
            && self.width == other.width
            && self.height == other.height
            && self.rgba.len() == other.rgba.len()
            && self.rgba == other.rgba
    }

    /// Option-level equality for layer caches on both sides.
    pub fn same_option(a: Option<&Self>, b: Option<&Self>) -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(x), Some(y)) => x.same_content(Some(y)),
            (None, Some(_)) | (Some(_), None) => false,
        }
    }
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

/// Base OSD font size from the client height. Single source for
/// the rasterizer and strip reservation; the 6px floor keeps tiny
/// surfaces legible on both sides.
pub fn osd_font_px(client_height: u32, ui_scale: f32) -> u32 {
    if client_height == 0 {
        return 0;
    }
    (f64::from(client_height) * 0.022 * f64::from(ui_scale))
        .ceil()
        .max(6.0) as u32
}

/// Strip reservation for `rows` of OSD pills. Rounds up where the
/// rasterizer truncates (`font_px * 16 / 10`), so the reservation
/// always covers the layer.
pub fn strip_pixels(client_height: u32, rows: u32, ui_scale: f32) -> u32 {
    if rows == 0 || client_height == 0 {
        return 0;
    }
    let font_px = osd_font_px(client_height, ui_scale);
    let row = (font_px * 16).div_ceil(10) + (4.0 * ui_scale).round() as u32;
    let margin = (8.0 * ui_scale).round() as u32;
    let gap = (4.0 * ui_scale).round() as u32;
    rows.saturating_mul(row) + margin + gap * rows.saturating_sub(1)
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
    /// OSD upload attempts before the layer drops. Per-config knob so
    /// dead-driver tolerance varies without recompiling the renderer.
    pub max_osd_upload_retries: u8,
}

/// Default OSD upload attempts. Single source for call-site literals.
pub const DEFAULT_OSD_UPLOAD_RETRIES: u8 = 3;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_pixels_matches_raster_floor_at_scale_one() {
        assert_eq!(strip_pixels(1000, 1, 1.0), 48);
        assert_eq!(strip_pixels(1000, 2, 1.0), 92);
        assert_eq!(strip_pixels(0, 1, 1.0), 0);
        assert_eq!(strip_pixels(1000, 0, 1.0), 0);
    }

    #[test]
    fn strip_pixels_halves_with_ui_scale() {
        // 640px surface at 0.5: font 8px, rows stay covered.
        assert_eq!(osd_font_px(640, 0.5), 8);
        assert_eq!(strip_pixels(640, 1, 0.5), 19);
    }

    #[test]
    fn layer_equality_gates_on_header_before_bytes() {
        let base = OsdLayer {
            x: 1,
            y: 2,
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        assert!(OsdLayer::same_option(None, None));
        assert!(base.same_content(Some(&base.clone())));
        assert!(!base.same_content(None));
        assert!(!OsdLayer::same_option(Some(&base), None));
        let shifted = OsdLayer {
            x: 9,
            ..base.clone()
        };
        assert!(!base.same_content(Some(&shifted)));
        let mut edited = base.clone();
        edited.rgba[7] = 9;
        assert!(!base.same_content(Some(&edited)));
    }
}
