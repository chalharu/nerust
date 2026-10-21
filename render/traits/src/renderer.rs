use std::sync::Arc;

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
    kind: RendererErrorKind,
    source: Box<dyn std::error::Error + Send + Sync>,
}

impl RendererError {
    pub fn new(context: &'static str, source: Box<dyn std::error::Error + Send + Sync>) -> Self {
        Self {
            context,
            kind: RendererErrorKind::Other,
            source,
        }
    }

    /// Kind-carrying constructor: one `RenderError` type with source plus kind.
    pub fn with_kind(
        context: &'static str,
        kind: RendererErrorKind,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self {
            context,
            kind,
            source,
        }
    }

    /// Machine-readable fault class for logs and tests.
    pub fn kind(&self) -> RendererErrorKind {
        self.kind
    }

    /// Typed attach fault: replaces `AttachError` string details.
    pub fn attach(
        backend: &'static str,
        stage: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    ) -> Self {
        Self::with_kind(
            "attach",
            RendererErrorKind::Attach { backend, stage },
            source,
        )
    }

    /// Renderer exists but no surface is attached yet.
    pub fn not_attached(op: &'static str) -> Self {
        Self::with_kind(
            op,
            RendererErrorKind::NotAttached,
            Box::new(RendererErrorKind::NotAttached),
        )
    }

    /// Surface exists but its context is not current on this thread.
    pub fn not_current(op: &'static str) -> Self {
        Self::with_kind(
            op,
            RendererErrorKind::NotCurrent,
            Box::new(RendererErrorKind::NotCurrent),
        )
    }
}

/// Typed renderer faults. Replaces `OpaqueError("")` sentinels with
/// diagnosable variants; free-form messages stay on `OpaqueError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererErrorKind {
    NotAttached,
    NotCurrent,
    NoConfig,
    /// Backend attach failure at `stage` (context/surface/adapter/device/pipeline).
    Attach {
        backend: &'static str,
        stage: &'static str,
    },
    /// OSD texture upload failure at `stage`.
    Upload {
        stage: &'static str,
    },
    /// Invalid surface or viewport geometry.
    InvalidSurface,
    /// Catch-all for sources without a dedicated variant.
    Other,
}

impl std::fmt::Display for RendererErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAttached => write!(f, "renderer not attached"),
            Self::NotCurrent => write!(f, "GL context not current"),
            Self::NoConfig => write!(f, "no GL config"),
            Self::Attach { backend, stage } => {
                write!(f, "{backend} {stage} failed")
            }
            Self::Upload { stage } => write!(f, "OSD upload failed at {stage}"),
            Self::InvalidSurface => write!(f, "invalid surface size"),
            Self::Other => write!(f, "renderer error"),
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

/// In-frame OSD layer, cropped over the game image.
/// Clones share immutable pixels and their construction-time digest.
///
/// Pixels cannot be replaced or mutated through the public API:
/// ```compile_fail
/// use nerust_render_traits::renderer::OsdLayer;
/// let mut layer = OsdLayer::new(0, 0, 1, 1, vec![0; 4]);
/// layer.rgba = vec![255; 4].into();
/// ```
/// ```compile_fail
/// use nerust_render_traits::renderer::OsdLayer;
/// let mut layer = OsdLayer::new(0, 0, 1, 1, vec![0; 4]);
/// layer.rgba()[0] = 255;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsdLayer {
    /// Top-left corner in physical surface pixels.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Row-major straight-alpha RGBA, `width * height * 4` bytes.
    rgba: Arc<[u8]>,
    payload_hash: u64,
}

impl OsdLayer {
    /// Freeze the pixels and compute their digest once.
    pub fn new(x: u32, y: u32, width: u32, height: u32, rgba: Vec<u8>) -> Self {
        let payload_hash = content_hash(&rgba);
        Self {
            x,
            y,
            width,
            height,
            rgba: Arc::from(rgba),
            payload_hash,
        }
    }

    /// Read-only row-major straight-alpha RGBA pixels.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Construction-time pixel digest, independent of mutable geometry.
    pub fn content_hash(&self) -> u64 {
        self.payload_hash
    }

