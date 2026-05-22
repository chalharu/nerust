// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::{
    srgb_lut::SRGB_TO_LINEAR_LUT_BYTES,
    upload::{FrameUploadLayout, pack_frame_rows},
};
use nerust_screen_filter::{
    NTSC_TEXTURE_HEIGHT, NTSC_TEXTURE_WIDTH, NesVideoAssets, PALETTE_TEXTURE_WIDTH,
};
use nerust_screen_traits::{LogicalSize, PhysicalSize, VideoPresentation};
use nerust_wgpuwrap::{RenderSurface, SurfaceSize, SurfaceTargetSource};
use wgpu::{
    BindGroup, BindGroupLayout, Buffer, BufferDescriptor, BufferUsages, Color, ColorTargetState,
    ColorWrites, CommandEncoderDescriptor, CompositeAlphaMode, Device, Extent3d, FragmentState,
    LoadOp, MultisampleState, Operations, Origin3d, PipelineCompilationOptions,
    PipelineLayoutDescriptor, PresentMode, PrimitiveState, Queue, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, StoreOp, Surface, SurfaceConfiguration, TexelCopyBufferInfo,
    TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureDescriptor, TextureDimension,
    TextureFormat, TextureSampleType, TextureUsages, TextureViewDescriptor, TextureViewDimension,
};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RenderOutcome {
    Presented,
    Skipped,
    RecreateSurface,
}

#[derive(Debug, Copy, Clone, PartialEq)]
struct Viewport {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn compute_viewport(window_size: SurfaceSize, content_size: PhysicalSize) -> Viewport {
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
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    frame_texture: Texture,
    _palette_texture: Texture,
    _ntsc_texture: Texture,
    _srgb_lut_texture: Texture,
    frame_upload_buffer: Buffer,
    frame_upload_layout: FrameUploadLayout,
    frame_upload_staging: Box<[u8]>,
    _uniforms_buffer: Buffer,
    _bind_group_layout: BindGroupLayout,
    bind_group: BindGroup,
    pipeline: RenderPipeline,
    source_logical_size: LogicalSize,
    content_size: PhysicalSize,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct FilterUniforms {
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
}

impl Renderer {
    pub async fn new<T: SurfaceTargetSource>(
        render_surface: &RenderSurface<T>,
        surface_size: SurfaceSize,
        presentation: &VideoPresentation,
        assets: &NesVideoAssets,
    ) -> Result<Self, String> {
        if !presentation.is_palette_frame() {
            return Err(
                "nerust_screen_wgpu does not yet support non-palette video presentations"
                    .to_string(),
            );
        }

        let instance = render_surface.instance();
        let surface = render_surface.surface();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|err| format!("failed to request wgpu adapter: {err:?}"))?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("nerust_wgpu_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .map_err(|err| format!("failed to request wgpu device: {err:?}"))?;

        let mut config = Self::surface_config(surface, &adapter, surface_size)?;
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
        let source_logical_size = presentation.source_logical_size();
        let logical_size = presentation.logical_size();
        let content_size = presentation.physical_size();
        let frame_upload_layout = FrameUploadLayout::for_logical_size(source_logical_size, 1)?;

        let frame_texture = device.create_texture(&TextureDescriptor {
            label: Some("nerust_frame_texture"),
            size: Extent3d {
                width: source_logical_size.width as u32,
                height: source_logical_size.height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::R8Uint,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let palette_texture = create_texture_from_bytes(
            &device,
            &queue,
            "nerust_palette_texture",
            TextureFormat::Rgba8Uint,
            Extent3d {
                width: PALETTE_TEXTURE_WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
            assets.palette_rgba8(),
        );
        let (ntsc_data, ntsc_size) = encode_ntsc_texture(assets.packed_ntsc_rgba8());
        let ntsc_texture = create_texture_from_bytes(
            &device,
            &queue,
            "nerust_ntsc_texture",
            TextureFormat::R32Uint,
            ntsc_size,
            &ntsc_data,
        );
        let srgb_lut_texture = create_texture_1d_from_bytes(
            &device,
            &queue,
            "nerust_srgb_lut_texture",
            TextureFormat::R32Float,
            &SRGB_TO_LINEAR_LUT_BYTES,
        );
        let frame_upload_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("nerust_frame_upload_buffer"),
            size: frame_upload_layout.buffer_size,
            usage: BufferUsages::COPY_DST | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let frame_upload_staging =
            vec![0; frame_upload_layout.buffer_size as usize].into_boxed_slice();
        let frame_view = frame_texture.create_view(&TextureViewDescriptor::default());
        let palette_view = palette_texture.create_view(&TextureViewDescriptor::default());
        let ntsc_view = ntsc_texture.create_view(&TextureViewDescriptor::default());
        let srgb_lut_view = srgb_lut_texture.create_view(&TextureViewDescriptor::default());
        let uniforms = FilterUniforms {
            source_width: source_logical_size.width as u32,
            source_height: source_logical_size.height as u32,
            output_width: logical_size.width as u32,
            output_height: logical_size.height as u32,
        };
        let uniforms_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("nerust_filter_uniforms"),
            size: std::mem::size_of::<FilterUniforms>() as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniforms_buffer, 0, cast_bytes(&uniforms));

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nerust_frame_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Uint,
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Uint,
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Uint,
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Float { filterable: false },
                        view_dimension: TextureViewDimension::D1,
                    },
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
                    resource: wgpu::BindingResource::TextureView(&palette_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&ntsc_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniforms_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&srgb_lut_view),
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
        let pipeline = create_render_pipeline(
            &device,
            &pipeline_layout,
            &shader,
            config.format,
            fragment_entry_point(assets.uses_ntsc_pipeline(), config.format.is_srgb()),
        );

        Ok(Self {
            device,
            queue,
            config,
            frame_texture,
            _palette_texture: palette_texture,
            _ntsc_texture: ntsc_texture,
            _srgb_lut_texture: srgb_lut_texture,
            frame_upload_buffer,
            frame_upload_layout,
            frame_upload_staging,
            _uniforms_buffer: uniforms_buffer,
            _bind_group_layout: bind_group_layout,
            bind_group,
            pipeline,
            source_logical_size,
            content_size,
        })
    }

    fn surface_config(
        surface: &Surface<'_>,
        adapter: &wgpu::Adapter,
        surface_size: SurfaceSize,
    ) -> Result<SurfaceConfiguration, String> {
        surface
            .get_default_config(
                adapter,
                surface_size.width.max(1),
                surface_size.height.max(1),
            )
            .ok_or_else(|| "failed to derive a default surface configuration".to_string())
    }

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

fn cast_bytes<T>(value: &T) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts((value as *const T).cast::<u8>(), std::mem::size_of::<T>())
    }
}

fn create_texture_from_bytes(
    device: &Device,
    queue: &Queue,
    label: &str,
    format: TextureFormat,
    size: Extent3d,
    bytes: &[u8],
) -> Texture {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.width * 4),
            rows_per_image: Some(size.height),
        },
        size,
    );
    texture
}

