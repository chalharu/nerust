use log::warn;

use nerust_render_traits::{ReservedStrip, SurfaceSize, physical::PhysicalSize};
use wgpu::{
    Color, CommandEncoder, CommandEncoderDescriptor, Extent3d, LoadOp, Operations, Origin3d,
    RenderPassColorAttachment, RenderPassDescriptor, StoreOp, TexelCopyBufferInfo,
    TexelCopyBufferLayout, TexelCopyTextureInfo, TextureView, TextureViewDescriptor,
};

use super::{RenderOutcome, RenderPipeline, fit_surface_size_to_limit};
use crate::upload::pack_frame_rows;

#[derive(Debug, Copy, Clone, PartialEq)]
pub(super) struct Viewport {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

/// True when readback bytes arrive BGR-ordered and need an R/B swap for
/// the R8G8B8A8 image contract.
pub(super) fn bgra_order(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    )
}

pub(super) fn compute_viewport(
    window_size: SurfaceSize,
    content_size: PhysicalSize,
    strip: ReservedStrip,
) -> Viewport {
    let available_height = window_size
        .height
        .saturating_sub(strip.height_loss(window_size.height));
    if window_size.width == 0
        || available_height == 0
        || content_size.width <= 0.0
        || content_size.height <= 0.0
        || !content_size.width.is_finite()
        || !content_size.height.is_finite()
    {
        return Viewport {
            x: 0.0,
            y: strip.top.min(window_size.height) as f32,
            width: 0.0,
            height: 0.0,
        };
    }

    // Finite f32 physical sizes can still overflow f32 ratios. Fit in f64,
    // then narrow only the bounded surface-space result.
    let rate_x = f64::from(window_size.width) / f64::from(content_size.width);
    let rate_y = f64::from(available_height) / f64::from(content_size.height);
    let rate = rate_x.min(rate_y);
    let width = (f64::from(content_size.width) * rate) as f32;
    let height = (f64::from(content_size.height) * rate) as f32;

    // On Android prefer slightly top-aligned viewport so on-screen controls
    // (drawn as overlays) do not overlap the game's important content. Non-Android
    // builds keep centered behavior.
    let y_bias = if cfg!(target_os = "android") {
        0.25_f32
    } else {
        0.5_f32
    };

    Viewport {
        x: (window_size.width as f32 - width) * 0.5,
        y: strip.top as f32 + (available_height as f32 - height) * y_bias,
        width,
        height,
    }
}

impl RenderPipeline {
    /// Point the OSD layer bookkeeping at an explicit size (the in-memory
    /// image path has no swapchain to reconfigure). Mirrors the OSD half of
    /// [`Self::reconfigure_surface`]; the next `set_osd_layer` rebuilds.
    pub fn sync_osd_surface(&mut self, size: SurfaceSize) {
        if self.osd_surface != size {
            // The backend re-applies its authoritative layer after resize.
            self.osd_visible = false;
            self.osd_surface = size;
        }
    }

    pub fn reconfigure_surface(&mut self, surface: &wgpu::Surface<'_>, surface_size: SurfaceSize) {
        if surface_size.width == 0 || surface_size.height == 0 {
            return;
        }
        let surface_size =
            fit_surface_size_to_limit(surface_size, self.device.limits().max_texture_dimension_2d);
        if self.osd_surface != surface_size {
            // The backend re-applies its authoritative layer after resize.
            self.osd_visible = false;
            self.osd_surface = surface_size;
        }
        self.config.width = surface_size.width;
        self.config.height = surface_size.height;
        surface.configure(&self.device, &self.config);
    }

