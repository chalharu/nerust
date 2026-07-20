use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::{FrameBuffer, SurfaceSize, VideoRenderProfile};

#[derive(Debug)]
pub struct OpaqueError(pub String);

impl std::fmt::Display for OpaqueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for OpaqueError {}

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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RenderResult {
    Presented,
    Skipped,
    Error,
}

pub trait GpuRenderer: std::fmt::Debug {
    fn size(&self) -> SurfaceSize;

    fn attach(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError>;

    fn detach(&mut self);

    fn reattach(
        &mut self,
        window_handle: RawWindowHandle,
        display_handle: RawDisplayHandle,
        size: SurfaceSize,
    ) -> Result<(), RendererError> {
        self.detach();
        self.attach(window_handle, display_handle, size)
    }

    fn resize(&mut self, size: SurfaceSize);

    fn update_render_profile(&mut self, profile: &VideoRenderProfile) -> Result<(), RendererError>;

    fn render(&mut self, frame: &FrameBuffer) -> RenderResult;
}

pub struct RendererConfig {
    pub render_profile: VideoRenderProfile,
    pub vsync: bool,
}

pub trait GpuFactory: std::fmt::Debug {
    fn create_renderer(
        &self,
        config: &RendererConfig,
        display_handle: RawDisplayHandle,
    ) -> Result<Box<dyn GpuRenderer>, RendererError>;
}
