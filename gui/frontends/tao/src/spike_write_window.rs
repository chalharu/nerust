// SPIKE (iteration 4, DO NOT MERGE): WRAM single-address write UX probe.
//
// Two-step confirm (Write -> Confirm/Cancel), failure display, and the
// shared `spike_format_dump` text also rendered by the GTK spike window.
// Session access stays in the host: the iced program only exchanges
// `SpikeRequest`/`SpikeReply` through the bridge. Deleted with the branch.

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
use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window as TaoWindow, WindowBuilder},
};

use crate::{settings_window::convert_tao_window_event, tao_conversions::default_font};

/// Request from the spike program to the host (session lives in host).
#[derive(Debug, Clone)]
pub(crate) enum SpikeRequest {
    Refresh,
    Write { addr: u32, width: u8, value: u64 },
}

/// Reply from the host, produced by `SessionHandle::spike_*`.
#[derive(Debug, Clone)]
pub(crate) enum SpikeReply {
    Dump(String),
    WriteResult(String),
}

/// Shared bridge between the iced program and the host drain point.
pub(crate) struct SpikeBridge {
    pub(crate) dump: Mutex<String>,
    pub(crate) log: Mutex<Vec<String>>,
    pub(crate) outbox: Mutex<Vec<SpikeRequest>>,
    pub(crate) view_invalidated: AtomicBool,
}

impl SpikeBridge {
    pub(crate) fn new(dump: String) -> Self {
        Self {
            dump: Mutex::new(dump),
            log: Mutex::new(Vec::new()),
            outbox: Mutex::new(Vec::new()),
            view_invalidated: AtomicBool::new(false),
        }
    }

    pub(crate) fn push_reply(&self, reply: SpikeReply) {
        match reply {
            SpikeReply::Dump(text) => *self.dump.lock().unwrap() = text,
            SpikeReply::WriteResult(line) => self.log.lock().unwrap().push(line),
        }
        self.view_invalidated.store(true, Ordering::Release);
    }

    pub(crate) fn take_requests(&self) -> Vec<SpikeRequest> {
        std::mem::take(&mut *self.outbox.lock().unwrap())
    }
}

#[derive(Debug, Clone)]
pub(crate) enum SpikeMessage {
    AddrInput(String),
    ValueInput(String),
    WidthInput(String),
    WritePressed,
    ConfirmPressed,
    CancelPressed,
    RefreshPressed,
}

pub(crate) struct SpikeWriteState {
    bridge: Arc<SpikeBridge>,
    addr_text: String,
    value_text: String,
    width_text: String,
    pending: Option<(u32, u8, u64)>,
}

pub(crate) struct SpikeWriteProgram {
    pub(crate) bridge: Arc<SpikeBridge>,
}

