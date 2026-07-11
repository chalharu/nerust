mod draw;
mod setup;

use nerust_render_base::{SurfaceSize, logical::LogicalSize, physical::PhysicalSize};
use wgpu::{BindGroup, Buffer, Device, Limits, Queue, SurfaceConfiguration, Texture};

use crate::upload::FrameUploadLayout;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RenderOutcome {
    Presented,
    Skipped,
    RecreateSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationOptions {
    pub vsync: bool,
}

impl Default for PresentationOptions {
    fn default() -> Self {
        Self { vsync: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceLimitProfile {
    Default,
    DownlevelWebGl2,
}

impl DeviceLimitProfile {
    pub fn required_limits(self) -> Limits {
        match self {
            Self::Default => Limits::default(),
            Self::DownlevelWebGl2 => Limits::downlevel_webgl2_defaults(),
        }
    }
}

pub(crate) fn fit_surface_size_to_limit(
    surface_size: SurfaceSize,
    max_texture_dimension_2d: u32,
) -> SurfaceSize {
    let width = surface_size.width.max(1);
    let height = surface_size.height.max(1);
    let max_texture_dimension_2d = max_texture_dimension_2d.max(1);

    if width <= max_texture_dimension_2d && height <= max_texture_dimension_2d {
        return SurfaceSize::new(width, height);
    }

    let largest_dimension = width.max(height);
    let scaled_width = (u64::from(width) * u64::from(max_texture_dimension_2d)
        / u64::from(largest_dimension))
    .max(1) as u32;
    let scaled_height = (u64::from(height) * u64::from(max_texture_dimension_2d)
        / u64::from(largest_dimension))
    .max(1) as u32;
    SurfaceSize::new(scaled_width, scaled_height)
}

pub struct RenderPipeline {
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    frame_texture: Texture,
    palette_texture: Texture,
    palette_width: u32,
    palette_height: u32,
    frame_upload_buffer: Buffer,
    frame_upload_layout: FrameUploadLayout,
    frame_upload_staging: Box<[u8]>,
    bind_group: BindGroup,
    pipeline: wgpu::RenderPipeline,
    frame_logical_size: LogicalSize,
    content_size: PhysicalSize,
}

impl RenderPipeline {
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn surface_config(&self) -> &wgpu::SurfaceConfiguration {
        &self.config
    }

    /// PaletteIndex 形式の FrameBuffer からパレットデータを palette texture に書き込む。
    /// `render()` の前に呼ばれることを想定。
    /// palette の width/height は texture 作成時の値から自動的に決まる。
    pub fn update_palette_texture(&self, rgba8: &[u8; 256]) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.palette_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba8[..],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.palette_width * 4),
                rows_per_image: Some(self.palette_height),
            },
            wgpu::Extent3d {
                width: self.palette_width,
                height: self.palette_height,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests;
