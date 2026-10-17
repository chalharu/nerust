use nerust_render_ntsc::NTSC_TEXTURE_WIDTH;
use nerust_render_traits::{
    SurfaceSize, VideoFrameFormat, VideoPresentation, filter::PALETTE_TEXTURE_WIDTH,
    logical::LogicalSize,
};
use wgpu::{
    BindGroupLayoutEntry, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CompositeAlphaMode, Device, Extent3d, FragmentState, MultisampleState, Origin3d,
    PipelineCompilationOptions, PipelineLayoutDescriptor, PresentMode, PrimitiveState, Queue,
    RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureDescriptor, TextureDimension,
    TextureFormat, TextureSampleType, TextureUsages, TextureViewDescriptor, TextureViewDimension,
};
use zerocopy::{Immutable, IntoBytes};

use super::{DeviceLimitProfile, PresentationOptions, RenderPipeline, fit_surface_size_to_limit};
use crate::{srgb_lut::SRGB_TO_LINEAR_LUT_BYTES, upload::FrameUploadLayout};

#[repr(C)]
#[derive(Debug, Copy, Clone, IntoBytes, Immutable)]
struct FilterUniforms {
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(super) enum FramePipelineKind {
    DirectColor,
    Palette,
    Ntsc,
}

impl RenderPipeline {
    pub async fn new(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        surface_size: SurfaceSize,
        presentation: &VideoPresentation,
        ntsc_data: Option<&[u8]>,
        presentation_options: PresentationOptions,
        device_limit_profile: DeviceLimitProfile,
    ) -> Result<Self, String> {
        let pipeline_kind = frame_pipeline_kind(presentation, ntsc_data)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|err| format!("failed to request wgpu adapter: {err:?}"))?;
        let adapter_limits = adapter.limits();
        let mut required_limits = device_limit_profile.required_limits();
        let requested_surface_dimension = surface_size.width.max(surface_size.height).max(1);
        required_limits.max_texture_dimension_2d = required_limits
            .max_texture_dimension_2d
            .max(requested_surface_dimension)
            .min(adapter_limits.max_texture_dimension_2d);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("nerust_wgpu_device"),
                required_features: wgpu::Features::empty(),
                required_limits,
                ..Default::default()
            })
            .await
            .map_err(|err| format!("failed to request wgpu device: {err:?}"))?;

        let normalized_size =
            fit_surface_size_to_limit(surface_size, device.limits().max_texture_dimension_2d);
        let mut config = surface
            .get_default_config(
                &adapter,
                normalized_size.width.max(1),
                normalized_size.height.max(1),
            )
            .ok_or_else(|| "failed to derive a default surface configuration".to_string())?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|format| format.is_srgb()) {
            config.format = format;
        }
        config.present_mode =
            Self::select_present_mode(&caps.present_modes, presentation_options.vsync);
        if caps.alpha_modes.contains(&CompositeAlphaMode::Opaque) {
            config.alpha_mode = CompositeAlphaMode::Opaque;
        }
        surface.configure(&device, &config);
        let logical_size = presentation.logical_size();
        let frame_logical_size = frame_logical_size(presentation, pipeline_kind);
        let content_size = presentation.physical_size();
        let frame_upload_layout = FrameUploadLayout::for_logical_size(
            frame_logical_size,
            frame_bytes_per_pixel(pipeline_kind),
        )?;

        let frame_texture = device.create_texture(&TextureDescriptor {
            label: Some("nerust_frame_texture"),
            size: Extent3d {
                width: frame_logical_size.width as u32,
                height: frame_logical_size.height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: frame_texture_format(pipeline_kind),
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Palette texture: 常に 64x1 RGBA8、ゼロ初期化。
        // 実データは render 時に FrameBuffer.palette_as_rgba8() から同期される。
        let palette_size = Extent3d {
            width: PALETTE_TEXTURE_WIDTH,
            height: 1,
            depth_or_array_layers: 1,
        };
        let palette_data = vec![0u8; PALETTE_TEXTURE_WIDTH as usize * 4];
        let palette_texture = create_texture_from_bytes(
            &device,
            &queue,
            "nerust_palette_texture",
            TextureFormat::Rgba8Uint,
            palette_size,
            &palette_data,
        );
        let (ntsc_data, ntsc_size) = encode_ntsc_texture(ntsc_data);
        let ntsc_texture = create_texture_from_bytes(
            &device,
            &queue,
            "nerust_ntsc_texture",
            TextureFormat::R32Uint,
            ntsc_size,
            &ntsc_data,
        );
        let srgb_lut_texture = create_srgb_lut_texture(
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
        // ntsc_texture / srgb_lut_texture / uniforms_buffer / bind_group_layout は
        // ここで drop。GPU リソースは BindGroup / View 経由で保持されるため安全。
        drop(ntsc_texture);
        drop(srgb_lut_texture);
        let uniforms = FilterUniforms {
            source_width: frame_logical_size.width as u32,
            source_height: frame_logical_size.height as u32,
            output_width: logical_size.width as u32,
            output_height: logical_size.height as u32,
        };
        let uniforms_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("nerust_filter_uniforms"),
            size: std::mem::size_of::<FilterUniforms>() as u64,
            usage: BufferUsages::COPY_DST | BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniforms_buffer, 0, uniforms.as_bytes());

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nerust_frame_bind_group_layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Uint,
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Uint,
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Uint,
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 4,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Float { filterable: false },
                        view_dimension: TextureViewDimension::D2,
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

        let osd_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nerust_osd_bind_group_layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let osd_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nerust_osd_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let osd_layout_handle = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("nerust_osd_pipeline_layout"),
            bind_group_layouts: &[Some(&osd_layout)],
            immediate_size: 0,
        });

        let shader_source = composed_shader_source();
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("nerust_wgpu_shader"),
            source: ShaderSource::Wgsl(shader_source.into()),
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
            fragment_entry_point(pipeline_kind, config.format.is_srgb()),
        );
        let osd_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("nerust_osd_pipeline"),
            layout: Some(&osd_layout_handle),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_osd"),
                buffers: &[],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_osd"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // uniforms_buffer / bind_group_layout は BindGroup 構築後に不要。
        // GPU リソースは BindGroup が内部参照を保持するため drop して安全。
        drop(uniforms_buffer);
        drop(bind_group_layout);

        Ok(Self {
            device,
            queue,
            config,
            frame_texture,
            palette_texture,
            palette_width: PALETTE_TEXTURE_WIDTH,
            palette_height: 1,
            frame_upload_buffer,
            frame_upload_layout,
            frame_upload_staging,
            bind_group,
            pipeline,
            osd_pipeline,
            osd_layout,
            osd_sampler,
            osd_texture: None,
            osd_bind_group: None,
            osd_visible: false,
            osd_surface: surface_size,
            osd_prev: None,
            osd_staging: Vec::new(),
            frame_logical_size,
            content_size,
            reserved: nerust_render_traits::ReservedStrip::default(),
        })
    }

    fn select_present_mode(modes: &[PresentMode], vsync: bool) -> PresentMode {
        let preferred = if vsync {
            [
                PresentMode::AutoVsync,
                PresentMode::Fifo,
                PresentMode::Mailbox,
            ]
        } else {
            [
                PresentMode::AutoNoVsync,
                PresentMode::Immediate,
                PresentMode::Mailbox,
            ]
        };
        preferred
            .into_iter()
            .find(|mode| modes.contains(mode))
            .unwrap_or(PresentMode::Fifo)
    }
}

