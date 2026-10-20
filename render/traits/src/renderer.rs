use std::rc::Rc;

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

/// In-frame OSD layer, cropped over the game image. `rgba` is shared;
/// `generation` orders re-stages without a memcmp.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsdLayer {
    /// Top-left corner in physical surface pixels.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Row-major straight-alpha RGBA, `width * height * 4` bytes.
    pub rgba: Rc<[u8]>,
    /// Monotonic stage counter; zero means "unset" (struct literals, tests).
    pub generation: u64,
}

/// Next OSD stage counter. Wraps only after 2^64 stages.
static OSD_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl OsdLayer {
    /// Build a layer with a fresh generation for the equality fast path.
    pub fn new(x: u32, y: u32, width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self {
            x,
            y,
            width,
            height,
            rgba: Rc::from(rgba),
            generation: OSD_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Cheap content equality: identity, then generation, then header, then bytes.
    pub fn same_content(&self, other: Option<&Self>) -> bool {
        let Some(other) = other else {
            return false;
        };
        if Rc::ptr_eq(&self.rgba, &other.rgba)
            && self.x == other.x
            && self.y == other.y
            && self.width == other.width
            && self.height == other.height
        {
            return true;
        }
        if self.generation != 0 && other.generation != 0 && self.generation == other.generation {
            return true;
        }
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

/// OSD anchor corner in render space. The rasterizer's own vocabulary;
/// the render crates never depend on settings types.
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

/// Viewport edge padding in physical pixels, on top of the layout margin.
/// [`SafeInsets`] is the historical alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewInsets {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

/// Historical alias for [`ViewInsets`].
pub type SafeInsets = ViewInsets;

/// Base OSD font size from the client height. The 6px floor keeps tiny
/// surfaces legible on both sides.
pub fn osd_font_px(client_height: u32, ui_scale: f32) -> u32 {
    if client_height == 0 {
        return 0;
    }
    (f64::from(client_height) * 0.022 * f64::from(ui_scale))
        .ceil()
        .max(6.0) as u32
}

/// Strip reservation for `rows` of OSD pills. Rounds up where the rasterizer
/// truncates (`font_px * 16 / 10`), so the reservation always covers the layer.
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

/// Outcome reported by [`Present::render`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderResult {
    Presented,
    Skipped,
    Error,
}

/// Frame presentation: surface lifecycle plus per-frame blit.
/// Backends implement this without any OSD vocabulary.
pub trait Present: std::fmt::Debug {
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

    /// Render a frame.  attach() must have been called.
    fn render(&mut self, frame: &FrameBuffer) -> RenderResult;
}

/// OSD overlay sink: ownership moves in, never clones out.
pub trait OsdSink: std::fmt::Debug {
    /// Overlay an OSD layer over the game image on the next render.
    /// Backends composite after filters at surface size; `None` clears.
    fn set_osd_layer(&mut self, layer: Option<OsdLayer>);
}

/// Reserved-strip sink: the game viewport fits into the remaining rect.
pub trait StripSink: std::fmt::Debug {
    /// Reserve edge strips for OSD labels. Default ignores the reservation.
    fn set_reserved_strip(&mut self, _strip: ReservedStrip) {}
}

/// Single OSD upload policy for every backend.
/// Bounded retries: transient blips survive, dead drivers drop the layer.
pub trait OsdUploadPolicy: std::fmt::Debug {
    /// Record one failed upload. Returns true when the layer must drop.
    fn record_failure(&mut self) -> bool;
    /// A successful upload re-arms the budget.
    fn reset(&mut self);
}

/// Single policy implementation: floor of one attempt, saturating counter.
#[derive(Debug)]
pub struct BoundedUploadPolicy {
    max: u8,
    fails: u8,
}

impl BoundedUploadPolicy {
    /// Zero would drop live layers on the first transient failure, so
    /// the floor is one attempt: the constructor clamps, not callers.
    pub fn new(max: u8) -> Self {
        Self {
            max: max.max(1),
            fails: 0,
        }
    }
}

impl OsdUploadPolicy for BoundedUploadPolicy {
    fn record_failure(&mut self) -> bool {
        self.fails = self.fails.saturating_add(1);
        self.fails >= self.max
    }

    fn reset(&mut self) {
        self.fails = 0;
    }
}

/// Grow-only byte staging for texture uploads.
/// One implementation (`Vec<u8>`) serves every backend; no steady allocs.
pub trait UploadCache {
    /// Grow-only staging of at least `need` bytes, zero-filled on growth.
    fn staging(&mut self, need: usize) -> &mut [u8];
}

impl UploadCache for Vec<u8> {
    fn staging(&mut self, need: usize) -> &mut [u8] {
        if self.len() < need {
            self.resize(need, 0);
        }
        &mut self[..need]
    }
}

/// GPU device + pipeline + surface. Composition of the three sinks.
/// Lifecycle: GpuFactory::create_renderer() → attach() → render() → detach() → drop
pub trait GpuRenderer: Present + OsdSink + StripSink {
    /// Update the render pipeline for a new render profile.
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError>;
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
        let base = OsdLayer::new(1, 2, 2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(OsdLayer::same_option(None, None));
        assert!(base.same_content(Some(&base.clone())));
        assert!(!base.same_content(None));
        assert!(!OsdLayer::same_option(Some(&base), None));
        let shifted = OsdLayer::new(9, 2, 2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(!base.same_content(Some(&shifted)));
        let mut bytes = base.rgba.to_vec();
        bytes[7] = 9;
        let edited = OsdLayer::new(1, 2, 2, 1, bytes);
        assert!(!base.same_content(Some(&edited)));
    }

    #[test]
    fn layer_generation_skips_memcmp_for_shared_handles() {
        let base = OsdLayer::new(0, 0, 2, 1, vec![9; 8]);
        let shared = base.clone();
        assert!(Rc::ptr_eq(&base.rgba, &shared.rgba));
        assert!(base.same_content(Some(&shared)));
    }
}
