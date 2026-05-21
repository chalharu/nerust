// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::app_menu::{AppMenu, MenuCommand, UserEvent};
use crate::surface::SurfaceTarget;
use nerust_console::{Console, ConsoleMetrics, PreviewFrame};
use nerust_core::CoreOptions;
use nerust_core::controller::standard_controller::Buttons;
use nerust_persistence::{
    LoadedStateSlot, SidecarPaths, StateSlotSummary, ThumbnailSource, allocate_next_slot_id,
    delete_state_slot, load_mapper_save, load_state_slot, resolve_sidecars,
    scan_state_slots_for_target, write_mapper_save, write_recovery_mapper_save, write_state_slot,
};
use nerust_screen_filter::FilterType;
use nerust_screen_traits::{LogicalSize, PhysicalSize};
use nerust_screen_wgpu::{RenderOutcome, Renderer};
use nerust_sound_openal::OpenAl;
use nerust_timer::CLOCK_RATE;
use nerust_wgpuwrap::{RenderSurface, SurfaceSize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
#[cfg(target_os = "macos")]
use tao::platform::macos::EventLoopExtMacOS;
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::{ElementState, Event, KeyEvent, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopWindowTarget},
    keyboard::KeyCode,
    window::{Window as TaoWindow, WindowBuilder},
};

const TITLE_UPDATE_INTERVAL: Duration = Duration::from_millis(500);
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(1);

