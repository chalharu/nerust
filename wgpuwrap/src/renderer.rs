// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::surface::SurfaceTarget;
use crate::upload::{FrameUploadLayout, pack_frame_rows};
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use std::sync::Arc;
use tao::{dpi::PhysicalSize as TaoPhysicalSize, window::Window as TaoWindow};
use wgpu::{
    BindGroup, BindGroupLayout, Buffer, BufferDescriptor, BufferUsages, Color, ColorTargetState,
    ColorWrites, CommandEncoderDescriptor, CompositeAlphaMode, Device, Extent3d, Features,
    FilterMode, FragmentState, Instance, Limits, LoadOp, MultisampleState, Operations, Origin3d,
    PipelineCompilationOptions, PipelineLayoutDescriptor, PowerPreference, PresentMode,
    PrimitiveState, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, RequestAdapterOptions, Sampler, SamplerBindingType,
    SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, Surface,
    SurfaceConfiguration, TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo,
    Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
    TextureViewDescriptor, TextureViewDimension,
};

#[derive(Debug, Copy, Clone, PartialEq)]
struct Viewport {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn compute_viewport(window_size: TaoPhysicalSize<u32>, content_size: PhysicalSize) -> Viewport {
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

pub struct Renderer {
    instance: Instance,
    window: Arc<TaoWindow>,
    // The surface must drop before the GTK render target that backs its raw handles.
    surface: Surface<'static>,
    surface_target: SurfaceTarget,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    frame_texture: Texture,
    frame_upload_buffer: Buffer,
    frame_upload_layout: FrameUploadLayout,
    frame_upload_staging: Box<[u8]>,
    _frame_sampler: Sampler,
    _bind_group_layout: BindGroupLayout,
    bind_group: BindGroup,
    pipeline: RenderPipeline,
    logical_size: LogicalSize,
    content_size: PhysicalSize,
}

impl Renderer {
    pub fn new(
        window: Arc<TaoWindow>,
        surface_target: SurfaceTarget,
        logical_size: LogicalSize,
        content_size: PhysicalSize,
    ) -> Result<Self, String> {
        pollster::block_on(Self::new_async(
            window,
            surface_target,
            logical_size,
            content_size,
        ))
    }

    async fn new_async(
        window: Arc<TaoWindow>,
        surface_target: SurfaceTarget,
        logical_size: LogicalSize,
        content_size: PhysicalSize,
    ) -> Result<Self, String> {
        let instance = Instance::default();
        surface_target.prepare();
        let surface = surface_target.create_surface(&instance)?;
        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                power_preference: PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|err| format!("failed to request wgpu adapter: {err:?}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("nerust_wgpu_device"),
                required_features: Features::empty(),
                required_limits: Limits::default(),
                ..Default::default()
            })
            .await
            .map_err(|err| format!("failed to request wgpu device: {err:?}"))?;

        let mut config = Self::surface_config(
            &surface,
            &adapter,
            surface_target.surface_size(window.inner_size()),
        )?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|format| format.is_srgb()) {
            config.format = format;
        }
        if caps.present_modes.contains(&PresentMode::AutoVsync) {
            config.present_mode = PresentMode::AutoVsync;
        }
        if caps.alpha_modes.contains(&CompositeAlphaMode::Opaque) {
            config.alpha_mode = CompositeAlphaMode::Opaque;
        }
        surface.configure(&device, &config);
        let frame_upload_layout = FrameUploadLayout::for_logical_size(logical_size)?;

        let frame_texture = device.create_texture(&TextureDescriptor {
            label: Some("nerust_frame_texture"),
            size: Extent3d {
                width: logical_size.width as u32,
                height: logical_size.height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let frame_upload_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("nerust_frame_upload_buffer"),
            size: frame_upload_layout.buffer_size,
            usage: BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let frame_upload_staging =
            vec![0; frame_upload_layout.buffer_size as usize].into_boxed_slice();
        let frame_view = frame_texture.create_view(&TextureViewDescriptor::default());
        let frame_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("nerust_frame_sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nerust_frame_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nerust_frame_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&frame_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&frame_sampler),
                },
            ],
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("nerust_wgpu_shader"),
            source: ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("nerust_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("nerust_render_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self {
            instance,
            window,
            surface_target,
            surface,
            device,
            queue,
            config,
            frame_texture,
            frame_upload_buffer,
            frame_upload_layout,
            frame_upload_staging,
            _frame_sampler: frame_sampler,
            _bind_group_layout: bind_group_layout,
            bind_group,
            pipeline,
            logical_size,
            content_size,
        })
    }

    fn surface_config(
        surface: &Surface<'_>,
        adapter: &wgpu::Adapter,
        window_size: TaoPhysicalSize<u32>,
    ) -> Result<SurfaceConfiguration, String> {
        surface
            .get_default_config(adapter, window_size.width.max(1), window_size.height.max(1))
            .ok_or_else(|| "failed to derive a default surface configuration".to_string())
    }

    pub fn resize_to_target(&mut self) {
        let size = self.surface_target.surface_size(self.window.inner_size());
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    fn recreate_surface(&mut self) -> Result<(), String> {
        self.surface = self.surface_target.create_surface(&self.instance)?;
        self.resize_to_target();
        Ok(())
    }

    fn update_frame_texture(&mut self, encoder: &mut wgpu::CommandEncoder, frame_buffer: &[u8]) {
        let upload_bytes = if self.frame_upload_layout.copy_bytes_per_row
            == self.frame_upload_layout.upload_bytes_per_row
        {
            frame_buffer
        } else {
            pack_frame_rows(
                frame_buffer,
                self.logical_size.height,
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
                    rows_per_image: Some(self.logical_size.height as u32),
                },
            },
            TexelCopyTextureInfo {
                texture: &self.frame_texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            Extent3d {
                width: self.logical_size.width as u32,
                height: self.logical_size.height as u32,
                depth_or_array_layers: 1,
            },
        );
    }

    pub fn render(&mut self, frame_buffer: &[u8]) -> Result<bool, String> {
        let (surface_texture, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.resize_to_target();
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                return Ok(false);
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
        let viewport = compute_viewport(
            self.surface_target.surface_size(self.window.inner_size()),
            self.content_size,
        );

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
            self.resize_to_target();
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::compute_viewport;
    use nerust_screen_traits::PhysicalSize;
    use tao::dpi::PhysicalSize as TaoPhysicalSize;

    #[test]
    fn viewport_preserves_aspect_ratio() {
        let viewport = compute_viewport(
            TaoPhysicalSize::new(1600, 900),
            PhysicalSize {
                width: 512.0,
                height: 480.0,
            },
        );

        assert_eq!(viewport.width, 960.0);
        assert_eq!(viewport.height, 900.0);
        assert_eq!(viewport.x, 320.0);
        assert_eq!(viewport.y, 0.0);
    }
}
