// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_console::Console;
use nerust_core::CoreOptions;
use nerust_core::controller::standard_controller::Buttons;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_timer::{CLOCK_RATE, Timer};
use std::sync::Arc;
#[cfg(target_os = "macos")]
use tao::platform::macos::EventLoopExtMacOS;
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::{ElementState, Event, KeyEvent, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopWindowTarget},
    keyboard::KeyCode,
    window::{Window as TaoWindow, WindowBuilder},
};
use wgpu::{
    BindGroup, BindGroupLayout, Color, ColorTargetState, ColorWrites, CommandEncoderDescriptor,
    CompositeAlphaMode, Device, Extent3d, Features, FilterMode, FragmentState, Instance, Limits,
    LoadOp, MultisampleState, Operations, Origin3d, PipelineCompilationOptions,
    PipelineLayoutDescriptor, PowerPreference, PresentMode, PrimitiveState, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    RequestAdapterOptions, Sampler, SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, StoreOp, Surface, SurfaceConfiguration, TexelCopyBufferLayout,
    TexelCopyTextureInfo, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureViewDescriptor, TextureViewDimension,
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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum MenuCommand {
    Pause,
    Resume,
    Reset,
    Quit,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum UserEvent {
    Menu(MenuCommand),
}

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "macos",
    target_os = "windows"
))]
mod app_menu {
    use super::{MenuCommand, TaoWindow, UserEvent};
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    use gtk::prelude::WidgetExt;
    use muda::{Menu, MenuEvent, MenuItem, Submenu};
    use tao::event_loop::EventLoopProxy;
    #[cfg(target_os = "macos")]
    use tao::platform::macos::WindowExtMacOS;
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    use tao::platform::unix::WindowExtUnix;
    #[cfg(target_os = "windows")]
    use tao::platform::windows::WindowExtWindows;

    pub(super) struct AppMenu {
        menu_bar: Menu,
        pause: MenuItem,
        resume: MenuItem,
    }

    impl AppMenu {
        pub(super) fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
            let menu_bar = Menu::new();
            let file_menu = Submenu::new("File", true);
            let emulation_menu = Submenu::new("Emulation", true);

            #[cfg(target_os = "macos")]
            {
                let app_menu = Submenu::new("App", true);
                app_menu
                    .append_items(&[
                        &muda::PredefinedMenuItem::about(None, None),
                        &muda::PredefinedMenuItem::separator(),
                        &muda::PredefinedMenuItem::services(None),
                        &muda::PredefinedMenuItem::separator(),
                        &muda::PredefinedMenuItem::hide(None),
                        &muda::PredefinedMenuItem::hide_others(None),
                        &muda::PredefinedMenuItem::show_all(None),
                        &muda::PredefinedMenuItem::separator(),
                        &muda::PredefinedMenuItem::quit(None),
                    ])
                    .unwrap();
                menu_bar.append(&app_menu).unwrap();
            }

            let pause = MenuItem::new("Pause", true, None);
            let resume = MenuItem::new("Resume", false, None);
            let reset = MenuItem::new("Reset", true, None);
            let quit = MenuItem::new("Quit", true, None);

            let pause_id = pause.id().clone();
            let resume_id = resume.id().clone();
            let reset_id = reset.id().clone();
            let quit_id = quit.id().clone();

            file_menu.append(&quit).unwrap();
            emulation_menu.append(&pause).unwrap();
            emulation_menu.append(&resume).unwrap();
            emulation_menu.append(&reset).unwrap();