fn window_title(paused: bool, _render_fps: f32, console_metrics: ConsoleMetrics) -> String {
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

fn preview_to_thumbnail_source(preview: &PreviewFrame) -> ThumbnailSource {
    ThumbnailSource {
        width: preview.width,
        height: preview.height,
        rgba: preview.rgba.clone(),
    }
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

fn create_window_builder(size: PhysicalSize, title: String) -> WindowBuilder {
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(TaoLogicalSize::new(
            f64::from(size.width),
            f64::from(size.height),
        ))
}

fn window_surface_size(size: TaoPhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize::new(size.width, size.height)
}

pub(crate) struct WindowRuntime {
    event_loop: Option<EventLoop<UserEvent>>,
    window: Option<Arc<TaoWindow>>,
    render_surface: Option<RenderSurface<SurfaceTarget>>,
    renderer: Option<Renderer>,
    last_render_error: Option<String>,
    keys: Buttons,
    paused: bool,
    console: Console,
    app_menu: AppMenu,
    physical_size: PhysicalSize,
    render_frame_count: u32,
    render_fps: f32,
    render_started_at: Instant,
    last_title_update: Instant,
    last_presented_frame_counter: u64,
    needs_redraw: bool,
    rom_path: Option<PathBuf>,
    sidecars: Option<SidecarPaths>,
    mapper_save_flush_allowed: bool,
    mapper_save_recovery_written: bool,
    state_slots: Vec<StateSlotSummary>,
    active_slot_id: Option<u64>,
}

impl WindowRuntime {
    pub(crate) fn new() -> Self {
        let filter_type = FilterType::NtscComposite;
        let source_logical_size = LogicalSize {
            width: 256,
            height: 240,
        };
        let speaker = OpenAl::new(48_000, CLOCK_RATE as i32, 128, 20);
        let console = Console::new_gpu(speaker, filter_type, source_logical_size);
        let physical_size = console.video().presentation().physical_size();

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
            render_surface: None,
            renderer: None,
            last_render_error: None,
            keys: Buttons::empty(),
            paused: false,
            console,
            app_menu,
            physical_size,
            render_frame_count: 0,
            render_fps: 0.0,
            render_started_at: Instant::now(),
            last_title_update: Instant::now(),
            last_presented_frame_counter: 0,
            needs_redraw: true,
            rom_path: None,
            sidecars: None,
            mapper_save_flush_allowed: true,
            mapper_save_recovery_written: false,
            state_slots: Vec::new(),
            active_slot_id: None,
        }
    }

    pub(crate) fn load(&mut self, data: Vec<u8>) {
        self.load_with_options(None, data, CoreOptions::default());
    }

    pub(crate) fn load_with_options(
        &mut self,
        rom_path: Option<PathBuf>,
        data: Vec<u8>,
        options: CoreOptions,
    ) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before load failed: {error}");
            return;
        }
        if self.console.load_with_options(data, options).is_ok() {
            self.rom_path = rom_path;
            self.sidecars = self.rom_path.as_deref().map(resolve_sidecars);
            self.mapper_save_flush_allowed = true;
            self.mapper_save_recovery_written = false;
            self.active_slot_id = None;
            self.refresh_state_slots();
            if let Err(error) = self.load_mapper_save_if_available() {
                self.mapper_save_flush_allowed = false;
                log::warn!("mapper save auto-load failed: {error}");
            }
        }
    }

    pub(crate) fn run(mut self) {
        self.console.resume();
        let event_loop = self.event_loop.take().unwrap();

        event_loop.run(move |event, event_loop, control_flow| match event {
            Event::NewEvents(StartCause::Init) => {
                self.ensure_window(event_loop);
                *control_flow = ControlFlow::Wait;
            }
            Event::WindowEvent {
                event, window_id, ..
            } if self
                .window
                .as_ref()
                .is_some_and(|window| window_id == window.id()) =>
            {
                match event {
                    WindowEvent::CloseRequested => {
                        if self.prepare_close() {
                            *control_flow = ControlFlow::Exit;
                        }
                    }
                    WindowEvent::Focused(false) => self.clear_keys(),
                    WindowEvent::Resized(_) => {
                        self.reconfigure_surface();
                        self.needs_redraw = true;
                        if let Some(window) = self.window.as_ref() {
                            window.request_redraw();
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
                let now = Instant::now();
                self.maybe_refresh_window_title(now);

                let Some(window) = self.window.as_ref() else {
                    *control_flow = ControlFlow::Wait;
                    return;
                };

                let metrics = self.console.metrics();
                if self.needs_redraw || metrics.frame_counter != self.last_presented_frame_counter {
                    window.request_redraw();
                }

                if self.needs_redraw || (metrics.loaded && !metrics.paused) {
                    *control_flow = ControlFlow::WaitUntil(now + FRAME_POLL_INTERVAL);
                } else {
                    *control_flow = ControlFlow::Wait;
                }
            }
            Event::UserEvent(UserEvent::Menu(command)) => {
                self.on_menu_command(control_flow, command);
            }
            Event::LoopDestroyed => {
                self.app_menu.clear_event_handler();
            }
            _ => (),
        });
    }

    fn ensure_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.window.is_some() {
            return;
        }

        let window = Arc::new(
            create_window_builder(self.physical_size, self.current_window_title())
                .build(event_loop)
                .unwrap(),
        );
        let surface_target = SurfaceTarget::new(window.clone(), self.physical_size);
        self.app_menu.init_for_window(&window);
        self.app_menu.update(
            self.console.metrics().loaded,
            self.paused,
            &self.state_slots,
            self.active_slot_id,
        );
        let render_surface = RenderSurface::new(surface_target).unwrap();
        let renderer = pollster::block_on(Renderer::new(
            &render_surface,
            render_surface.surface_size(window_surface_size(window.inner_size())),
            self.console.video().presentation(),
        ))
        .unwrap();
        self.window = Some(window);
        self.render_surface = Some(render_surface);
        self.renderer = Some(renderer);
        self.needs_redraw = true;
        self.refresh_window_title();
    }

    fn current_window_title(&self) -> String {
        window_title(self.paused, self.render_fps, self.console.metrics())
    }

    fn refresh_window_title(&mut self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&self.current_window_title());
        }
    }

    fn maybe_refresh_window_title(&mut self, now: Instant) {
        if now.duration_since(self.last_title_update) >= TITLE_UPDATE_INTERVAL {
            self.last_title_update = now;
            self.refresh_window_title();
        }
    }

    fn record_render_frame(&mut self) {
        self.render_frame_count += 1;
        let now = Instant::now();
        let elapsed = now.duration_since(self.render_started_at);
        if elapsed >= TITLE_UPDATE_INTERVAL {
            self.render_fps = self.render_frame_count as f32 / elapsed.as_secs_f32();
            self.render_frame_count = 0;
            self.render_started_at = now;
        }
        self.maybe_refresh_window_title(now);
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
        self.app_menu.update(
            self.console.metrics().loaded,
            self.paused,
            &self.state_slots,
            self.active_slot_id,
        );
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
        self.app_menu.update(
            self.console.metrics().loaded,
            self.paused,
            &self.state_slots,
            self.active_slot_id,
        );
        self.refresh_window_title();
    }

    fn refresh_state_slots(&mut self) {
        self.state_slots = if let Some(sidecars) = self.sidecars.as_ref() {
            match self.console.persistence_target() {
                Ok((rom_identity, options)) => {
                    match scan_state_slots_for_target(&sidecars.states_dir, rom_identity, options) {
                        Ok(slots) => slots,
                        Err(error) => {
                            log::warn!("state slot refresh failed: {error}");
                            Vec::new()
                        }
                    }
                }
                Err(error) => {
                    log::warn!("state slot target unavailable: {error}");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if self
            .active_slot_id
            .is_some_and(|slot_id| !self.state_slots.iter().any(|slot| slot.slot_id == slot_id))
        {
            self.active_slot_id = None;
        }
        self.app_menu.update(
            self.console.metrics().loaded,
            self.paused,
            &self.state_slots,
            self.active_slot_id,
        );
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
        let slot_id = if let Some(slot_id) = self.active_slot_id {
            slot_id
        } else {
            match self
                .sidecars
                .as_ref()
                .map(|sidecars| allocate_next_slot_id(&sidecars.states_dir))
                .transpose()
            {
                Ok(Some(slot_id)) => slot_id,
                Ok(None) => return,
                Err(error) => {
                    log::warn!("allocating state slot failed: {error}");
                    return;
                }
            }
        };
        self.save_slot(slot_id, true);
    }

    fn create_slot(&mut self) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        match allocate_next_slot_id(&sidecars.states_dir) {
            Ok(slot_id) => self.save_slot(slot_id, true),
            Err(error) => log::warn!("creating state slot failed: {error}"),
        }
    }

    fn save_slot(&mut self, slot_id: u64, make_active: bool) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        match self.console.export_state() {
            Ok(export) => {
                let preview = export.preview.as_ref().map(preview_to_thumbnail_source);
                match write_state_slot(
                    &sidecars.states_dir,
                    slot_id,
                    &export.machine_state,
                    export.rom_identity,
                    export.options,
                    preview.as_ref(),
                ) {
                    Ok(_) => {
                        if make_active {
                            self.active_slot_id = Some(slot_id);
                        }
                        self.refresh_state_slots();
                    }
                    Err(error) => log::warn!("saving state slot failed: {error}"),
                }
            }
            Err(error) => log::warn!("state export failed: {error}"),
        }
    }

    fn load_slot(&mut self, slot_id: u64) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        let path = nerust_persistence::state_slot_path(&sidecars.states_dir, slot_id);
        match load_state_slot(&path) {
            Ok(LoadedStateSlot { machine_state, .. }) => {
                if let Err(error) = self.console.import_state(machine_state) {
                    log::warn!("state import failed: {error}");
                } else {
                    self.active_slot_id = Some(slot_id);
                    self.sync_paused_from_console();
                    self.needs_redraw = true;
                    if let Some(window) = self.window.as_ref() {
                        window.request_redraw();
                    }
                    self.refresh_state_slots();
                }
            }
            Err(error) => log::warn!("loading state slot failed: {error}"),
        }
    }

    fn delete_slot(&mut self, slot_id: u64) {
        let Some(sidecars) = self.sidecars.as_ref() else {
            return;
        };
        let path = nerust_persistence::state_slot_path(&sidecars.states_dir, slot_id);
        match delete_state_slot(&path) {
            Ok(()) => {
                if self.active_slot_id == Some(slot_id) {
                    self.active_slot_id = None;
                }
                self.refresh_state_slots();
            }
            Err(error) => log::warn!("deleting state slot failed: {error}"),
        }
    }

    fn on_menu_command(&mut self, control_flow: &mut ControlFlow, command: MenuCommand) {
        match command {
            MenuCommand::Pause => self.set_paused(true),
            MenuCommand::Resume => self.set_paused(false),
            MenuCommand::Reset => {
                let _ = self
                    .console
                    .reset()
                    .map_err(|error| log::warn!("reset failed: {error}"));
            }
            MenuCommand::Quit => {
                if self.prepare_close() {
                    *control_flow = ControlFlow::Exit;
                }
            }
            MenuCommand::CreateSlot => self.create_slot(),
            MenuCommand::SaveActiveSlot => self.save_active_slot_or_new(),
            MenuCommand::LoadActiveSlot => {
                if let Some(slot_id) = self.active_slot_id {
                    self.load_slot(slot_id);
                }
            }
            MenuCommand::SelectActiveSlot(slot_id) => {
                self.active_slot_id = Some(slot_id);
                self.refresh_state_slots();
            }
            MenuCommand::SaveSlot(slot_id) => self.save_slot(slot_id, false),
            MenuCommand::LoadSlot(slot_id) => self.load_slot(slot_id),
            MenuCommand::DeleteSlot(slot_id) => self.delete_slot(slot_id),
        }
    }

    fn reconfigure_surface(&mut self) {
        let Some(window_size) = self
            .window
            .as_ref()
            .map(|window| window_surface_size(window.inner_size()))
        else {
            return;
        };
        let (Some(renderer), Some(render_surface)) =
            (self.renderer.as_mut(), self.render_surface.as_mut())
        else {
            return;
        };
        renderer.reconfigure_surface(render_surface, render_surface.surface_size(window_size));
    }

    fn on_update(&mut self) {
        let Some(window_size) = self
            .window
            .as_ref()
            .map(|window| window_surface_size(window.inner_size()))
        else {
            return;
        };
        let (Some(renderer), Some(render_surface)) =
            (self.renderer.as_mut(), self.render_surface.as_mut())
        else {
            return;
        };
        let surface_size = render_surface.surface_size(window_size);
        let render_result = self
            .console
            .video()
            .frame_buffer()
            .with_bytes(|frame_buffer| renderer.render(render_surface, surface_size, frame_buffer));

        match render_result {
            Ok(RenderOutcome::Presented) => {
                self.last_render_error = None;
                self.last_presented_frame_counter = self.console.metrics().frame_counter;
                self.needs_redraw = false;
                self.record_render_frame();
            }
            Ok(RenderOutcome::Skipped) => {
                self.needs_redraw = true;
            }
            Ok(RenderOutcome::RecreateSurface) => {
                self.needs_redraw = true;
                match render_surface.recreate_surface() {
                    Ok(()) => {
                        self.last_render_error = None;
                        renderer.reconfigure_surface(
                            render_surface,
                            render_surface.surface_size(window_size),
                        );
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
            Err(err) => {
                let should_log = self.last_render_error.as_deref() != Some(err.as_str());
                self.last_render_error = Some(err.clone());
                self.needs_redraw = true;
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
                let _ = self.console.reset();
                Buttons::empty()
            }
            KeyCode::F5 if input.state == ElementState::Released && !input.repeat => {
                self.save_active_slot_or_new();
                Buttons::empty()
            }
            KeyCode::F8 if input.state == ElementState::Released && !input.repeat => {
                if let Some(slot_id) = self.active_slot_id {
                    self.load_slot(slot_id);
                }
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

    fn prepare_close(&mut self) -> bool {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush before close failed: {error}");
        }
        true
    }
}

impl Drop for WindowRuntime {
    fn drop(&mut self) {
        if let Err(error) = self.flush_mapper_save() {
            log::warn!("mapper save flush during shutdown failed: {error}");
        }
        self.renderer = None;
        self.render_surface = None;
        self.window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{keycode_button, window_title};
    use nerust_console::ConsoleMetrics;
    use nerust_core::controller::standard_controller::Buttons;
    use tao::keyboard::KeyCode;

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

    #[test]
    fn window_title_surfaces_runtime_metrics() {
        let title = window_title(
            false,
            59.9,
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
}
