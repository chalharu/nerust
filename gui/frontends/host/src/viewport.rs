//! Framework viewport vocabulary: present outcomes and captured frames.
//! The render driver stays view-side; frontends provide only the content-area
//! region (`wakui::viewport::ViewportRegion`) and composite the outcome.

/// Renderer state after one present call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportPresent {
    Presented,
    NoPipeline,
    Skipped,
}

/// Composited frame for frontends that blend themselves: plain pixels,
/// no render vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewportImage {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: usize,
    pub pixels: Vec<u8>,
}
