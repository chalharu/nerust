// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

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
#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
use {
    gtk::{
        EventBox,
        gdk::prelude::DisplayExtManual,
        prelude::{BoxExt, ObjectType, WidgetExt},
    },
    raw_window_handle::{
        HandleError, RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
        XlibDisplayHandle, XlibWindowHandle,
    },
    std::ptr::NonNull,
    tao::platform::unix::WindowExtUnix,
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

pub struct SurfaceTarget {
    kind: SurfaceTargetKind,
}

enum SurfaceTargetKind {
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    Window(Arc<TaoWindow>),
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    Gtk(GtkRenderTarget),
}

impl SurfaceTarget {
    pub fn new(window: Arc<TaoWindow>, content_size: PhysicalSize) -> Self {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            Self {
                kind: SurfaceTargetKind::Gtk(GtkRenderTarget::new(&window, content_size)),
            }
        }

        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        {
            let _ = content_size;
            Self {
                kind: SurfaceTargetKind::Window(window),
            }
        }
    }

    fn prepare(&self) {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        match &self.kind {
            SurfaceTargetKind::Gtk(target) => target.prepare(),
        }
    }

    fn surface_size(&self, fallback: TaoPhysicalSize<u32>) -> TaoPhysicalSize<u32> {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            match &self.kind {
                SurfaceTargetKind::Gtk(target) => target.surface_size(fallback),
            }
        }

        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        {
            let _ = fallback;
            match &self.kind {
                SurfaceTargetKind::Window(window) => window.inner_size(),
            }
        }
    }

    fn create_surface(&self, instance: &Instance) -> Result<Surface<'static>, String> {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            match &self.kind {
                SurfaceTargetKind::Gtk(target) => unsafe {
                    let raw_display_handle = target
                        .raw_display_handle()
                        .map_err(|err| format!("failed to acquire raw display handle: {err:?}"))?;
                    let raw_window_handle = target
                        .raw_window_handle()
                        .map_err(|err| format!("failed to acquire raw window handle: {err:?}"))?;
                    instance
                        .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                            raw_display_handle: Some(raw_display_handle),
                            raw_window_handle,
                        })
                        .map_err(|err| format!("failed to create wgpu surface: {err:?}"))
                },
            }
        }

        #[cfg(not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        )))]
        {
            match &self.kind {
                SurfaceTargetKind::Window(window) => instance
                    .create_surface(window.clone())
                    .map_err(|err| format!("failed to create wgpu surface: {err:?}")),
            }
        }
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
struct GtkRenderTarget {
    widget: EventBox,
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
impl GtkRenderTarget {
    fn new(window: &TaoWindow, content_size: PhysicalSize) -> Self {
        let widget = EventBox::new();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_size_request(content_size.width as i32, content_size.height as i32);
        window
            .default_vbox()
            .expect("tao default_vbox must exist for Linux menu integration")
            .pack_start(&widget, true, true, 0);

        Self { widget }
    }

    fn prepare(&self) {
        self.widget.realize();
    }

    fn surface_size(&self, fallback: TaoPhysicalSize<u32>) -> TaoPhysicalSize<u32> {
        let width = self.widget.allocated_width();
        let height = self.widget.allocated_height();
        if width > 0 && height > 0 {
            let scale = self.widget.scale_factor().max(1) as u32;
            TaoPhysicalSize::new(width as u32 * scale, height as u32 * scale)
        } else {
            fallback
        }
    }

    fn gdk_window(&self) -> Result<gtk::gdk::Window, HandleError> {
        self.widget.window().ok_or(HandleError::Unavailable)
    }

    fn is_wayland(&self) -> bool {
        self.widget.display().backend().is_wayland()
    }