fn create_texture_1d_from_bytes(
    device: &Device,
    queue: &Queue,
    label: &str,
    format: TextureFormat,
    bytes: &[u8],
) -> Texture {
    let width =
        u32::try_from(bytes.len() / std::mem::size_of::<f32>()).expect("LUT width must fit u32");
    let texture = device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D1,
        format,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: None,
            rows_per_image: None,
        },
        Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn create_render_pipeline(
    device: &Device,
    pipeline_layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: TextureFormat,
    fragment_entry_point: &'static str,
) -> RenderPipeline {
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("nerust_render_pipeline"),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: PipelineCompilationOptions::default(),
        },
        fragment: Some(FragmentState {
            module: shader,
            entry_point: Some(fragment_entry_point),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: ColorWrites::ALL,
            })],
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn fragment_entry_point(uses_ntsc_pipeline: bool, surface_is_srgb: bool) -> &'static str {
    match (uses_ntsc_pipeline, surface_is_srgb) {
        (false, true) => "fs_palette_srgb",
        (false, false) => "fs_palette_linear",
        (true, true) => "fs_ntsc_srgb",
        (true, false) => "fs_ntsc_linear",
    }
}

fn encode_ntsc_texture(packed_ntsc_rgba8: Option<&[u8]>) -> (Box<[u8]>, Extent3d) {
    let Some(texture) = packed_ntsc_rgba8 else {
        return (
            vec![0; 4].into_boxed_slice(),
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    };

    let mut packed = Vec::with_capacity(texture.len());
    let mut chunks = texture.chunks_exact(4);
    assert!(
        chunks.remainder().is_empty(),
        "packed NTSC texture must be a multiple of 4 bytes"
    );
    for chunk in &mut chunks {
        packed.extend_from_slice(
            &u32::from_be_bytes(
                chunk
                    .try_into()
                    .expect("NTSC texture chunk must be 4 bytes"),
            )
            .to_le_bytes(),
        );
    }

    (
        packed.into_boxed_slice(),
        Extent3d {
            width: NTSC_TEXTURE_WIDTH,
            height: NTSC_TEXTURE_HEIGHT,
            depth_or_array_layers: 1,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{compute_viewport, encode_ntsc_texture};
    use nerust_screen_filter::{FilterType, NTSC_TEXTURE_HEIGHT, NTSC_TEXTURE_WIDTH};
    use nerust_screen_traits::PhysicalSize;
    use nerust_wgpuwrap::SurfaceSize;

    #[test]
    fn viewport_preserves_aspect_ratio() {
        let viewport = compute_viewport(
            SurfaceSize::new(1600, 900),
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

    #[test]
    fn ntsc_texture_is_prepacked_for_r32uint_upload() {
        let assets = FilterType::NtscRGB.palette_nes_video_assets();
        let source = assets
            .packed_ntsc_rgba8()
            .expect("NTSC filter should provide a packed texture");
        let (packed, size) = encode_ntsc_texture(Some(source));

        assert_eq!(size.width, NTSC_TEXTURE_WIDTH);
        assert_eq!(size.height, NTSC_TEXTURE_HEIGHT);
        assert_eq!(packed.len(), source.len());
        assert_eq!(
            &packed[..4],
            &u32::from_be_bytes(source[..4].try_into().expect("first texel must exist"))
                .to_le_bytes()
        );
        assert_eq!(
            &packed[packed.len() - 4..],
            &u32::from_be_bytes(
                source[source.len() - 4..]
                    .try_into()
                    .expect("last texel must exist")
            )
            .to_le_bytes()
        );
    }
}
