// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use glutin::config::{Config, ConfigTemplateBuilder};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentContext, PossiblyCurrentContext,
    Version,
};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::prelude::*;
use glutin::surface::{Surface, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use nerust_console::{Console, ConsoleMetrics, PreviewFrame};
use nerust_core::controller::standard_controller::Buttons;
use nerust_persistence::{
    SidecarPaths, ThumbnailSource, allocate_next_slot_id, load_mapper_save, load_state_slot,
    resolve_sidecars, scan_state_slots_for_target, state_slot_path, write_mapper_save,
    write_recovery_mapper_save, write_state_slot,
};
use nerust_screen_filter::FilterType;
use nerust_screen_opengl::GlView;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;
use raw_window_handle::HasWindowHandle;
use std::f64;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize as WinitLogicalSize, PhysicalSize as WinitPhysicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window as WinitWindow, WindowAttributes};

fn create_window_attributes(size: PhysicalSize) -> WindowAttributes {
    WinitWindow::default_attributes()
        .with_inner_size(WinitLogicalSize::new(
            f64::from(size.width),
            f64::from(size.height),
        ))
        .with_title("Nes")
}

fn create_gl_context(window: &WinitWindow, gl_config: &Config) -> NotCurrentContext {
    let raw_window_handle = window.window_handle().ok().map(|handle| handle.as_raw());
    let gl_display = gl_config.display();
    let context_attributes = ContextAttributesBuilder::new()
        .with_profile(GlProfile::Core)
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(raw_window_handle);
    let fallback_context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
        .build(raw_window_handle);

    unsafe {
        gl_display
            .create_context(gl_config, &context_attributes)
            .unwrap_or_else(|_| {
                gl_display
                    .create_context(gl_config, &fallback_context_attributes)
                    .expect("failed to create GL context")
            })
    }
}

