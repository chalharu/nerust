// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_console::Console;
use nerust_core::controller::standard_controller::Buttons;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_filter::FilterType;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_timer::{CLOCK_RATE, Timer};
use std::sync::Arc;
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
#[cfg(target_os = "macos")]
use winit::platform::macos::EventLoopBuilderExtMacOS;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize as WinitLogicalSize, PhysicalSize as WinitPhysicalSize},
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window as WinitWindow, WindowAttributes},
};

#[allow(
    dead_code,
    reason = "menu integration is only active on macOS and Windows"
)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum MenuCommand {
    Pause,
    Resume,
    Reset,
    Quit,
}

#[allow(
    dead_code,
    reason = "menu integration is only active on macOS and Windows"
)]
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum UserEvent {
    Menu(MenuCommand),
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod app_menu {
    use super::{MenuCommand, UserEvent};
    use muda::{Menu, MenuEvent, MenuItem, Submenu};
    #[cfg(target_os = "windows")]
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use winit::{event_loop::EventLoopProxy, window::Window as WinitWindow};

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

            MenuEvent::set_event_handler(Some(move |event| {
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

        pub(super) fn init_for_window(&self, window: &WinitWindow) {
            #[cfg(target_os = "windows")]
            {
                if let RawWindowHandle::Win32(handle) = window.window_handle().unwrap().as_raw() {
                    unsafe {
                        self.menu_bar.init_for_hwnd(handle.hwnd.get());
                    }
                }
            }

            #[cfg(target_os = "macos")]
            {
                let _ = window;
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

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod app_menu {
    use super::UserEvent;
    use winit::{event_loop::EventLoopProxy, window::Window as WinitWindow};

    pub(super) struct AppMenu;

    impl AppMenu {
        pub(super) fn new(_proxy: EventLoopProxy<UserEvent>) -> Self {
            Self
        }

        pub(super) fn init_for_window(&self, _window: &WinitWindow) {}

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

fn create_window_attributes(size: PhysicalSize, paused: bool) -> WindowAttributes {
    WinitWindow::default_attributes()
        .with_inner_size(WinitLogicalSize::new(
            f64::from(size.width),
            f64::from(size.height),
        ))
        .with_title(window_title(paused))
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

fn compute_viewport(window_size: WinitPhysicalSize<u32>, content_size: PhysicalSize) -> Viewport {
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

struct Renderer {
    instance: Instance,
    window: Arc<WinitWindow>,
    surface: Surface<'static>,
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
        window: Arc<WinitWindow>,
        logical_size: LogicalSize,
        content_size: PhysicalSize,
    ) -> Result<Self, String> {
        let instance = Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|err| format!("failed to create wgpu surface: {err:?}"))?;
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

        let mut config = Self::surface_config(&surface, &adapter, window.inner_size())?;
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
        window_size: WinitPhysicalSize<u32>,
    ) -> Result<SurfaceConfiguration, String> {
        surface
            .get_default_config(adapter, window_size.width.max(1), window_size.height.max(1))
            .ok_or_else(|| "failed to derive a default surface configuration".to_string())
    }

    fn resize(&mut self, size: WinitPhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    fn recreate_surface(&mut self) -> Result<(), String> {
        self.surface = self
            .instance
            .create_surface(self.window.clone())
            .map_err(|err| format!("failed to recreate wgpu surface: {err:?}"))?;
        self.surface.configure(&self.device, &self.config);
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
                self.resize(self.window.inner_size());
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
            WinitPhysicalSize::new(self.config.width, self.config.height),
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
            self.resize(self.window.inner_size());
        }
        Ok(())
    }
}

pub struct Window {
    window: Option<Arc<WinitWindow>>,
    renderer: Option<Renderer>,
    event_loop: Option<EventLoop<UserEvent>>,
    timer: Timer,
    keys: Buttons,
    paused: bool,
    console: Console,
    physical_size: PhysicalSize,
    logical_size: LogicalSize,
    app_menu: AppMenu,
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

        let mut event_loop_builder = EventLoop::<UserEvent>::with_user_event();
        #[cfg(target_os = "macos")]
        event_loop_builder.with_default_menu(false);
        let event_loop = event_loop_builder.build().unwrap();
        let proxy = event_loop.create_proxy();
        let app_menu = AppMenu::new(proxy);

        Self {
            window: None,
            renderer: None,
            event_loop: Some(event_loop),
            timer: Timer::new(),
            keys: Buttons::empty(),
            paused: false,
            console,
            physical_size,
            logical_size,
            app_menu,
        }
    }

    pub fn load(&mut self, data: Vec<u8>) {
        self.console.load(data);
    }

    pub fn run(&mut self) {
        self.console.resume();
        let event_loop = self.event_loop.take().unwrap();
        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop.run_app(self).unwrap();
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

    fn on_menu_command(&mut self, event_loop: &ActiveEventLoop, command: MenuCommand) {
        match command {
            MenuCommand::Pause => self.set_paused(true),
            MenuCommand::Resume => self.set_paused(false),
            MenuCommand::Reset => self.console.reset(),
            MenuCommand::Quit => {
                self.on_close();
                event_loop.exit();
            }
        }
    }

    fn on_load(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window = Arc::new(
            event_loop
                .create_window(create_window_attributes(self.physical_size, self.paused))
                .unwrap(),
        );
        self.app_menu.init_for_window(&window);
        self.app_menu.update(self.paused);
        let renderer = pollster::block_on(Renderer::new(
            window.clone(),
            self.logical_size,
            self.physical_size,
        ))
        .unwrap();
        window.request_redraw();
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn on_update(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        self.console.with_frame_buffer(|frame_buffer| {
            if let Err(err) = renderer.render(frame_buffer) {
                log::error!("{err}");
            }
        });
    }

    fn on_resize(&mut self, size: WinitPhysicalSize<u32>) {
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.resize(size);
        }
    }

    fn on_close(&mut self) {
        self.renderer = None;
        self.window = None;
    }

    fn on_keyboard_input(&mut self, input: KeyEvent) {
        let code = match input.physical_key {
            PhysicalKey::Code(KeyCode::Space)
                if input.state == ElementState::Pressed && !input.repeat =>
            {
                self.set_paused(!self.paused);
                Buttons::empty()
            }
            PhysicalKey::Code(KeyCode::Escape) if input.state == ElementState::Released => {
                self.console.reset();
                Buttons::empty()
            }
            PhysicalKey::Code(code) => keycode_button(code),
            _ => Buttons::empty(),
        };

        self.keys = match input.state {
            ElementState::Pressed => self.keys | code,
            ElementState::Released => self.keys & !code,
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

impl ApplicationHandler<UserEvent> for Window {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.on_load(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.on_close();
                event_loop.exit();
            }
            WindowEvent::Focused(false) => self.clear_keys(),
            WindowEvent::Resized(size) => self.on_resize(size),
            WindowEvent::KeyboardInput { event, .. } => self.on_keyboard_input(event),
            WindowEvent::RedrawRequested => self.on_update(),
            _ => (),
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Menu(event) => self.on_menu_command(event_loop, event),
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            self.timer.wait();
            window.request_redraw();
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.on_close();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.on_close();
        self.app_menu.clear_event_handler();
    }
}

#[cfg(test)]
mod tests {
    use super::{compute_viewport, keycode_button};
    use nerust_core::controller::standard_controller::Buttons;
    use nerust_screen_traits::PhysicalSize;
    use winit::{dpi::PhysicalSize as WinitPhysicalSize, keyboard::KeyCode};

    #[test]
    fn viewport_preserves_aspect_ratio() {
        let viewport = compute_viewport(
            WinitPhysicalSize::new(1600, 900),
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
