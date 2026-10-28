//! Game-viewport abstractions. Widget chrome belongs to native controls;
//! the shared emulation frame belongs here, behind framework-owned traits.
//!
//! Concrete selection (wgpu/gl/softbuffer) is injected by the composition
//! root, never chosen by frontends. The render engine (`render/*`) imports
//! this vocabulary from here and re-exports it for existing users; new code
//! names `wakui::viewport` paths directly.
//!
//! Migration: `nerust_render_traits::{SurfaceSize, FrameBuffer, ...}` and
//! `nerust_render_traits::renderer::*` resolve to these same items through
//! engine-side re-exports. Frontend migration switches the import path only.

pub mod frame;
pub mod geometry;
pub mod renderer;

pub use frame::{
    FrameBuffer, PixelFormat, RenderedImage, ReservedStrip, VideoFrameFormat, VideoFrameHandle,
    VideoFrameSpec, VideoPresentation, VideoRenderProfile,
};
pub use geometry::{LogicalSize, PhysicalSize, SurfaceSize};
