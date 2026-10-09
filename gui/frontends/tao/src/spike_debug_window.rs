// SPIKE (iteration 7, DO NOT MERGE): debugger-window probe with execution
// toolbar, memory navigation, space dropdown, and shortcut keys, stacked on
// iteration 6. No per-system branches. Deleted with the branch.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use iced::{Size, advanced::renderer, keyboard, mouse, theme};
use iced_tiny_skia::{
    Renderer,
    graphics::compositor::Compositor as _,
    window::{Compositor, Surface, compositor},
};
use iced_winit::{
    Clipboard,
    graphics::Viewport,
    program::{self, Program},
    runtime::{
        Task,
        user_interface::{Cache, UserInterface},
    },
};
use nerust_core_traits::debugger::SpaceId;
use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window as TaoWindow, WindowBuilder},
};

use crate::{settings_window::convert_tao_window_event, tao_conversions::default_font};

/// Request from the spike program to the host.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SpikeDebugRequest {
    Refresh,
    Pause,
    Resume,
    TogglePause,
    StepFrame,
    StepInstr,
    MemNav,
}

/// Shared bridge between the iced program and the host drain point.
pub(crate) struct SpikeDebugBridge {
    pub(crate) regs: Mutex<String>,
    pub(crate) dump: Mutex<String>,
    pub(crate) disasm: Mutex<String>,
    pub(crate) spaces: Vec<(SpaceId, String, u32)>,
    pub(crate) space_idx: Mutex<usize>,
    pub(crate) mem_addr: Mutex<u32>,
    pub(crate) mem_input: Mutex<String>,
    pub(crate) dis_addr: Mutex<u32>,
    pub(crate) dis_input: Mutex<String>,
    pub(crate) follow_pc: AtomicBool,
    pub(crate) status: Mutex<String>,
    pub(crate) outbox: Mutex<Vec<SpikeDebugRequest>>,
    pub(crate) view_invalidated: AtomicBool,
}

impl SpikeDebugBridge {
    pub(crate) fn new(
        regs: String,
        dump: String,
        disasm: String,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
    ) -> Self {
        Self {
            regs: Mutex::new(regs),
            dump: Mutex::new(dump),
            disasm: Mutex::new(disasm),
            spaces,
            space_idx: Mutex::new(0),
            mem_addr: Mutex::new(mem_addr),
            mem_input: Mutex::new(format!("{mem_addr:04X}")),
            dis_addr: Mutex::new(0),
            dis_input: Mutex::new(String::new()),
            follow_pc: AtomicBool::new(true),
            status: Mutex::new("paused".to_string()),
            outbox: Mutex::new(Vec::new()),
            view_invalidated: AtomicBool::new(false),
        }
    }

    pub(crate) fn selected_space(&self) -> Option<SpaceId> {
        let idx = *self.space_idx.lock().unwrap();
        self.spaces.get(idx).map(|(id, _, _)| *id)
    }

    pub(crate) fn selected_name(&self) -> Option<String> {
        let idx = *self.space_idx.lock().unwrap();
        self.spaces.get(idx).map(|(_, name, _)| name.clone())
    }

    fn step_space(&self, delta: i32) {
        if self.spaces.is_empty() {
            return;
        }
        let len = self.spaces.len() as i32;
        let idx = *self.space_idx.lock().unwrap() as i32;
        let next = (idx + delta).rem_euclid(len) as usize;
        *self.space_idx.lock().unwrap() = next;
        let start = self.spaces[next].2;
        *self.mem_addr.lock().unwrap() = start;
        *self.mem_input.lock().unwrap() = format!("{start:04X}");
        self.push(SpikeDebugRequest::Refresh);
    }

    pub(crate) fn set_all(
        &self,
        regs: String,
        dump: String,
        disasm: String,
        status: String,
    ) {
        *self.regs.lock().unwrap() = regs;
        *self.dump.lock().unwrap() = dump;
        *self.disasm.lock().unwrap() = disasm;
        *self.status.lock().unwrap() = status;
        self.view_invalidated.store(true, Ordering::Release);
    }