impl Program for SpikeWriteProgram {
    type State = SpikeWriteState;
    type Message = SpikeMessage;
    type Theme = iced::Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_spike_write"
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
            SpikeWriteState {
                bridge: Arc::clone(&self.bridge),
                // 0x0010 sits in the second visible row: 16 rows from 0x0000
                // cover 0x0000-0x00FF, so the edited byte is visible.
                addr_text: "0010".to_string(),
                value_text: "AB".to_string(),
                width_text: "1".to_string(),
                pending: None,
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        match message {
            SpikeMessage::AddrInput(s) => state.addr_text = s,
            SpikeMessage::ValueInput(s) => state.value_text = s,
            SpikeMessage::WidthInput(s) => state.width_text = s,
            SpikeMessage::WritePressed => {
                if let Some((addr, width, value)) =
                    parse_write(&state.addr_text, &state.width_text, &state.value_text)
                {
                    state.pending = Some((addr, width, value));
                } else {
                    state
                        .bridge
                        .log
                        .lock()
                        .unwrap()
                        .push("parse failed: addr/width/value must be hex".to_string());
                }
            }
            SpikeMessage::ConfirmPressed => {
                if let Some((addr, width, value)) = state.pending.take() {
                    state
                        .bridge
                        .outbox
                        .lock()
                        .unwrap()
                        .push(SpikeRequest::Write { addr, width, value });
                }
            }
            SpikeMessage::CancelPressed => state.pending = None,
            SpikeMessage::RefreshPressed => {
                state
                    .bridge
                    .outbox
                    .lock()
                    .unwrap()
                    .push(SpikeRequest::Refresh);
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
        use iced::widget::{button, column, row, scrollable, text, text_input};
        let dump = state.bridge.dump.lock().unwrap().clone();
        let log = state.bridge.log.lock().unwrap().join("\n");
        let mut col = column![
            text("SPIKE write probe (WRAM, paused only)").size(18),
            scrollable(text(dump).size(14).font(iced::Font::MONOSPACE)).height(280),
            row![
                text("addr"),
                text_input("0100", &state.addr_text).on_input(SpikeMessage::AddrInput),
                text("value"),
                text_input("AB", &state.value_text).on_input(SpikeMessage::ValueInput),
                text("width"),
                text_input("1", &state.width_text).on_input(SpikeMessage::WidthInput),
            ]
            .spacing(8),
            row![
                button("Write").on_press(SpikeMessage::WritePressed),
                button("Refresh").on_press(SpikeMessage::RefreshPressed),
            ]
            .spacing(8),
        ]
        .spacing(8)
        .padding(12);
        if let Some((addr, width, value)) = state.pending {
            col = col.push(
                row![
                    text(format!(
                        "Write {value:02X} to WRAM:{addr:04X} width {width}?"
                    )),
                    button("Confirm").on_press(SpikeMessage::ConfirmPressed),
                    button("Cancel").on_press(SpikeMessage::CancelPressed),
                ]
                .spacing(8),
            );
        }
        col = col.push(text(log).size(14));
        col.into()
    }
}

fn parse_hex_u32(s: &str) -> Option<u32> {
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

fn parse_hex_u64(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

fn parse_write(addr: &str, width: &str, value: &str) -> Option<(u32, u8, u64)> {
    // Width passes through unchecked: `BadWidth` refusal is a failure-display
    // probe, so the core (not the client) must reject it.
    let addr = parse_hex_u32(addr)?;
    let width = u8::try_from(parse_hex_u64(width)?).ok()?;
    let value = parse_hex_u64(value)?;
    Some((addr, width, value))
}

/// Owns Instance + Cache + UI, mirroring `settings_window::UiState`.
pub(crate) struct SpikeUiState {
    ui: std::mem::ManuallyDrop<
        UserInterface<'static, SpikeMessage, iced::Theme, iced_tiny_skia::Renderer>,
    >,
    instance: program::Instance<SpikeWriteProgram>,
    bridge: Arc<SpikeBridge>,
}

impl SpikeUiState {
    fn build_ui(
        instance: &program::Instance<SpikeWriteProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        cache: Cache,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> UserInterface<'static, SpikeMessage, iced::Theme, iced_tiny_skia::Renderer> {
        unsafe {
            std::mem::transmute::<
                UserInterface<'_, SpikeMessage, iced::Theme, iced_tiny_skia::Renderer>,
                UserInterface<'static, SpikeMessage, iced::Theme, iced_tiny_skia::Renderer>,
            >(UserInterface::build(
                instance.view(window_id),
                bounds,
                cache,
                renderer,
            ))
        }
    }

    fn new(
        instance: program::Instance<SpikeWriteProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
        bridge: Arc<SpikeBridge>,
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
    ) -> &mut UserInterface<'static, SpikeMessage, iced::Theme, iced_tiny_skia::Renderer> {
        &mut self.ui
    }

    fn process_messages(
        &mut self,
        messages: Vec<SpikeMessage>,
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

    /// Rebuild after the host mutated the bridge (replies carry no message).
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

impl Drop for SpikeUiState {
    fn drop(&mut self) {
        unsafe { std::mem::ManuallyDrop::drop(&mut self.ui) };
    }
}

pub(crate) struct SpikeWriteWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    ui_state: SpikeUiState,
    renderer: SpikeRenderer,
    viewport_physical: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) modifiers: keyboard::Modifiers,
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) bridge: Arc<SpikeBridge>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
}

pub(crate) struct SpikeRenderer {
    compositor: Compositor,
    surface: Surface,
    backend: Renderer,
}

impl SpikeRenderer {
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

impl SpikeWriteWindowHandle {
    pub(crate) fn new(
        initial_dump: String,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        let bridge = Arc::new(SpikeBridge::new(initial_dump));
        let window = Arc::new(
            WindowBuilder::new()
                .with_title("Memory write (spike)")
                .with_inner_size(tao::dpi::LogicalSize::new(640.0, 560.0))
                .build(event_loop)
                .map_err(|e| {
                    log::error!("failed to create spike write window: {e}");
                })
                .ok()?,
        );
        let window_id = iced::window::Id::unique();
        let program = SpikeWriteProgram {
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
        let ui_state = SpikeUiState::new(
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
            renderer: SpikeRenderer {
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

    pub(crate) fn take_requests(&self) -> Vec<SpikeRequest> {
        self.bridge.take_requests()
    }

    pub(crate) fn push_reply(&mut self, reply: SpikeReply) {
        self.bridge.push_reply(reply);
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
            self.ui_state.process_messages(
                messages,
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
            log::warn!("spike write render present failed: {e:?}");
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