            menu_bar.append(&file_menu).unwrap();
            menu_bar.append(&emulation_menu).unwrap();

            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let command = if event.id() == &pause_id {
                    Some(MenuCommand::Pause)
                } else if event.id() == &resume_id {
                    Some(MenuCommand::Resume)
                } else if event.id() == &reset_id {
                    Some(MenuCommand::Reset)
                } else if event.id() == &quit_id {
                    Some(MenuCommand::Quit)
                } else {
                    None
                };
                if let Some(command) = command {
                    let _ = proxy.send_event(UserEvent::Menu(command));
                }
            }));

            Self {
                menu_bar,
                pause,
                resume,
            }
        }

        pub(super) fn init_for_window(&self, window: &TaoWindow) {
            #[cfg(target_os = "windows")]
            unsafe {
                self.menu_bar.init_for_hwnd(window.hwnd() as _).unwrap();
            }

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            {
                self.menu_bar
                    .init_for_gtk_window(window.gtk_window(), window.default_vbox())
                    .unwrap();
                window.gtk_window().show_all();
            }

            #[cfg(target_os = "macos")]
            {
                let _ = window.ns_view();
                self.menu_bar.init_for_nsapp();
            }
        }

        pub(super) fn update(&self, paused: bool) {
            self.pause.set_enabled(!paused);
            self.resume.set_enabled(paused);
        }

        pub(super) fn clear_event_handler(&self) {
            MenuEvent::set_event_handler::<fn(MenuEvent)>(None);
        }
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "macos",
    target_os = "windows"
)))]
mod app_menu {
    use super::{TaoWindow, UserEvent};
    use tao::event_loop::EventLoopProxy;

    pub(super) struct AppMenu;

    impl AppMenu {
        pub(super) fn new(_proxy: EventLoopProxy<UserEvent>) -> Self {
            Self
        }

        pub(super) fn init_for_window(&self, _window: &TaoWindow) {}

        pub(super) fn update(&self, _paused: bool) {}

        pub(super) fn clear_event_handler(&self) {}
    }
}

use app_menu::AppMenu;

#[derive(Debug, Copy, Clone, PartialEq)]
struct Viewport {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn window_title(paused: bool) -> &'static str {
    if paused { "Nes -- Paused" } else { "Nes" }
}