    pub(crate) fn take_requests(&self) -> Vec<SpikeDebugRequest> {
        std::mem::take(&mut *self.outbox.lock().unwrap())
    }

    fn push(&self, request: SpikeDebugRequest) {
        self.outbox.lock().unwrap().push(request);
        self.view_invalidated.store(true, Ordering::Release);
    }
}

#[derive(Debug, Clone)]
pub(crate) enum SpikeDebugMessage {
    RefreshPressed,
    SpacePrev,
    SpaceNext,
    PausePressed,
    ResumePressed,
    StepFramePressed,
    StepInstrPressed,
    MemInputChanged(String),
    MemGo,
    MemPage(i32),
    DisInputChanged(String),
    DisGo,
    FollowPcToggle,
}

pub(crate) struct SpikeDebugState {
    bridge: Arc<SpikeDebugBridge>,
}

pub(crate) struct SpikeDebugProgram {
    pub(crate) bridge: Arc<SpikeDebugBridge>,
}

impl Program for SpikeDebugProgram {
    type State = SpikeDebugState;
    type Message = SpikeDebugMessage;
    type Theme = iced::Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_spike_debug"
    }

    fn settings(&self) -> iced::Settings {
        iced::Settings {
            default_font: default_font(),
            default_text_size: iced::Pixels(16.0),
            ..Default::default()
        }
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            SpikeDebugState {
                bridge: Arc::clone(&self.bridge),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        use nerust_gui_shell::session::spike_parse_hex_addr;
        let bridge = &state.bridge;
        match message {
            SpikeDebugMessage::RefreshPressed => bridge.push(SpikeDebugRequest::Refresh),
            SpikeDebugMessage::SpacePrev => bridge.step_space(-1),
            SpikeDebugMessage::SpaceNext => bridge.step_space(1),
            SpikeDebugMessage::PausePressed => bridge.push(SpikeDebugRequest::Pause),
            SpikeDebugMessage::ResumePressed => bridge.push(SpikeDebugRequest::Resume),
            SpikeDebugMessage::StepFramePressed => bridge.push(SpikeDebugRequest::StepFrame),
            SpikeDebugMessage::StepInstrPressed => bridge.push(SpikeDebugRequest::StepInstr),
            SpikeDebugMessage::MemInputChanged(text) => {
                *bridge.mem_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            SpikeDebugMessage::MemGo => {
                let input = bridge.mem_input.lock().unwrap().clone();
                match spike_parse_hex_addr(&input) {
                    Some(addr) => {
                        *bridge.mem_addr.lock().unwrap() = addr;
                        bridge.push(SpikeDebugRequest::MemNav);
                    }
                    None => {
                        *bridge.status.lock().unwrap() =
                            format!("parse failed: {input}");
                        bridge.view_invalidated.store(true, Ordering::Release);
                    }
                }
            }
            SpikeDebugMessage::MemPage(delta) => {
                let addr = *bridge.mem_addr.lock().unwrap();
                // Page = 12 rows of 16 bytes.
                let next = if delta < 0 {
                    addr.saturating_sub(192)
                } else {
                    addr.saturating_add(192)
                };
                *bridge.mem_addr.lock().unwrap() = next;
                *bridge.mem_input.lock().unwrap() = format!("{next:04X}");
                bridge.push(SpikeDebugRequest::MemNav);
            }
            SpikeDebugMessage::DisInputChanged(text) => {
                *bridge.dis_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            SpikeDebugMessage::DisGo => {
                let input = bridge.dis_input.lock().unwrap().clone();
                match spike_parse_hex_addr(&input) {
                    Some(addr) => {
                        *bridge.dis_addr.lock().unwrap() = addr;
                        bridge.follow_pc.store(false, Ordering::Release);
                        bridge.push(SpikeDebugRequest::Refresh);
                    }
                    None => {
                        *bridge.status.lock().unwrap() =
                            format!("parse failed: {input}");
                        bridge.view_invalidated.store(true, Ordering::Release);
                    }
                }
            }
            SpikeDebugMessage::FollowPcToggle => {
                bridge.follow_pc.store(true, Ordering::Release);
                bridge.push(SpikeDebugRequest::Refresh);
            }
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{button, column, row, scrollable, text, text_input};
        let regs = state.bridge.regs.lock().unwrap().clone();
        let dump = state.bridge.dump.lock().unwrap().clone();
        let disasm = state.bridge.disasm.lock().unwrap().clone();
        let status = state.bridge.status.lock().unwrap().clone();
        let mem_input = state.bridge.mem_input.lock().unwrap().clone();
        let dis_input = state.bridge.dis_input.lock().unwrap().clone();
        let follow_pc = state.bridge.follow_pc.load(Ordering::Acquire);
        let toolbar = row![
            button(text("Pause")).on_press(SpikeDebugMessage::PausePressed),
            button(text("Resume")).on_press(SpikeDebugMessage::ResumePressed),
            button(text("Step Frame")).on_press(SpikeDebugMessage::StepFramePressed),
            button(text("Step Instr")).on_press(SpikeDebugMessage::StepInstrPressed),
            text(status).size(14),
        ]
        .spacing(8);
        let space_row = row![
            text("space"),
            button(text("Prev")).on_press(SpikeDebugMessage::SpacePrev),
            text(state.bridge.selected_name().unwrap_or_default()).size(14),
            button(text("Next")).on_press(SpikeDebugMessage::SpaceNext),
        ]
        .spacing(8);
        let mem_nav = row![
            button(text("Prev")).on_press(SpikeDebugMessage::MemPage(-1)),
            button(text("Next")).on_press(SpikeDebugMessage::MemPage(1)),
            text_input("addr hex", &mem_input)
                .on_input(SpikeDebugMessage::MemInputChanged)
                .on_submit(SpikeDebugMessage::MemGo)
                .width(120),
            button(text("Go")).on_press(SpikeDebugMessage::MemGo),
        ]
        .spacing(8);
        let dis_nav = row![
            text(if follow_pc {
                "Disassembly (follow PC)"
            } else {
                "Disassembly (fixed)"
            }),
            button(text("Follow PC")).on_press(SpikeDebugMessage::FollowPcToggle),
            text_input("addr hex", &dis_input)
                .on_input(SpikeDebugMessage::DisInputChanged)
                .on_submit(SpikeDebugMessage::DisGo)
                .width(120),
            button(text("Go")).on_press(SpikeDebugMessage::DisGo),
        ]
        .spacing(8);
        column![
            text("SPIKE debugger probe (paused only)").size(18),
            toolbar,
            text("Registers").size(16),
            text(regs).size(14).font(iced::Font::MONOSPACE),
            text("Memory").size(16),
            space_row,
            mem_nav,
            scrollable(
                column![
                    text(dump).size(14).font(iced::Font::MONOSPACE),
                    dis_nav,
                    text(disasm).size(14).font(iced::Font::MONOSPACE),
                ]
                .spacing(8),
            )
            .height(iced::Length::Fill),
            button("Refresh").on_press(SpikeDebugMessage::RefreshPressed),
        ]
        .spacing(8)
        .padding(12)
        .width(iced::Length::Fill)
        .into()
    }
}

/// Owns Instance + Cache + UI, mirroring `settings_window::UiState`.
pub(crate) struct SpikeDebugUiState {
    ui: std::mem::ManuallyDrop<
        UserInterface<'static, SpikeDebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
    >,
    instance: program::Instance<SpikeDebugProgram>,
    bridge: Arc<SpikeDebugBridge>,
}

impl SpikeDebugUiState {
    fn build_ui(
        instance: &program::Instance<SpikeDebugProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        cache: Cache,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> UserInterface<'static, SpikeDebugMessage, iced::Theme, iced_tiny_skia::Renderer> {
        unsafe {
            std::mem::transmute::<
                UserInterface<'_, SpikeDebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
                UserInterface<'static, SpikeDebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
            >(UserInterface::build(
                instance.view(window_id),
                bounds,
                cache,
                renderer,
            ))
        }
    }

    fn new(
        instance: program::Instance<SpikeDebugProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
        bridge: Arc<SpikeDebugBridge>,
    ) -> Self {
        let ui = Self::build_ui(&instance, window_id, bounds, Cache::default(), renderer);
        Self {
            ui: std::mem::ManuallyDrop::new(ui),
            instance,
            bridge,
        }
    }

    fn ui_mut(
        &mut self,
    ) -> &mut UserInterface<'static, SpikeDebugMessage, iced::Theme, iced_tiny_skia::Renderer> {
        &mut self.ui
    }

    fn process_messages(
        &mut self,
        messages: Vec<SpikeDebugMessage>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
    ) {
        if messages.is_empty() && !self.bridge.view_invalidated.load(Ordering::Acquire) {
            return;
        }
        let placeholder = std::mem::replace(
            &mut *self.ui,
            Self::build_ui(
                &self.instance,
                window_id,
                bounds,
                Cache::default(),
                renderer,
            ),
        );
        let cache = placeholder.into_cache();
        for msg in messages {
            let _task = self.instance.update(msg);
        }
        let stale = std::mem::replace(
            &mut *self.ui,
            Self::build_ui(&self.instance, window_id, bounds, cache, renderer),
        );
        let _ = stale.into_cache();
        self.bridge.view_invalidated.store(false, Ordering::Release);
    }

    fn sync_from_bridge(
        &mut self,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
    ) {
        self.bridge.view_invalidated.store(true, Ordering::Release);
        self.process_messages(Vec::new(), window_id, bounds, renderer);
    }
}

impl Drop for SpikeDebugUiState {
    fn drop(&mut self) {
        unsafe { std::mem::ManuallyDrop::drop(&mut self.ui) };
    }
}

pub(crate) struct SpikeDebugWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    ui_state: SpikeDebugUiState,
    renderer: SpikeDebugRenderer,
    viewport_physical: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) modifiers: keyboard::Modifiers,
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) bridge: Arc<SpikeDebugBridge>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
}

