mod draw;
pub(crate) mod setup;

use nerust_render_traits::{SurfaceSize, logical::LogicalSize, physical::PhysicalSize};
use wgpu::{
    BindGroup, Buffer, CommandEncoder, Device, Limits, LoadOp, Operations, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, StoreOp, SurfaceConfiguration, Texture,
    TextureView,
};

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
    osd_pipeline: wgpu::RenderPipeline,
    osd_layout: wgpu::BindGroupLayout,
    osd_sampler: wgpu::Sampler,
    osd_texture: Option<wgpu::Texture>,
    osd_bind_group: Option<BindGroup>,
    osd_rect: Option<(u32, u32, u32, u32)>,
    osd_staging: Vec<u8>,
    frame_logical_size: LogicalSize,
    content_size: PhysicalSize,
    reserved: nerust_render_traits::ReservedStrip,
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

    /// Edge strips the game viewport avoids; applied on next render.
    pub fn set_reserved_strip(&mut self, strip: nerust_render_traits::ReservedStrip) {
        self.reserved = strip;
    }

    /// Composite the OSD layer after the game pass, if one is set.
    /// Same surface, viewport clipped to the layer rect.
    pub fn draw_osd(&self, encoder: &mut CommandEncoder, view: &TextureView) {
        let (Some(bind_group), Some((x, y, w, h))) = (self.osd_bind_group.as_ref(), self.osd_rect)
        else {
            return;
        };
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("nerust_osd_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        render_pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        render_pass.set_pipeline(&self.osd_pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
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