    fn update_frame_texture(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame_buffer: &[u8],
        source_bytes_per_row: usize,
    ) -> Result<(), String> {
        nerust_render_traits::FrameLayout::checked(
            self.frame_logical_size,
            self.frame_upload_layout.bytes_per_pixel as usize,
            source_bytes_per_row,
            frame_buffer.len(),
        )
        .map_err(|e| e.to_string())?;
        let buffer_size = self.frame_upload_layout.buffer_size as usize;
        let upload_bytes = if source_bytes_per_row
            == self.frame_upload_layout.upload_bytes_per_row as usize
            && frame_buffer.len() >= buffer_size
        {
            &frame_buffer[..buffer_size]
        } else {
            pack_frame_rows(
                frame_buffer,
                source_bytes_per_row,
                self.frame_logical_size.height,
                &mut self.frame_upload_staging,
                self.frame_upload_layout,
            )
            .map_err(|e| e.to_string())?;
            &self.frame_upload_staging
        };
        self.queue
            .write_buffer(&self.frame_upload_buffer, 0, upload_bytes);
        encoder.copy_buffer_to_texture(
            TexelCopyBufferInfo {
                buffer: &self.frame_upload_buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.frame_upload_layout.upload_bytes_per_row),
                    rows_per_image: Some(self.frame_logical_size.height as u32),
                },
            },
            TexelCopyTextureInfo {
                texture: &self.frame_texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            Extent3d {
                width: self.frame_logical_size.width as u32,
                height: self.frame_logical_size.height as u32,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Game quad over a cleared backdrop, viewport-clipped. Shared by the
    /// surface present and the in-memory image path.
    fn draw_game(&self, encoder: &mut CommandEncoder, view: &TextureView, viewport: &Viewport) {
        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("nerust_render_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Clear(Color::BLACK),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        if viewport.width > 0.0 && viewport.height > 0.0 {
            render_pass.set_viewport(
                viewport.x,
                viewport.y,
                viewport.width,
                viewport.height,
                0.0,
                1.0,
            );
            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }
    }

    /// Render into memory instead of the swapchain, for frontends that
    /// composite themselves (in-window chrome must survive presents: a
    /// swapchain present would bury it). Same passes as `render`, copied
    /// out through a staging buffer; nothing is presented.
    pub fn render_image(
        &mut self,
        size: SurfaceSize,
        frame_buffer: &[u8],
        source_bytes_per_row: usize,
    ) -> Option<nerust_render_traits::RenderedImage> {
        if size.width == 0 || size.height == 0 {
            return None;
        }
        if nerust_render_traits::FrameLayout::checked(
            self.frame_logical_size,
            self.frame_upload_layout.bytes_per_pixel as usize,
            source_bytes_per_row,
            frame_buffer.len(),
        )
        .is_err()
        {
            warn!("wgpu: invalid frame for image render");
            return None;
        }
        let size = fit_surface_size_to_limit(size, self.device.limits().max_texture_dimension_2d);
        if size.width == 0 || size.height == 0 {
            return None;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("nerust_render_image_encoder"),
            });
        if self
            .update_frame_texture(&mut encoder, frame_buffer, source_bytes_per_row)
            .is_err()
        {
            return None;
        }
        let viewport = compute_viewport(size, self.content_size, self.reserved);
        self.ensure_image_target(size);
        let view = self
            .image_texture
            .as_ref()
            .map(|(_, texture)| texture.create_view(&TextureViewDescriptor::default()))?;
        self.draw_game(&mut encoder, &view, &viewport);
        self.draw_osd(&mut encoder, &view);
        let pitch = self.ensure_image_staging(size)?;
        let staging = &self.image_staging.as_ref()?.2;
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &self.image_texture.as_ref()?.1,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: staging,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(size.height),
                },
            },
            Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        // Drop cached GPU objects on failure so the next frame rebuilds
        // them instead of retrying a dead mapping.
        if self
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .is_err()
        {
            self.image_texture = None;
            self.image_staging = None;
            return None;
        }
        if receiver.recv().ok().is_none_or(|result| result.is_err()) {
            staging.unmap();
            self.image_texture = None;
            self.image_staging = None;
            return None;
        }
        let bgra = bgra_order(self.config.format);
        let row_bytes = size.width.saturating_mul(4) as usize;
        let mut pixels = Vec::with_capacity(row_bytes.saturating_mul(size.height as usize));
        {
            let mapped = staging.slice(..).get_mapped_range().map_err(|error| {
                warn!("wgpu: image readback map failed: {error}");
                staging.unmap();
            });
            let Ok(mapped) = mapped else {
                return None;
            };
            for y in 0..size.height as usize {
                let row = &mapped[y * pitch as usize..y * pitch as usize + row_bytes];
                if bgra {
                    for px in row.as_chunks::<4>().0 {
                        pixels.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                    }
                } else {
                    pixels.extend_from_slice(row);
                }
            }
        }
        staging.unmap();
        Some(nerust_render_traits::RenderedImage {
            width: size.width,
            height: size.height,
            stride_bytes: row_bytes,
            pixels,
        })
    }

    /// Offscreen target sized to the requested image, recreated on change.
    fn ensure_image_target(&mut self, size: SurfaceSize) {
        let current = self.image_texture.as_ref().map(|(size, _)| *size);
        if current == Some(size) {
            return;
        }
        self.image_texture = Some((
            size,
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("nerust_image_texture"),
                size: Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }),
        ));
        self.image_staging = None;
    }

    /// Staging buffer pitch for one image readback: 256-byte aligned rows.
    /// Returns `None` when there is nothing to stage.
    fn ensure_image_staging(&mut self, size: SurfaceSize) -> Option<u32> {
        let pitch = size.width.saturating_mul(4).next_multiple_of(256);
        let current = self
            .image_staging
            .as_ref()
            .map(|(size, pitch, _)| (*size, *pitch));
        if current != Some((size, pitch)) {
            if size.height == 0 {
                return None;
            }
            self.image_staging = Some((
                size,
                pitch,
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("nerust_image_staging"),
                    size: u64::from(pitch).saturating_mul(u64::from(size.height)),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
            ));
        }
        Some(pitch)
    }

    pub fn render(
        &mut self,
        surface: &wgpu::Surface<'_>,
        surface_size: SurfaceSize,
        frame_buffer: &[u8],
        source_bytes_per_row: usize,
    ) -> Result<RenderOutcome, String> {
        if surface_size.width == 0 || surface_size.height == 0 {
            return Ok(RenderOutcome::Skipped);
        }
        nerust_render_traits::FrameLayout::checked(
            self.frame_logical_size,
            self.frame_upload_layout.bytes_per_pixel as usize,
            source_bytes_per_row,
            frame_buffer.len(),
        )
        .map_err(|e| e.to_string())?;
        let surface_size =
            fit_surface_size_to_limit(surface_size, self.device.limits().max_texture_dimension_2d);
        let (surface_texture, suboptimal) = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(RenderOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.reconfigure_surface(surface, surface_size);
                return Ok(RenderOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                return Ok(RenderOutcome::RecreateSurface);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("wgpu surface validation error".to_string());
            }
        };

        let view = surface_texture
            .texture
            .create_view(&TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("nerust_render_encoder"),
            });
        self.update_frame_texture(&mut encoder, frame_buffer, source_bytes_per_row)?;
        let viewport = compute_viewport(surface_size, self.content_size, self.reserved);
        self.draw_game(&mut encoder, &view, &viewport);
        self.draw_osd(&mut encoder, &view);

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(surface_texture);
        if suboptimal {
            self.reconfigure_surface(surface, surface_size);
        }
        Ok(RenderOutcome::Presented)
    }
}