pub(super) fn frame_logical_size(
    presentation: &VideoPresentation,
    pipeline_kind: FramePipelineKind,
) -> LogicalSize {
    match pipeline_kind {
        FramePipelineKind::DirectColor => presentation.logical_size(),
        FramePipelineKind::Palette | FramePipelineKind::Ntsc => presentation.source_logical_size(),
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
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.width * 4),
            rows_per_image: Some(size.height),
        },
        size,
    );
    texture
}

fn create_srgb_lut_texture(
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
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(1),
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
) -> wgpu::RenderPipeline {
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

pub(super) fn composed_shader_source() -> String {
    [
        include_str!("../shader/vertex.wgsl"),
        include_str!("../shader/common.wgsl"),
        include_str!("../shader/palette_decode.wgsl"),
        include_str!("../shader/ntsc_decode.wgsl"),
        include_str!("../shader/presentation.wgsl"),
        include_str!("../shader/overlay.wgsl"),
    ]
    .join("\n\n")
}

/// Upload an OSD layer into a full-surface texture at its offset.
/// Viewport quirks vary across drivers, so placement rides on texture
/// coordinates (always identity-mapped) instead of small viewports.
pub(crate) fn set_osd_layer(
    pipeline: &mut RenderPipeline,
    layer: Option<nerust_render_traits::renderer::OsdLayer>,
) {
    let surface = pipeline.osd_surface;
    if surface.width == 0 || surface.height == 0 {
        return;
    }
    let sized = pipeline.osd_texture.as_ref().is_some_and(|texture| {
        texture.width() == surface.width && texture.height() == surface.height
    });
    if !sized {
        let texture = pipeline.device.create_texture(&TextureDescriptor {
            label: Some("nerust_osd_texture"),
            size: Extent3d {
                width: surface.width,
                height: surface.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        pipeline.osd_bind_group = Some(pipeline.device.create_bind_group(
            &wgpu::BindGroupDescriptor {
                label: Some("nerust_osd_bind_group"),
                layout: &pipeline.osd_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipeline.osd_sampler),
                    },
                ],
            },
        ));
        pipeline.osd_texture = Some(texture);
        pipeline.osd_prev = None;
    }
    let Some(texture) = pipeline.osd_texture.clone() else {
        return;
    };
    // Clear the previous rect so moved/shrunk layers leave no ghosts.
    let clear_rect = match (&layer, pipeline.osd_prev) {
        (Some(next), Some(prev)) if (next.x, next.y, next.width, next.height) != prev => Some(prev),
        (None, Some(prev)) => Some(prev),
        _ => None,
    };
    if let Some((x, y, w, h)) = clear_rect {
        upload_rect(
            pipeline,
            &texture,
            x,
            y,
            w.min(surface.width.saturating_sub(x)),
            h.min(surface.height.saturating_sub(y)),
            None,
        );
    }
    let Some(layer) = layer else {
        pipeline.osd_visible = false;
        pipeline.osd_prev = None;
        return;
    };
    if layer.width == 0 || layer.height == 0 {
        return;
    }
    let w = layer.width.min(surface.width.saturating_sub(layer.x));
    let h = layer.height.min(surface.height.saturating_sub(layer.y));
    if w == 0 || h == 0 {
        return;
    }
    upload_rect(
        pipeline,
        &texture,
        layer.x,
        layer.y,
        w,
        h,
        Some(&layer.rgba),
    );
    pipeline.osd_visible = true;
    pipeline.osd_prev = Some((layer.x, layer.y, w, h));
}

/// Upload RGBA rows (or zeros) into a texture subrect with 256-byte
/// pitch padding.
fn upload_rect(
    pipeline: &mut RenderPipeline,
    texture: &wgpu::Texture,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    rgba: Option<&[u8]>,
) {
    let row_bytes = w as usize * 4;
    let pitch = row_bytes.next_multiple_of(256);
    let needed = pitch * h as usize;
    if pipeline.osd_staging.len() < needed {
        pipeline.osd_staging.resize(needed, 0);
    }
    if let Some(rgba) = rgba {
        for (index, row) in rgba.chunks_exact(row_bytes).take(h as usize).enumerate() {
            pipeline.osd_staging[index * pitch..index * pitch + row_bytes].copy_from_slice(row);
        }
    } else {
        pipeline.osd_staging[..needed].fill(0);
    }
    pipeline.queue.write_texture(
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        &pipeline.osd_staging[..needed],
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(pitch as u32),
            rows_per_image: Some(h),
        },
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

fn frame_pipeline_kind(
    presentation: &VideoPresentation,
    ntsc_data: Option<&[u8]>,
) -> Result<FramePipelineKind, String> {
    match presentation.frame_format() {
        VideoFrameFormat::Rgba => Ok(FramePipelineKind::DirectColor),
        VideoFrameFormat::Palette => Ok(match ntsc_data {
            Some(_) => FramePipelineKind::Ntsc,
            None => FramePipelineKind::Palette,
        }),
    }
}

fn frame_bytes_per_pixel(kind: FramePipelineKind) -> u32 {
    match kind {
        FramePipelineKind::DirectColor => 4,
        FramePipelineKind::Palette | FramePipelineKind::Ntsc => 1,
    }
}

fn frame_texture_format(kind: FramePipelineKind) -> TextureFormat {
    match kind {
        FramePipelineKind::DirectColor => TextureFormat::Rgba8Uint,
        FramePipelineKind::Palette | FramePipelineKind::Ntsc => TextureFormat::R8Uint,
    }
}

fn fragment_entry_point(kind: FramePipelineKind, surface_is_srgb: bool) -> &'static str {
    match (kind, surface_is_srgb) {
        (FramePipelineKind::DirectColor, true) => "fs_direct_srgb",
        (FramePipelineKind::DirectColor, false) => "fs_direct_linear",
        (FramePipelineKind::Palette, true) => "fs_palette_srgb",
        (FramePipelineKind::Palette, false) => "fs_palette_linear",
        (FramePipelineKind::Ntsc, true) => "fs_ntsc_srgb",
        (FramePipelineKind::Ntsc, false) => "fs_ntsc_linear",
    }
}

pub(super) fn encode_ntsc_texture(packed_ntsc_rgba8: Option<&[u8]>) -> (Box<[u8]>, Extent3d) {
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

    // texture 高さは実データ長から計算する (固定定数ではなく entry_stride 変更に追従)
    let width = NTSC_TEXTURE_WIDTH;
    let height = texture.len() / (width as usize * 4);
    let mut packed = Vec::with_capacity(texture.len());
    let chunks = texture.as_chunks::<4>();
    assert!(
        chunks.1.is_empty(),
        "packed NTSC texture must be a multiple of 4 bytes"
    );
    for chunk in chunks.0.iter() {
        packed.extend_from_slice(&u32::from_be_bytes(*chunk).to_le_bytes());
    }

    (
        packed.into_boxed_slice(),
        Extent3d {
            width,
            height: height as u32,
            depth_or_array_layers: 1,
        },
    )
}