fn create_window(
    event_loop: &ActiveEventLoop,
    size: PhysicalSize,
) -> (WinitWindow, PossiblyCurrentContext, Surface<WindowSurface>) {
    let template = ConfigTemplateBuilder::new().with_alpha_size(8);
    let display_builder =
        DisplayBuilder::new().with_window_attributes(Some(create_window_attributes(size)));
    let (window, gl_config) = display_builder
        .build(event_loop, template, |configs| {
            configs
                .reduce(|accum, config| {
                    if config.num_samples() > accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .unwrap()
        })
        .unwrap();
    let window = window.unwrap();
    let attrs = window
        .build_surface_attributes(Default::default())
        .expect("failed to build GL surface attributes");
    let gl_surface = unsafe {
        gl_config
            .display()
            .create_window_surface(&gl_config, &attrs)
            .expect("failed to create GL surface")
    };
    let gl_context = create_gl_context(&window, &gl_config)
        .make_current(&gl_surface)
        .expect("failed to make GL context current");

    let gl_display = gl_config.display();
    GlView::load_with(|symbol| {
        let symbol = CString::new(symbol).unwrap();
        gl_display.get_proc_address(symbol.as_c_str()).cast()
    });

    let _ =
        gl_surface.set_swap_interval(&gl_context, SwapInterval::Wait(NonZeroU32::new(1).unwrap()));

    (window, gl_context, gl_surface)
}

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);

fn window_title(paused: bool, console_metrics: ConsoleMetrics) -> String {
    let state = if paused { "Nes -- Paused" } else { "Nes" };
    if console_metrics.loaded {
        format!(
            "{state} | FPS {:.1} | Speed x{:.2}",
            console_metrics.emulation_fps, console_metrics.speed_multiplier
        )
    } else {
        format!("{state} | No ROM")
    }
}

pub struct Window {
    view: Option<GlView>,
    gl_context: Option<PossiblyCurrentContext>,
    gl_surface: Option<Surface<WindowSurface>>,
    window: Option<WinitWindow>,
    event_loop: Option<EventLoop<()>>,
    keys: Buttons,
    paused: bool,
    console: Console,
    physical_size: PhysicalSize,
    last_title_update: Instant,
    last_presented_frame_counter: u64,
    needs_redraw: bool,
    rom_path: Option<PathBuf>,
    sidecars: Option<SidecarPaths>,
    mapper_save_flush_allowed: bool,
    mapper_save_recovery_written: bool,
    active_slot_id: Option<u64>,
}

impl Window {
    pub fn new() -> Self {
        let filter_type = FilterType::NtscComposite;
        let source_logical_size = LogicalSize {
            width: 256,
            height: 240,
        };
        let speaker = OpenAl::new(48000, CLOCK_RATE as i32, 128, 20);
        let console = Console::new_gpu(speaker, filter_type, source_logical_size);
        let physical_size = console.video().presentation().physical_size();

        Self {
            event_loop: Some(EventLoop::new().unwrap()),
            view: None,
            gl_context: None,
            gl_surface: None,
            window: None,
            keys: Buttons::empty(),
            paused: false,
            console,
            physical_size,
            last_title_update: Instant::now(),
            last_presented_frame_counter: 0,
            needs_redraw: true,
            rom_path: None,
            sidecars: None,
            mapper_save_flush_allowed: true,
            mapper_save_recovery_written: false,
            active_slot_id: None,
        }
    }

    pub fn load(&mut self, rom_path: Option<PathBuf>, data: Vec<u8>) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before load failed: {error}");
            return;
        }
        if self.console.load(data).is_ok() {
            self.rom_path = rom_path;
            self.sidecars = self.rom_path.as_deref().map(resolve_sidecars);
            self.mapper_save_flush_allowed = true;
            self.mapper_save_recovery_written = false;
            self.active_slot_id = None;
            if let Err(error) = self.load_mapper_save_if_available() {
                self.mapper_save_flush_allowed = false;
                log::warn!("mapper save auto-load failed: {error}");
            }
        }
    }

    pub fn run(&mut self) {
        self.console.resume();
        let event_loop = self.event_loop.take().unwrap();
        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop.run_app(self).unwrap();
    }

    fn on_load(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let (window, gl_context, gl_surface) = create_window(event_loop, self.physical_size);
        let mut view = GlView::new();
        view.use_vao(true);
        view.on_load(self.console.video().presentation()).unwrap();
        let initial_size = window.inner_size();

        self.window = Some(window);
        self.gl_context = Some(gl_context);
        self.gl_surface = Some(gl_surface);
        self.view = Some(view);
        self.on_resize(initial_size);
        self.refresh_window_title();
    }

    fn on_update(&mut self) {
        self.console
            .video()
            .frame_buffer()
            .with_bytes(|frame_buffer| {
                self.view.as_ref().unwrap().on_update(frame_buffer.as_ptr());
            });
        self.gl_surface
            .as_ref()
            .unwrap()
            .swap_buffers(self.gl_context.as_ref().unwrap())
            .unwrap();
        self.last_presented_frame_counter = self.console.metrics().frame_counter;
        self.needs_redraw = false;
        self.maybe_refresh_window_title(Instant::now());
    }

    fn on_resize(&mut self, physical_size: WinitPhysicalSize<u32>) {
        let Some(width) = NonZeroU32::new(physical_size.width) else {
            return;
        };
        let Some(height) = NonZeroU32::new(physical_size.height) else {
            return;
        };

        self.gl_surface
            .as_ref()
            .unwrap()
            .resize(self.gl_context.as_ref().unwrap(), width, height);

        let rate_x = physical_size.width as f32 / self.physical_size.width;
        let rate_y = physical_size.height as f32 / self.physical_size.height;
        let rate = f32::min(rate_x, rate_y);
        let scale_x = rate / rate_x;
        let scale_y = rate / rate_y;

        self.view.as_mut().unwrap().on_resize(
            scale_x,
            scale_y,
            physical_size.width as i32,
            physical_size.height as i32,
        );
        self.needs_redraw = true;
    }

    fn on_close(&mut self) -> bool {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before close failed: {error}");
        }
        if let Some(view) = self.view.as_mut() {
            view.on_close();
        }
        self.view = None;
        self.gl_surface = None;
        self.gl_context = None;
        self.window = None;
        true
    }

    fn current_window_title(&self) -> String {
        window_title(self.paused, self.console.metrics())
    }

    fn refresh_window_title(&mut self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(self.current_window_title().as_str());
        }
    }

    fn maybe_refresh_window_title(&mut self, now: Instant) {
        if now.duration_since(self.last_title_update) >= TITLE_UPDATE_INTERVAL {
            self.last_title_update = now;
            self.refresh_window_title();
        }
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
            self.needs_redraw = true;
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
        self.refresh_window_title();
    }

    fn sync_paused_from_console(&mut self) {
        let was_paused = self.paused;
        self.paused = self.console.metrics().paused;
        if was_paused && !self.paused {
            self.needs_redraw = true;
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }
        self.refresh_window_title();
    }

    fn select_adjacent_slot(&mut self, forward: bool) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        let (rom_identity, options) = match self.console.persistence_target() {
            Ok(target) => target,
            Err(error) => {
                log::warn!("state slot target unavailable: {error}");
                return;
            }
        };
        let slots = match scan_state_slots_for_target(&sidecars.states_dir, rom_identity, options) {
            Ok(slots) => slots,
            Err(error) => {
                log::warn!("state slot scan failed: {error}");
                return;
            }
        };
        let Some(next_slot_id) = (!slots.is_empty()).then(|| {
            if let Some(current) = self.active_slot_id
                && let Some(index) = slots.iter().position(|slot| slot.slot_id == current)
            {
                let offset = if forward {
                    (index + 1) % slots.len()
                } else {
                    (index + slots.len() - 1) % slots.len()
                };
                slots[offset].slot_id
            } else if forward {
                slots[0].slot_id
            } else {
                slots[slots.len() - 1].slot_id
            }
        }) else {
            return;
        };
        self.active_slot_id = Some(next_slot_id);
        log::info!("selected save slot {next_slot_id}");
    }

    fn on_keyboard_input(&mut self, input: KeyEvent) {
        // とりあえず、pad1のみ次の通りとする。
        // A      -> Z
        // B      -> X
        // Select -> C
        // Start  -> V
        // Up     -> Up
        // Down   -> Down
        // Left   -> Left
        // Right  -> Right
        let code = match input.physical_key {
            PhysicalKey::Code(KeyCode::KeyZ) => Buttons::A,
            PhysicalKey::Code(KeyCode::KeyX) => Buttons::B,
            PhysicalKey::Code(KeyCode::KeyC) => Buttons::SELECT,
            PhysicalKey::Code(KeyCode::KeyV) => Buttons::START,
            PhysicalKey::Code(KeyCode::ArrowUp) => Buttons::UP,
            PhysicalKey::Code(KeyCode::ArrowDown) => Buttons::DOWN,
            PhysicalKey::Code(KeyCode::ArrowLeft) => Buttons::LEFT,
            PhysicalKey::Code(KeyCode::ArrowRight) => Buttons::RIGHT,
            PhysicalKey::Code(KeyCode::Space) if input.state == ElementState::Pressed => {
                self.set_paused(!self.paused);
                Buttons::empty()
            }
            PhysicalKey::Code(KeyCode::Escape) => {
                if input.state == ElementState::Released {
                    let _ = self.console.reset();
                }
                Buttons::empty()
            }
            PhysicalKey::Code(KeyCode::F5) if input.state == ElementState::Released => {
                self.save_active_slot_or_new();
                Buttons::empty()
            }
            PhysicalKey::Code(KeyCode::F6) if input.state == ElementState::Released => {
                self.select_adjacent_slot(true);
                Buttons::empty()
            }
            PhysicalKey::Code(KeyCode::F7) if input.state == ElementState::Released => {
                self.select_adjacent_slot(false);
                Buttons::empty()
            }
            PhysicalKey::Code(KeyCode::F8) if input.state == ElementState::Released => {
                if let Some(slot_id) = self.active_slot_id {
                    self.load_slot(slot_id);
                }
                Buttons::empty()
            }
            _ => Buttons::empty(),
        };
        self.keys = match input.state {
            ElementState::Pressed => self.keys | code,
            ElementState::Released => self.keys & !code,
        };
        self.console.set_pad1(self.keys);
    }

    fn load_mapper_save_if_available(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return Ok(());
        };
        if let Some(bytes) =
            load_mapper_save(&sidecars.mapper_save_path).map_err(|error| error.to_string())?
        {
            self.console
                .import_mapper_save(bytes)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn flush_mapper_save(&mut self) -> Result<(), String> {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return Ok(());
        };
        if !self.mapper_save_flush_allowed {
            if self.mapper_save_recovery_written {
                return Ok(());
            }
            if let Some(bytes) = self
                .console
                .export_mapper_save()
                .map_err(|error| error.to_string())?
            {
                let path = write_recovery_mapper_save(&sidecars.mapper_save_path, &bytes)
                    .map_err(|error| error.to_string())?;
                self.mapper_save_recovery_written = true;
                log::warn!(
                    "mapper save auto-load failed earlier; wrote recovery save to {}",
                    path.display()
                );
            }
            return Ok(());
        }
        let bytes = self
            .console
            .export_mapper_save()
            .map_err(|error| error.to_string())?;
        match bytes {
            Some(bytes) => write_mapper_save(&sidecars.mapper_save_path, &bytes)
                .map_err(|error| error.to_string()),
            None => Ok(()),
        }
    }

    fn save_active_slot_or_new(&mut self) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        let slot_id = match self.active_slot_id {
            Some(slot_id) => slot_id,
            None => match allocate_next_slot_id(&sidecars.states_dir) {
                Ok(slot_id) => slot_id,
                Err(error) => {
                    log::warn!("allocating state slot failed: {error}");
                    return;
                }
            },
        };
        match self.console.export_state() {
            Ok(export) => {
                let preview = export.preview.as_ref().map(preview_to_thumbnail_source);
                if let Err(error) = write_state_slot(
                    &sidecars.states_dir,
                    slot_id,
                    &export.machine_state,
                    export.rom_identity,
                    export.options,
                    preview.as_ref(),
                ) {
                    log::warn!("saving state slot failed: {error}");
                } else {
                    self.active_slot_id = Some(slot_id);
                }
            }
            Err(error) => log::warn!("state export failed: {error}"),
        }
    }

    fn load_slot(&mut self, slot_id: u64) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        match load_state_slot(&state_slot_path(&sidecars.states_dir, slot_id)) {
            Ok(slot) => {
                if let Err(error) = self.console.import_state(slot.machine_state) {
                    log::warn!("state import failed: {error}");
                } else {
                    self.active_slot_id = Some(slot_id);
                    self.sync_paused_from_console();
                    self.needs_redraw = true;
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                }
            }
            Err(error) => log::warn!("loading state slot failed: {error}"),
        }
    }
}