    /// Exact visual equality: shared pixels skip hashing and byte comparison;
    /// distinct buffers use the digest only to reject, never to prove equality.
    pub fn same_content(&self, other: Option<&Self>) -> bool {
        let Some(other) = other else {
            return false;
        };
        self.x == other.x
            && self.y == other.y
            && self.width == other.width
            && self.height == other.height
            && (Arc::ptr_eq(&self.rgba, &other.rgba)
                || (self.payload_hash == other.payload_hash && self.rgba == other.rgba))
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

/// FNV-1a hash over staged bytes; empty input hashes to zero (unset).
fn content_hash(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let mut hash: u64 = 0x0cbf_29ce_4842_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
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

/// One raster pill row: tag plus body in a single color.
/// Port-owned DTO; the rasterizer and the shell share this type.
#[derive(Debug, Clone)]
pub struct OsdRow {
    pub tag: std::sync::Arc<str>,
    pub text: std::sync::Arc<str>,
    pub rgb: (u8, u8, u8),
}

/// Status readout: label plus right-aligned value.
#[derive(Debug, Clone)]
pub struct OsdStatus {
    pub label: std::sync::Arc<str>,
    pub value: std::sync::Arc<str>,
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

impl OsdRow {
    /// Shared tag handle; clones the `Arc` without reallocating.
    pub fn tag_shared(&self) -> std::sync::Arc<str> {
        self.tag.clone()
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewInsets {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

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

/// Outcome reported by [`FrameSink::render`].
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderResult {
    Presented,
    Skipped,
    Error,
}

/// Explicit detach + attach for surface loss or Wayland.
/// Free function so the side-effectful sequence never hides in a default.
pub fn reattach_surface(
    lifecycle: &mut (impl SurfaceLifecycle + ?Sized),
    window_handle: RawWindowHandle,
    display_handle: RawDisplayHandle,
    size: SurfaceSize,
) -> Result<(), RendererError> {
    lifecycle.detach();
    lifecycle.attach(window_handle, display_handle, size)
}

/// Surface ownership: attach/detach/resize without any frame vocabulary.
pub trait SurfaceLifecycle: std::fmt::Debug {
    /// Current output size.
    fn size(&self) -> SurfaceSize;

    /// Attach a native window. Must be called before render().
    /// The native window/display owner must stay alive until detach, and
    /// calls must obey the window system's thread requirements. Raw handles
    /// do not transfer ownership or permit resurrection after native teardown.
    fn attach(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError>;

    /// Detach before destroying the native window. Backends may discard GPU resources.
    fn detach(&mut self);

    /// Resize the output (swapchain for wgpu; stored for GL).
    fn resize(&mut self, size: SurfaceSize);
}

/// Per-frame blit without any lifecycle vocabulary.
pub trait FrameSink: std::fmt::Debug {
    /// Render a frame. attach() must have been called.
    fn render(&mut self, frame: &FrameBuffer) -> RenderResult;
}

/// Render-pipeline switch without any surface vocabulary.
pub trait ProfileSwitch: std::fmt::Debug {
    /// Update the render pipeline for a new render profile.
    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError>;
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

/// Bounded OSD upload budget shared by every backend.
/// Floor of one attempt: zero never drops live layers at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadBudget {
    max: u8,
    fails: u8,
}

impl UploadBudget {
    /// Mint a budget from the config knob; the floor is one attempt.
    pub fn new(max: u8) -> Self {
        Self {
            max: max.max(1),
            fails: 0,
        }
    }
}

impl OsdUploadPolicy for UploadBudget {
    fn record_failure(&mut self) -> bool {
        self.fails = self.fails.saturating_add(1);
        self.fails >= self.max
    }

    fn reset(&mut self) {
        self.fails = 0;
    }
}

/// Fold one OSD upload attempt into the slot and budget.
/// Drops a repeatedly rejected layer so identical pushes re-stage.
pub fn note_osd_upload(
    accepted: bool,
    slot: &mut Option<OsdLayer>,
    budget: &mut UploadBudget,
) -> bool {
    if accepted {
        budget.reset();
        return false;
    }
    if budget.record_failure() {
        slot.take();
        return true;
    }
    false
}
/// Per-use-site pipeline composition without the legacy composites.
/// Synthetic marker: implement the five legs, never this trait directly.
pub trait RenderPipeline:
    SurfaceLifecycle + FrameSink + OsdSink + StripSink + ProfileSwitch
{
}

impl<T> RenderPipeline for T where
    T: SurfaceLifecycle + FrameSink + OsdSink + StripSink + ProfileSwitch
{
}

/// GPU device + pipeline + surface. Composition of the three sinks.
pub struct RendererConfig {
    pub render_profile: VideoRenderProfile,
    pub vsync: bool,
    /// OSD upload attempts before the layer drops. Per-config knob so
    /// dead-driver tolerance varies without recompiling the renderer.
    pub max_osd_upload_retries: u8,
}

/// Default OSD upload attempts. Single source for call-site literals.
pub const DEFAULT_OSD_UPLOAD_RETRIES: u8 = 3;

impl RendererConfig {
    /// Upload budget with the one-attempt floor: zero clamps instead of
    /// dropping live layers on the first transient failure.
    pub fn upload_budget(&self) -> u8 {
        self.max_osd_upload_retries.max(1)
    }
}

/// Narrow factory: returns the per-use-site [`RenderPipeline`] composition.
pub trait PipelineFactory: std::fmt::Debug {
    /// Create a pipeline; call `attach()` on the result before rendering.
    fn create_pipeline(
        &self,
        config: &RendererConfig,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn RenderPipeline>, RendererError>;
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
        let mut bytes = base.rgba().to_vec();
        bytes[7] = 9;
        let edited = OsdLayer::new(1, 2, 2, 1, bytes);
        assert_ne!(base.content_hash(), edited.content_hash());
        assert!(!base.same_content(Some(&edited)));
    }

    #[test]
    fn layer_equality_skips_memcmp_for_shared_handles() {
        let base = OsdLayer::new(0, 0, 2, 1, vec![9; 8]);
        let shared = base.clone();
        assert!(Arc::ptr_eq(&base.rgba, &shared.rgba));
        assert!(base.same_content(Some(&shared)));
    }

    #[test]
    fn layer_equality_compares_distinct_buffers() {
        // Same bytes, distinct allocations.
        let first = OsdLayer::new(0, 0, 2, 1, vec![9; 8]);
        let second = OsdLayer::new(0, 0, 2, 1, vec![9; 8]);
        assert!(!Arc::ptr_eq(&first.rgba, &second.rgba));
        assert_eq!(first.content_hash(), second.content_hash());
        assert!(first.same_content(Some(&second)));
        assert!(OsdLayer::same_option(Some(&first), Some(&second)));
        // Empty input hashes to zero (unset), never a false fast-path.
        let empty = OsdLayer::new(0, 0, 0, 0, vec![]);
        assert_eq!(empty.content_hash(), 0);
    }

    #[test]
    fn matching_digest_does_not_prove_byte_equality() {
        let first = OsdLayer::new(0, 0, 2, 1, vec![9; 8]);
        let mut collision = OsdLayer::new(0, 0, 2, 1, vec![8; 8]);
        // Private test seam simulates a digest collision without exposing mutation.
        collision.payload_hash = first.content_hash();
        assert!(!first.same_content(Some(&collision)));
        assert!(!OsdLayer::same_option(Some(&first), Some(&collision)));
    }

    #[test]
    fn geometry_changes_preserve_the_immutable_payload_and_digest() {
        let base = OsdLayer::new(0, 0, 2, 1, vec![9; 8]);
        let mut changed = base.clone();
        changed.x = 1;
        assert!(!base.same_content(Some(&changed)));
        assert!(Arc::ptr_eq(&base.rgba, &changed.rgba));
        assert_eq!(changed.rgba(), &[9; 8]);
        assert_eq!(changed.content_hash(), content_hash(changed.rgba()));
        assert_eq!(base.content_hash(), changed.content_hash());
        changed.x = base.x;
        changed.width = 1;
        changed.height = 2;
        assert!(!base.same_content(Some(&changed)));
    }

    #[test]
    fn layer_payload_is_thread_safe() {
        // Layers are Send + Sync: no Rc payload crosses threads.
        fn assert_thread_safe<T: Send + Sync>() {}
        assert_thread_safe::<OsdLayer>();
        assert_thread_safe::<OsdRow>();
        assert_thread_safe::<OsdStatus>();
        assert_thread_safe::<OsdContent>();
    }

    #[test]
    fn attach_error_carries_kind_and_source() {
        let err = RendererError::attach(
            "softbuffer",
            "surface",
            Box::new(std::io::Error::other("no display")),
        );
        assert_eq!(
            err.kind(),
            RendererErrorKind::Attach {
                backend: "softbuffer",
                stage: "surface"
            }
        );
        assert!(err.to_string().contains("no display"));
        let err = RendererError::with_kind(
            "upload",
            RendererErrorKind::Upload { stage: "blit" },
            Box::new(RendererErrorKind::Upload { stage: "blit" }),
        );
        assert_eq!(err.kind(), RendererErrorKind::Upload { stage: "blit" });
        assert!(err.to_string().contains("upload"));
        let err = RendererError::with_kind(
            "size",
            RendererErrorKind::InvalidSurface,
            Box::new(RendererErrorKind::InvalidSurface),
        );
        assert_eq!(
            err.kind().to_string(),
            RendererErrorKind::InvalidSurface.to_string()
        );
    }

    #[test]
    fn config_budget_floors_at_one() {
        let config = RendererConfig {
            render_profile: crate::VideoRenderProfile {
                source_logical_size: crate::logical::LogicalSize {
                    width: 64,
                    height: 64,
                },
                logical_size: crate::logical::LogicalSize {
                    width: 64,
                    height: 64,
                },
                physical_size: crate::physical::PhysicalSize {
                    width: 64.0,
                    height: 64.0,
                },
                frame_format: crate::VideoFrameFormat::Rgba,
                ntsc_packed_rgba8: None,
            },
            vsync: true,
            max_osd_upload_retries: 0,
        };
        // Zero clamps to one attempt, never drops live layers at once.
        assert_eq!(config.upload_budget(), 1);
        assert_eq!(DEFAULT_OSD_UPLOAD_RETRIES, 3);
    }

    #[test]
    fn shared_budget_counts_then_drops_and_rearms() {
        let mut budget = UploadBudget::new(2);
        assert!(!budget.record_failure());
        assert!(budget.record_failure());
        // Saturating counter never wraps on a dead driver.
        assert!(budget.record_failure());
        budget.reset();
        assert!(!budget.record_failure());
        // Zero config never drops live layers at once.
        let mut floored = UploadBudget::new(0);
        assert!(floored.record_failure());
    }

    #[test]
    fn upload_fold_drops_rejected_layers_and_rearms_on_success() {
        let layer = OsdLayer::new(0, 0, 2, 1, vec![9; 8]);
        let mut slot = Some(layer);
        let mut budget = UploadBudget::new(2);
        assert!(!note_osd_upload(false, &mut slot, &mut budget));
        assert!(slot.is_some());
        assert!(note_osd_upload(false, &mut slot, &mut budget));
        assert!(slot.is_none());
        // Success re-arms: a fresh push survives one blip again.
        slot = Some(OsdLayer::new(0, 0, 2, 1, vec![9; 8]));
        assert!(!note_osd_upload(true, &mut slot, &mut budget));
        assert!(slot.is_some());
        assert!(!note_osd_upload(false, &mut slot, &mut budget));
        assert!(slot.is_some());
    }

    #[test]
    fn reattach_helper_sequences_detach_then_attach() {
        #[derive(Debug, Default)]
        struct Order {
            log: Vec<&'static str>,
            fail_attach: bool,
        }
        impl SurfaceLifecycle for Order {
            fn size(&self) -> SurfaceSize {
                SurfaceSize::new(1, 1)
            }
            fn attach(
                &mut self,
                _w: RawWindowHandle,
                _d: RawDisplayHandle,
                _s: SurfaceSize,
            ) -> Result<(), RendererError> {
                self.log.push("attach");
                if self.fail_attach {
                    return Err(RendererError::not_attached("attach"));
                }
                Ok(())
            }
            fn detach(&mut self) {
                self.log.push("detach");
            }
            fn resize(&mut self, _size: SurfaceSize) {}
        }
        // Never dereferenced: the stub records order without touching hardware.
        let window: RawWindowHandle = unsafe { std::mem::zeroed() };
        let display: RawDisplayHandle = unsafe { std::mem::zeroed() };
        let mut order = Order::default();
        reattach_surface(&mut order, window, display, SurfaceSize::new(2, 2)).unwrap();
        assert_eq!(order.log, vec!["detach", "attach"]);
        // A failed attach still detached first: the sequence is explicit.
        let mut failing = Order {
            log: Vec::new(),
            fail_attach: true,
        };
        assert!(reattach_surface(&mut failing, window, display, SurfaceSize::new(2, 2)).is_err());
        assert_eq!(failing.log, vec!["detach", "attach"]);
    }

    #[test]
    fn pipeline_composition_covers_narrow_traits() {
        #[derive(Debug, Default)]
        struct Stub;
        impl SurfaceLifecycle for Stub {
            fn size(&self) -> SurfaceSize {
                SurfaceSize::new(1, 1)
            }
            fn attach(
                &mut self,
                _w: RawWindowHandle,
                _d: RawDisplayHandle,
                _s: SurfaceSize,
            ) -> Result<(), RendererError> {
                Ok(())
            }
            fn detach(&mut self) {}
            fn resize(&mut self, _size: SurfaceSize) {}
        }
        impl FrameSink for Stub {
            fn render(&mut self, _frame: &FrameBuffer) -> RenderResult {
                RenderResult::Skipped
            }
        }
        impl OsdSink for Stub {
            fn set_osd_layer(&mut self, _layer: Option<OsdLayer>) {}
        }
        impl StripSink for Stub {}
        impl ProfileSwitch for Stub {
            fn update_render_profile(
                &mut self,
                _profile: &VideoRenderProfile,
            ) -> Result<(), RendererError> {
                Ok(())
            }
        }
        fn assert_pipeline<T: RenderPipeline>() {}
        assert_pipeline::<Stub>();
        // Narrow traits compose as trait objects for per-use-site callers.
        fn lifecycle_only(_: &mut dyn SurfaceLifecycle) {}
        fn sink_only(_: &mut dyn FrameSink) {}
        fn pipeline(_: &mut dyn RenderPipeline) {}
        let _ = (lifecycle_only, sink_only, pipeline);
    }

    #[test]
    fn anchor_edges_classify_corners() {
        assert!(OsdAnchor::TopLeft.is_top() && OsdAnchor::TopLeft.is_left());
        assert!(OsdAnchor::TopRight.is_top() && !OsdAnchor::TopRight.is_left());
        assert!(!OsdAnchor::BottomLeft.is_top() && OsdAnchor::BottomLeft.is_left());
        assert!(!OsdAnchor::BottomRight.is_top() && !OsdAnchor::BottomRight.is_left());
        assert!(OsdAnchor::TopLeft.same_edge(OsdAnchor::TopRight));
        assert!(OsdAnchor::BottomLeft.same_edge(OsdAnchor::BottomRight));
        assert!(!OsdAnchor::TopLeft.same_edge(OsdAnchor::BottomLeft));
        assert_eq!(OsdAnchor::default(), OsdAnchor::BottomLeft);
    }

    #[test]
    fn font_and_error_helpers_cover_edges() {
        assert_eq!(osd_font_px(0, 1.0), 0);
        assert_eq!(osd_font_px(720, 1.0), 16);
        let err = RendererError::not_attached("attach");
        assert!(err.to_string().contains("attach"));
        assert_eq!(err.kind(), RendererErrorKind::NotAttached);
        let err = RendererError::not_current("present");
        assert!(err.to_string().contains("present"));
        assert_eq!(err.kind(), RendererErrorKind::NotCurrent);
        assert_eq!(
            RendererErrorKind::NotAttached.to_string(),
            "renderer not attached"
        );
        assert_eq!(RendererErrorKind::NoConfig.to_string(), "no GL config");
        assert_eq!(
            RendererErrorKind::Attach {
                backend: "b",
                stage: "s"
            }
            .to_string(),
            "b s failed"
        );
        assert!(
            RendererErrorKind::Upload { stage: "s" }
                .to_string()
                .contains('s')
        );
        assert!(!RendererErrorKind::Other.to_string().is_empty());
        assert!(
            RendererErrorKind::InvalidSurface
                .to_string()
                .contains("surface")
        );
        assert_eq!(DEFAULT_OSD_UPLOAD_RETRIES, 3);
    }
}