pub(crate) struct SpikeDebugRenderer {
    compositor: Compositor,
    surface: Surface,
    backend: Renderer,
}

impl SpikeDebugRenderer {
    fn present(
        &mut self,
        viewport: &Viewport,
        background_color: iced::Color,
    ) -> Result<(), iced_tiny_skia::graphics::compositor::SurfaceError> {
        self.compositor.present(
            &mut self.backend,
            &mut self.surface,
            viewport,
            background_color,
            || {},
        )
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.compositor
            .configure_surface(&mut self.surface, width, height);
    }
}

impl SpikeDebugWindowHandle {
    pub(crate) fn new(
        regs: String,
        dump: String,
        disasm: String,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        let bridge = Arc::new(SpikeDebugBridge::new(
            regs, dump, disasm, spaces, mem_addr,
        ));
        let window = Arc::new(
            WindowBuilder::new()
                .with_title("Debugger (spike)")
                .with_inner_size(tao::dpi::LogicalSize::new(700.0, 700.0))
                .build(event_loop)
                .map_err(|e| {
                    log::error!("failed to create spike debugger window: {e}");
                })
                .ok()?,
        );
        let window_id = iced::window::Id::unique();
        let program = SpikeDebugProgram {
            bridge: Arc::clone(&bridge),
        };
        let (instance, _task) = program::Instance::new(program);
        let scale_factor = window.scale_factor() as f32;
        let window_size = window.inner_size();
        let viewport_physical = (window_size.width, window_size.height);
        let logical_size = window_size.to_logical::<f64>(scale_factor as f64);
        let bounds = Size::new(logical_size.width as f32, logical_size.height as f32);
        let mut compositor = compositor::new(
            iced_tiny_skia::Settings {
                default_font: default_font(),
                default_text_size: iced::Pixels(16.0),
            },
            Arc::clone(&window),
        );
        let mut renderer = compositor.create_renderer();
        let surface =
            compositor.create_surface(Arc::clone(&window), window_size.width, window_size.height);
        let ui_state = SpikeDebugUiState::new(
            instance,
            window_id,
            bounds,
            &mut renderer,
            Arc::clone(&bridge),
        );
        window.request_redraw();
        Some(Self {
            window,
            window_id,
            ui_state,
            renderer: SpikeDebugRenderer {
                compositor,
                surface,
                backend: renderer,
            },
            viewport_physical,
            scale_factor,
            modifiers: keyboard::Modifiers::default(),
            should_close,
            bridge,
            cursor: mouse::Cursor::default(),
            clipboard: Clipboard::unconnected(),
        })
    }

