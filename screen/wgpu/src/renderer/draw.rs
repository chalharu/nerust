// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::{RenderOutcome, Renderer};
use crate::traits_api::PhysicalSize;
use crate::{RenderSurface, SurfaceSize, SurfaceTargetSource, upload::pack_frame_rows};
use wgpu::{
    Color, CommandEncoderDescriptor, Extent3d, LoadOp, Operations, Origin3d,
    RenderPassColorAttachment, RenderPassDescriptor, StoreOp, TexelCopyBufferInfo,
    TexelCopyBufferLayout, TexelCopyTextureInfo, TextureViewDescriptor,
};

#[derive(Debug, Copy, Clone, PartialEq)]
pub(super) struct Viewport {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) width: f32,
    pub(super) height: f32,
}

pub(super) fn compute_viewport(window_size: SurfaceSize, content_size: PhysicalSize) -> Viewport {
    if window_size.width == 0
        || window_size.height == 0
        || content_size.width <= 0.0
        || content_size.height <= 0.0
    {
        return Viewport {
            x: 0.0,
            y: 0.0,
            width: window_size.width as f32,
            height: window_size.height as f32,
        };
    }

    let rate_x = window_size.width as f32 / content_size.width;
    let rate_y = window_size.height as f32 / content_size.height;
    let rate = rate_x.min(rate_y);
    let width = content_size.width * rate;
    let height = content_size.height * rate;

    Viewport {
        x: (window_size.width as f32 - width) * 0.5,
        y: (window_size.height as f32 - height) * 0.5,
        width,
        height,
    }
}

impl Renderer {
    pub fn reconfigure_surface<T: SurfaceTargetSource>(
        &mut self,
        render_surface: &RenderSurface<T>,
        surface_size: SurfaceSize,
    ) {
        if surface_size.width == 0 || surface_size.height == 0 {
            return;
        }
        let surface = render_surface.surface();
        self.config.width = surface_size.width;
        self.config.height = surface_size.height;
        surface.configure(&self.device, &self.config);
    }

    fn update_frame_texture(&mut self, encoder: &mut wgpu::CommandEncoder, frame_buffer: &[u8]) {
        let upload_bytes = if self.frame_upload_layout.copy_bytes_per_row
            == self.frame_upload_layout.upload_bytes_per_row
        {
            frame_buffer
        } else {
            pack_frame_rows(
                frame_buffer,
                self.source_logical_size.height,
                &mut self.frame_upload_staging,
                self.frame_upload_layout,
            );
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
                    rows_per_image: Some(self.source_logical_size.height as u32),
                },
            },
            TexelCopyTextureInfo {
                texture: &self.frame_texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            Extent3d {
                width: self.source_logical_size.width as u32,
                height: self.source_logical_size.height as u32,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn render<T: SurfaceTargetSource>(
        &mut self,
        render_surface: &RenderSurface<T>,
        surface_size: SurfaceSize,
        frame_buffer: &[u8],
    ) -> Result<RenderOutcome, String> {
        let surface = render_surface.surface();
        let (surface_texture, suboptimal) = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(RenderOutcome::Skipped);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.reconfigure_surface(render_surface, surface_size);
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
        self.update_frame_texture(&mut encoder, frame_buffer);
        let viewport = compute_viewport(surface_size, self.content_size);

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("nerust_render_pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
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

        self.queue.submit(Some(encoder.finish()));
        surface_texture.present();
        if suboptimal {
            self.reconfigure_surface(render_surface, surface_size);
        }
        Ok(RenderOutcome::Presented)
    }
}
