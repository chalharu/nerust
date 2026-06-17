use super::{DeviceLimitProfile, PresentationOptions, Renderer, fit_surface_size_to_limit};
use crate::{
    srgb_lut::SRGB_TO_LINEAR_LUT_BYTES,
    surface::{RenderSurface, SurfaceSize, SurfaceTargetSource},
    upload::FrameUploadLayout,
};
use nerust_screen_filter::presentation::ConsoleVideoAssets;
use nerust_screen_filter::{NTSC_TEXTURE_HEIGHT, NTSC_TEXTURE_WIDTH, PALETTE_TEXTURE_WIDTH};
use nerust_screen_logical::LogicalSize;
use nerust_screen_video::{VideoFrameFormat, VideoPresentation};
use wgpu::{
    BindGroupLayoutEntry, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CompositeAlphaMode, Device, Extent3d, FragmentState, MultisampleState, Origin3d,
    PipelineCompilationOptions, PipelineLayoutDescriptor, PresentMode, PrimitiveState, Queue,
    RenderPipeline, RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    Surface, SurfaceConfiguration, TexelCopyTextureInfo, Texture, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureViewDescriptor,
    TextureViewDimension,
};

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct FilterUniforms {
    source_width: u32,
    source_height: u32,
    output_width: u32,
    output_height: u32,
}

const BLACK_RGBA8_TEXEL: [u8; 4] = [0, 0, 0, 0xFF];

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(super) enum FramePipelineKind {
    DirectColor,
    Palette,
    Ntsc,
}

impl Renderer {
    pub async fn new<T: SurfaceTargetSource>(
        render_surface: &RenderSurface<T>,
        surface_size: SurfaceSize,
        presentation: &VideoPresentation,
        assets: Option<&ConsoleVideoAssets>,
        presentation_options: PresentationOptions,
    ) -> Result<Self, String> {
        Self::new_with_device_limit_profile(
            render_surface,
            surface_size,
            presentation,
            assets,
            DeviceLimitProfile::Default,
            presentation_options,
        )
        .await
    }

    pub async fn new_with_device_limit_profile<T: SurfaceTargetSource>(
        render_surface: &RenderSurface<T>,
        surface_size: SurfaceSize,
        presentation: &VideoPresentation,
        assets: Option<&ConsoleVideoAssets>,
        device_limit_profile: DeviceLimitProfile,
        presentation_options: PresentationOptions,
    ) -> Result<Self, String> {
        let pipeline_kind = frame_pipeline_kind(presentation, assets)?;

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

        let mut config = Self::surface_config(surface, &adapter, surface_size, &device)?;
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
        let (palette_rgba8, palette_size) = match assets {
            Some(assets) => (
                assets.palette_rgba8(),
                Extent3d {
                    width: PALETTE_TEXTURE_WIDTH,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            ),
            None => (
                BLACK_RGBA8_TEXEL.as_slice(),
                Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            ),
        };
        let palette_texture = create_texture_from_bytes(
            &device,
            &queue,
            "nerust_palette_texture",
            TextureFormat::Rgba8Uint,
            palette_size,
            palette_rgba8,
        );
        let (ntsc_data, ntsc_size) =
            encode_ntsc_texture(assets.and_then(|assets| assets.packed_ntsc_rgba8()));
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
        queue.write_buffer(&uniforms_buffer, 0, cast_bytes(&uniforms));

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
            frame_logical_size,
            content_size,
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

    fn surface_config(
        surface: &Surface<'_>,
        adapter: &wgpu::Adapter,
        surface_size: SurfaceSize,
        device: &Device,
    ) -> Result<SurfaceConfiguration, String> {
        let normalized_size =
            fit_surface_size_to_limit(surface_size, device.limits().max_texture_dimension_2d);
        surface
            .get_default_config(
                adapter,
                normalized_size.width.max(1),
                normalized_size.height.max(1),
            )
            .ok_or_else(|| "failed to derive a default surface configuration".to_string())
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

pub(super) fn composed_shader_source() -> String {
    [
        include_str!("../shader/vertex.wgsl"),
        include_str!("../shader/common.wgsl"),
        include_str!("../shader/palette_decode.wgsl"),
        include_str!("../shader/ntsc_decode.wgsl"),
        include_str!("../shader/presentation.wgsl"),
    ]
    .join("\n\n")
}

fn frame_pipeline_kind(
    presentation: &VideoPresentation,
    assets: Option<&ConsoleVideoAssets>,
) -> Result<FramePipelineKind, String> {
    match presentation.frame_format() {
        VideoFrameFormat::Rgba => Ok(FramePipelineKind::DirectColor),
        VideoFrameFormat::Palette => Ok(match assets.and_then(|a| a.packed_ntsc_rgba8()) {
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