    pub(crate) fn take_requests(&self) -> Vec<SpikeDebugRequest> {
        self.bridge.take_requests()
    }

    pub(crate) fn set_all(
        &mut self,
        regs: String,
        dump: String,
        disasm: String,
        status: String,
    ) {
        self.bridge.set_all(regs, dump, disasm, status);
        let bounds = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        )
        .logical_size();
        self.ui_state
            .sync_from_bridge(self.window_id, bounds, &mut self.renderer.backend);
        self.window.request_redraw();
    }

    pub(crate) fn handle_event(&mut self, mapped: iced::Event) {
        // SPIKE shortcuts on physical codes (F6 step frame, F7 step
        // instruction, F9 pause/resume toggle). Physical codes avoid the
        // logical-key mapping gap for named keys.
        if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            physical_key,
            repeat: false,
            ..
        }) = &mapped
        {
            use iced::keyboard::key::{Code, Physical};
            // NOTE: `physical_key` binds by reference through `&mapped`;
            // dereference before matching or no arm ever hits.
            let request = match *physical_key {
                Physical::Code(Code::F6) => Some(SpikeDebugRequest::StepFrame),
                Physical::Code(Code::F7) => Some(SpikeDebugRequest::StepInstr),
                Physical::Code(Code::F9) => Some(SpikeDebugRequest::TogglePause),
                _ => None,
            };
            if let Some(request) = request {
                self.bridge.push(request);
            }
        }
        let mut messages = Vec::new();
        self.ui_state.ui_mut().update(
            &[mapped],
            self.cursor,
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut messages,
        );
        if !messages.is_empty() {
            let bounds = Viewport::with_physical_size(
                Size::new(self.viewport_physical.0, self.viewport_physical.1),
                self.scale_factor,
            )
            .logical_size();
            self.ui_state.process_messages(                messages,
                self.window_id,
                bounds,
                &mut self.renderer.backend,
            );
            self.window.request_redraw();
        }
    }

    pub(crate) fn render(&mut self) {
        let theme = iced::Theme::Dark;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let vp = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        );
        let redraw_event = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let _ = self.ui_state.ui_mut().update(
            &[redraw_event],
            self.cursor,
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut std::vec::Vec::new(),
        );
        self.ui_state.ui_mut().draw(
            &mut self.renderer.backend,
            &theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            self.cursor,
        );
        if let Err(e) = self.renderer.present(&vp, iced::Color::BLACK) {
            log::warn!("spike debugger render present failed: {e:?}");
        }
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport_physical = (width, height);
        self.renderer.resize(width, height);
    }

    pub(crate) fn set_scale_factor(&mut self, sf: f32) {
        self.scale_factor = sf;
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: tao::keyboard::ModifiersState) {
        self.modifiers = crate::tao_conversions::tao_modifiers_to_iced(modifiers);
    }

    pub(crate) fn handle_tao_event(&mut self, event: tao::event::WindowEvent) {
        if let Some(iced_event) = convert_tao_window_event(
            event,
            &mut self.cursor,
            self.scale_factor,
            &mut self.modifiers,
            &self.should_close,
        ) {
            self.handle_event(iced_event);
        }
    }
}