fn keycode_button(code: KeyCode) -> Buttons {
    match code {
        KeyCode::KeyZ => Buttons::A,
        KeyCode::KeyX => Buttons::B,
        KeyCode::KeyC => Buttons::SELECT,
        KeyCode::KeyV => Buttons::START,
        KeyCode::ArrowUp => Buttons::UP,
        KeyCode::ArrowDown => Buttons::DOWN,
        KeyCode::ArrowLeft => Buttons::LEFT,
        KeyCode::ArrowRight => Buttons::RIGHT,
        _ => Buttons::empty(),
    }
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

fn create_window_builder(size: PhysicalSize, paused: bool) -> WindowBuilder {
    WindowBuilder::new()
        .with_title(window_title(paused))
        .with_inner_size(TaoLogicalSize::new(
            f64::from(size.width),
            f64::from(size.height),
        ))
}

#[derive(Clone)]
enum SurfaceTarget {
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
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    fn new(window: Arc<TaoWindow>, content_size: PhysicalSize) -> Self {
        Self::Gtk(GtkRenderTarget::new(&window, content_size))
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    fn new(window: Arc<TaoWindow>, _content_size: PhysicalSize) -> Self {
        Self::Window(window)
    }

    fn prepare(&self) {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        match self {
            Self::Gtk(target) => target.prepare(),
        }
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    fn surface_size(&self, fallback: TaoPhysicalSize<u32>) -> TaoPhysicalSize<u32> {
        match self {
            Self::Gtk(target) => target.surface_size(fallback),
        }
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    fn surface_size(&self, _fallback: TaoPhysicalSize<u32>) -> TaoPhysicalSize<u32> {
        match self {
            Self::Window(window) => window.inner_size(),
        }
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    fn create_surface(&self, instance: &Instance) -> Result<Surface<'static>, String> {
        match self {
            Self::Gtk(target) => unsafe {
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
    fn create_surface(&self, instance: &Instance) -> Result<Surface<'static>, String> {
        match self {
            Self::Window(window) => instance
                .create_surface(window.clone())
                .map_err(|err| format!("failed to create wgpu surface: {err:?}")),
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
#[derive(Clone)]
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

struct Renderer {
    instance: Instance,
    window: Arc<TaoWindow>,
    // The surface must drop before the GTK render target that backs its raw handles.
    surface: Surface<'static>,
    surface_target: SurfaceTarget,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    frame_texture: Texture,
    _frame_sampler: Sampler,
    _bind_group_layout: BindGroupLayout,
    bind_group: BindGroup,
    pipeline: RenderPipeline,
    logical_size: LogicalSize,
    content_size: PhysicalSize,
}

impl Renderer {
    async fn new(
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

    fn resize_to_target(&mut self) {
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

    fn update_frame_texture(&self, frame_buffer: &[u8]) {
        self.queue.write_texture(
            TexelCopyTextureInfo {
                texture: &self.frame_texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            frame_buffer,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((self.logical_size.width * 4) as u32),
                rows_per_image: Some(self.logical_size.height as u32),
            },
            Extent3d {
                width: self.logical_size.width as u32,
                height: self.logical_size.height as u32,
                depth_or_array_layers: 1,
            },
        );
    }

    fn render(&mut self, frame_buffer: &[u8]) -> Result<(), String> {
        self.update_frame_texture(frame_buffer);

        let (surface_texture, suboptimal) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.resize_to_target();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                return Ok(());
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
        Ok(())
    }
}

pub struct Window {
    event_loop: Option<EventLoop<UserEvent>>,
    window: Option<Arc<TaoWindow>>,
    renderer: Option<Renderer>,
    last_render_error: Option<String>,
    timer: Timer,
    keys: Buttons,
    paused: bool,
    console: Console,
    app_menu: AppMenu,
    physical_size: PhysicalSize,
    logical_size: LogicalSize,
}

impl Window {
    pub fn new() -> Self {
        let screen_buffer = ScreenBuffer::new(
            FilterType::NtscComposite,
            LogicalSize {
                width: 256,
                height: 240,
            },
        );
        let physical_size = screen_buffer.physical_size();
        let logical_size = screen_buffer.logical_size();
        let speaker = OpenAl::new(48_000, CLOCK_RATE as i32, 128, 20);
        let console = Console::new(speaker, screen_buffer);

        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        #[cfg(target_os = "macos")]
        let event_loop = {
            let mut event_loop = event_loop;
            // Explicitly let macOS activate the app even when another app is currently active.
            event_loop.set_activate_ignoring_other_apps(true);
            event_loop
        };
        let proxy = event_loop.create_proxy();
        let app_menu = AppMenu::new(proxy);

        Self {
            event_loop: Some(event_loop),
            window: None,
            renderer: None,
            last_render_error: None,
            timer: Timer::new(),
            keys: Buttons::empty(),
            paused: false,
            console,
            app_menu,
            physical_size,
            logical_size,
        }
    }

    pub fn load(&mut self, data: Vec<u8>) {
        self.load_with_options(data, CoreOptions::default());
    }

    pub fn load_with_options(&mut self, data: Vec<u8>, options: CoreOptions) {
        self.console.load_with_options(data, options);
    }

    pub fn run(mut self) {
        self.console.resume();
        let event_loop = self.event_loop.take().unwrap();

        event_loop.run(move |event, event_loop, control_flow| {
            *control_flow = ControlFlow::Poll;

            match event {
                Event::NewEvents(StartCause::Init) => self.ensure_window(event_loop),
                Event::WindowEvent {
                    event, window_id, ..
                } if self
                    .window
                    .as_ref()
                    .is_some_and(|window| window_id == window.id()) =>
                {
                    match event {
                        WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                        WindowEvent::Focused(false) => self.clear_keys(),
                        WindowEvent::Resized(_) => {
                            if let Some(renderer) = self.renderer.as_mut() {
                                renderer.resize_to_target();
                            }
                        }
                        WindowEvent::KeyboardInput { event, .. } => self.on_keyboard_input(event),
                        _ => (),
                    }
                }
                Event::RedrawRequested(window_id)
                    if self
                        .window
                        .as_ref()
                        .is_some_and(|window| window_id == window.id()) =>
                {
                    self.on_update()
                }
                Event::MainEventsCleared => {
                    if let Some(window) = self.window.as_ref() {
                        self.timer.wait();
                        window.request_redraw();
                    }
                }
                Event::UserEvent(UserEvent::Menu(command)) => {
                    self.on_menu_command(control_flow, command);
                }
                Event::LoopDestroyed => {
                    self.app_menu.clear_event_handler();
                }
                _ => (),
            }
        });
    }

    fn ensure_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.window.is_some() {
            return;
        }

        let window = Arc::new(
            create_window_builder(self.physical_size, self.paused)
                .build(event_loop)
                .unwrap(),
        );
        let surface_target = SurfaceTarget::new(window.clone(), self.physical_size);
        self.app_menu.init_for_window(&window);
        self.app_menu.update(self.paused);
        let renderer = pollster::block_on(Renderer::new(
            window.clone(),
            surface_target,
            self.logical_size,
            self.physical_size,
        ))
        .unwrap();
        self.window = Some(window);
        self.renderer = Some(renderer);
    }

    fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }

        self.paused = paused;
        if self.paused {
            self.console.pause();
        } else {
            self.console.resume();
        }
        self.app_menu.update(self.paused);
        if let Some(window) = self.window.as_ref() {
            window.set_title(window_title(self.paused));
        }
    }

    fn on_menu_command(&mut self, control_flow: &mut ControlFlow, command: MenuCommand) {
        match command {
            MenuCommand::Pause => self.set_paused(true),
            MenuCommand::Resume => self.set_paused(false),
            MenuCommand::Reset => self.console.reset(),
            MenuCommand::Quit => *control_flow = ControlFlow::Exit,
        }
    }

    fn on_update(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let render_result = self
            .console
            .with_frame_buffer(|frame_buffer| renderer.render(frame_buffer));

        match render_result {
            Ok(()) => {
                self.last_render_error = None;
            }
            Err(err) => {
                let should_log = self.last_render_error.as_deref() != Some(err.as_str());
                self.last_render_error = Some(err.clone());
                if should_log {
                    log::error!("{err}");
                }
            }
        }
    }

    fn on_keyboard_input(&mut self, input: KeyEvent) {
        let code = match input.physical_key {
            KeyCode::Space if input.state == ElementState::Pressed && !input.repeat => {
                self.set_paused(!self.paused);
                Buttons::empty()
            }
            KeyCode::Escape if input.state == ElementState::Released => {
                self.console.reset();
                Buttons::empty()
            }
            code => keycode_button(code),
        };

        self.keys = match input.state {
            ElementState::Pressed => self.keys | code,
            ElementState::Released => self.keys & !code,
            _ => self.keys,
        };
        self.console.set_pad1(self.keys);
    }

    fn clear_keys(&mut self) {
        self.keys = Buttons::empty();
        self.console.set_pad1(self.keys);
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{compute_viewport, keycode_button};
    use nerust_core::controller::standard_controller::Buttons;
    use nerust_screen_traits::PhysicalSize;
    use tao::{dpi::PhysicalSize as TaoPhysicalSize, keyboard::KeyCode};

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

    #[test]
    fn keycode_mapping_matches_controller_layout() {
        assert_eq!(keycode_button(KeyCode::KeyZ).bits(), Buttons::A.bits());
        assert_eq!(keycode_button(KeyCode::KeyX).bits(), Buttons::B.bits());
        assert_eq!(keycode_button(KeyCode::ArrowUp).bits(), Buttons::UP.bits());
        assert_eq!(
            keycode_button(KeyCode::ArrowRight).bits(),
            Buttons::RIGHT.bits()
        );
        assert_eq!(
            keycode_button(KeyCode::Enter).bits(),
            Buttons::empty().bits()
        );
    }
}