    fn raw_window_handle(&self) -> Result<RawWindowHandle, HandleError> {
        let window = self.gdk_window()?;
        if self.is_wayland() {
            let surface = unsafe {
                gdk_wayland_sys::gdk_wayland_window_get_wl_surface(window.as_ptr() as *mut _)
            };
            let surface = NonNull::new(surface)
                .ok_or(HandleError::Unavailable)?
                .cast();
            Ok(RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)))
        } else {
            let xid = unsafe { gdk_x11_sys::gdk_x11_window_get_xid(window.as_ptr() as *mut _) };
            Ok(RawWindowHandle::Xlib(XlibWindowHandle::new(xid)))
        }
    }

    fn raw_display_handle(&self) -> Result<RawDisplayHandle, HandleError> {
        let display = self.widget.display();
        if self.is_wayland() {
            let display = unsafe {
                gdk_wayland_sys::gdk_wayland_display_get_wl_display(display.as_ptr() as *mut _)
            };
            let display = NonNull::new(display)
                .ok_or(HandleError::Unavailable)?
                .cast();
            Ok(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                display,
            )))
        } else {
            let display =
                unsafe { gdk_x11_sys::gdk_x11_display_get_xdisplay(display.as_ptr() as *mut _) };
            let display = NonNull::new(display as *mut _).ok_or(HandleError::Unavailable)?;
            let screen = self.widget.screen().ok_or(HandleError::Unavailable)?;
            let screen =
                unsafe { gdk_x11_sys::gdk_x11_screen_get_screen_number(screen.as_ptr() as *mut _) }
                    as _;
            Ok(RawDisplayHandle::Xlib(XlibDisplayHandle::new(
                Some(display),
                screen,
            )))
        }
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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
struct FrameUploadLayout {
    copy_bytes_per_row: u32,
    upload_bytes_per_row: u32,
    buffer_size: u64,
}

impl FrameUploadLayout {
    fn for_logical_size(logical_size: LogicalSize) -> Result<Self, String> {
        let copy_bytes_per_row = logical_size
            .width
            .checked_mul(4)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "frame upload row size overflowed u32".to_string())?;
        let upload_bytes_per_row =
            align_copy_bytes_per_row(copy_bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer_size = u64::from(upload_bytes_per_row)
            .checked_mul(logical_size.height as u64)
            .ok_or_else(|| "frame upload buffer size overflowed u64".to_string())?;
        Ok(Self {
            copy_bytes_per_row,
            upload_bytes_per_row,
            buffer_size,
        })
    }
}

fn align_copy_bytes_per_row(bytes_per_row: u32, alignment: u32) -> u32 {
    bytes_per_row.div_ceil(alignment) * alignment
}

fn pack_frame_rows(
    source: &[u8],
    height: usize,
    destination: &mut [u8],
    layout: FrameUploadLayout,
) {
    let copy_bytes_per_row = layout.copy_bytes_per_row as usize;
    let upload_bytes_per_row = layout.upload_bytes_per_row as usize;
    debug_assert_eq!(source.len(), copy_bytes_per_row * height);
    debug_assert!(destination.len() >= upload_bytes_per_row * height);

    if copy_bytes_per_row == upload_bytes_per_row {
        destination[..source.len()].copy_from_slice(source);
        return;
    }

    for (source_row, destination_row) in source
        .chunks_exact(copy_bytes_per_row)
        .zip(destination.chunks_exact_mut(upload_bytes_per_row))
        .take(height)
    {
        destination_row[..copy_bytes_per_row].copy_from_slice(source_row);
    }
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
    use super::{FrameUploadLayout, compute_viewport, pack_frame_rows};
    use nerust_screen_traits::{LogicalSize, PhysicalSize};
    use tao::dpi::PhysicalSize as TaoPhysicalSize;

    #[test]
    fn aligned_upload_layout_keeps_native_row_pitch() {
        let layout = FrameUploadLayout::for_logical_size(LogicalSize {
            width: 256,
            height: 240,
        })
        .expect("layout should be valid");

        assert_eq!(layout.copy_bytes_per_row, 1024);
        assert_eq!(layout.upload_bytes_per_row, 1024);
        assert_eq!(layout.buffer_size, 245_760);
    }

    #[test]
    fn unaligned_upload_layout_rounds_up_to_copy_alignment() {
        let layout = FrameUploadLayout::for_logical_size(LogicalSize {
            width: 602,
            height: 240,
        })
        .expect("layout should be valid");

        assert_eq!(layout.copy_bytes_per_row, 2408);
        assert_eq!(
            layout.upload_bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
            0
        );
        assert_eq!(layout.upload_bytes_per_row, 2560);
        assert_eq!(layout.buffer_size, 614_400);
    }

    #[test]
    fn pack_frame_rows_inserts_row_padding_without_reordering_pixels() {
        let layout = FrameUploadLayout {
            copy_bytes_per_row: 8,
            upload_bytes_per_row: 16,
            buffer_size: 32,
        };
        let source = [
            1_u8, 2, 3, 4, 5, 6, 7, 8, //
            9, 10, 11, 12, 13, 14, 15, 16,
        ];
        let mut destination = [0_u8; 32];

        pack_frame_rows(&source, 2, &mut destination, layout);

        assert_eq!(&destination[0..8], &source[0..8]);
        assert_eq!(&destination[16..24], &source[8..16]);
        assert_eq!(&destination[8..16], &[0; 8]);
        assert_eq!(&destination[24..32], &[0; 8]);
    }

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
