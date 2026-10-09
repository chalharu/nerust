// SPIKE (iteration 6, DO NOT MERGE): debugger-window probe with three
// generic sections (Registers, Memory with space selector, PC-centered
// Disassembly). No per-system branches: spaces come from the static
// table, the PC from the register list. Deleted with the branch.

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
}

/// Shared bridge between the iced program and the host drain point.
pub(crate) struct SpikeDebugBridge {
    pub(crate) regs: Mutex<String>,
    pub(crate) dump: Mutex<String>,
    pub(crate) disasm: Mutex<String>,
    pub(crate) spaces: Vec<(SpaceId, String)>,
    pub(crate) space_idx: Mutex<usize>,
    pub(crate) outbox: Mutex<Vec<SpikeDebugRequest>>,
    pub(crate) view_invalidated: AtomicBool,
}

impl SpikeDebugBridge {
    pub(crate) fn new(
        regs: String,
        dump: String,
        disasm: String,
        spaces: Vec<(SpaceId, String)>,
    ) -> Self {
        Self {
            regs: Mutex::new(regs),
            dump: Mutex::new(dump),
            disasm: Mutex::new(disasm),
            spaces,
            space_idx: Mutex::new(0),
            outbox: Mutex::new(Vec::new()),
            view_invalidated: AtomicBool::new(false),
        }
    }

    pub(crate) fn selected_space(&self) -> Option<SpaceId> {
        let idx = *self.space_idx.lock().unwrap();
        self.spaces.get(idx).map(|(id, _)| *id)
    }

    pub(crate) fn set_all(&self, regs: String, dump: String, disasm: String) {
        *self.regs.lock().unwrap() = regs;
        *self.dump.lock().unwrap() = dump;
        *self.disasm.lock().unwrap() = disasm;
        self.view_invalidated.store(true, Ordering::Release);
    }

    pub(crate) fn take_requests(&self) -> Vec<SpikeDebugRequest> {
        std::mem::take(&mut *self.outbox.lock().unwrap())
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SpikeDebugMessage {
    RefreshPressed,
    SpaceSelected(usize),
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
        match message {
            SpikeDebugMessage::RefreshPressed => {
                state
                    .bridge
                    .outbox
                    .lock()
                    .unwrap()
                    .push(SpikeDebugRequest::Refresh);
            }
            SpikeDebugMessage::SpaceSelected(idx) => {
                if idx < state.bridge.spaces.len() {
                    *state.bridge.space_idx.lock().unwrap() = idx;
                    state
                        .bridge
                        .outbox
                        .lock()
                        .unwrap()
                        .push(SpikeDebugRequest::Refresh);
                }
            }
        }
        state.bridge.view_invalidated.store(true, Ordering::Release);
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{button, column, row, scrollable, text};
        let regs = state.bridge.regs.lock().unwrap().clone();
        let dump = state.bridge.dump.lock().unwrap().clone();
        let disasm = state.bridge.disasm.lock().unwrap().clone();
        let selected = *state.bridge.space_idx.lock().unwrap();
        let mut space_row = row![text("space")].spacing(8);
        for (idx, (_, name)) in state.bridge.spaces.iter().enumerate() {
            let label = if idx == selected {
                format!("[{name}]")
            } else {
                name.clone()
            };
            space_row =
                space_row.push(button(text(label)).on_press(SpikeDebugMessage::SpaceSelected(idx)));
        }
        column![
            text("SPIKE debugger probe (paused only)").size(18),
            text("Registers").size(16),
            text(regs).size(14).font(iced::Font::MONOSPACE),
            text("Memory").size(16),
            space_row,
            scrollable(
                column![
                    text(dump).size(14).font(iced::Font::MONOSPACE),
                    text("Disassembly (PC-centered)").size(16),
                    text(disasm).size(14).font(iced::Font::MONOSPACE),
                ]
                .spacing(8),
            )
            .height(iced::Length::Fill),
            button("Refresh").on_press(SpikeDebugMessage::RefreshPressed),
        ]
        .spacing(8)
        .padding(12)
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
        spaces: Vec<(SpaceId, String)>,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        let bridge = Arc::new(SpikeDebugBridge::new(regs, dump, disasm, spaces));
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

    pub(crate) fn set_all(&mut self, regs: String, dump: String, disasm: String) {
        self.bridge.set_all(regs, dump, disasm);
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