fn preview_to_thumbnail_source(preview: &PreviewFrame) -> ThumbnailSource {
    ThumbnailSource {
        width: preview.width,
        height: preview.height,
        rgba: preview.rgba.clone(),
    }
}

impl ApplicationHandler for Window {
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
                if self.on_close() {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                self.on_resize(size);
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.on_keyboard_input(event),
            WindowEvent::RedrawRequested => self.on_update(),
            _ => (),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.maybe_refresh_window_title(now);

        let Some(window) = self.window.as_ref() else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };

        let metrics = self.console.metrics();
        if self.needs_redraw || metrics.frame_counter != self.last_presented_frame_counter {
            window.request_redraw();
        }

        if self.needs_redraw || (metrics.loaded && !metrics.paused) {
            event_loop.set_control_flow(ControlFlow::WaitUntil(now + FRAME_POLL_INTERVAL));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.on_close();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        let _ = self.on_close();
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.on_close();
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::window_title;
    use nerust_console::ConsoleMetrics;

    #[test]
    fn window_title_surfaces_runtime_metrics() {
        let title = window_title(
            false,
            ConsoleMetrics {
                loaded: true,
                emulation_fps: 59.9,
                speed_multiplier: 1.01,
                ..ConsoleMetrics::default()
            },
        );

        assert!(title.contains("FPS 59.9"));
        assert!(title.contains("Speed x1.01"));
    }

    #[test]
    fn window_title_marks_no_rom() {
        let title = window_title(true, ConsoleMetrics::default());

        assert!(title.contains("Paused"));
        assert!(title.contains("No ROM"));
    }
}
