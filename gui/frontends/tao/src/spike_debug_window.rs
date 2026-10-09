// SPIKE (iteration 8, DO NOT MERGE): integrated-window probe with PPU
// panes (pattern images + register panels), in-window memory edit flow
// (dump-row select -> value input -> confirm -> write), and the same
// toolbar/nav as iteration 7. No per-system branches. Deleted with the
// branch.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpikeDebugRequest {
    Refresh,
    Pause,
    Resume,
    TogglePause,
    StepFrame,
    StepInstr,
    MemNav,
    EditSelect(u32),
    WriteConfirm,
}

/// In-window edit state: dump-row selection, new-value input, and an
/// optional pending write awaiting Confirm. The old byte is read by
/// the host at selection time (confirm row shows it).
#[derive(Debug, Clone, Default)]
pub(crate) struct SpikeEditState {
    pub(crate) selected: Option<u32>,
    pub(crate) old: Option<u8>,
    pub(crate) input: String,
    pub(crate) pending: Option<(u32, Option<u8>, u64)>,
}

/// Shared bridge between the iced program and the host drain point.
pub(crate) struct SpikeDebugBridge {
    pub(crate) regs: Mutex<String>,
    pub(crate) dump_rows: Mutex<Vec<(u32, String)>>,
    pub(crate) disasm: Mutex<String>,
    pub(crate) panels: Mutex<String>,
    #[allow(clippy::type_complexity)]
    pub(crate) images: Mutex<Vec<(String, u32, u32, Vec<u8>)>>,
    pub(crate) spaces: Vec<(SpaceId, String, u32)>,
    pub(crate) space_idx: Mutex<usize>,
    pub(crate) mem_addr: Mutex<u32>,
    pub(crate) mem_input: Mutex<String>,
    pub(crate) dis_addr: Mutex<u32>,
    pub(crate) dis_input: Mutex<String>,
    pub(crate) follow_pc: AtomicBool,
    pub(crate) status: Mutex<String>,
    pub(crate) edit: Mutex<SpikeEditState>,
    pub(crate) outbox: Mutex<Vec<SpikeDebugRequest>>,
    pub(crate) view_invalidated: AtomicBool,
}

impl SpikeDebugBridge {
    pub(crate) fn new(
        regs: String,
        dump_rows: Vec<(u32, String)>,
        disasm: String,
        panels: String,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
    ) -> Self {
        Self {
            regs: Mutex::new(regs),
            dump_rows: Mutex::new(dump_rows),
            disasm: Mutex::new(disasm),
            panels: Mutex::new(panels),
            images: Mutex::new(images),
            spaces,
            space_idx: Mutex::new(0),
            mem_addr: Mutex::new(mem_addr),
            mem_input: Mutex::new(format!("{mem_addr:08X}")),
            dis_addr: Mutex::new(0),
            dis_input: Mutex::new(String::new()),
            follow_pc: AtomicBool::new(true),
            status: Mutex::new("paused".to_string()),
            edit: Mutex::new(SpikeEditState::default()),
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
        *self.mem_input.lock().unwrap() = format!("{start:08X}");
        // Selection belongs to the old space: clear it so the edit
        // section never confirms against a stale address.
        *self.edit.lock().unwrap() = SpikeEditState::default();
        self.push(SpikeDebugRequest::Refresh);
    }

    pub(crate) fn set_all(
        &self,
        regs: String,
        dump_rows: Vec<(u32, String)>,
        disasm: String,
        panels: String,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        status: String,
    ) {
        *self.regs.lock().unwrap() = regs;
        *self.dump_rows.lock().unwrap() = dump_rows;
        *self.disasm.lock().unwrap() = disasm;
        *self.panels.lock().unwrap() = panels;
        *self.images.lock().unwrap() = images;
        *self.status.lock().unwrap() = status;
        self.view_invalidated.store(true, Ordering::Release);
    }

    pub(crate) fn set_edit_selected(&self, addr: u32, old: Option<u8>, status: String) {
        let mut edit = self.edit.lock().unwrap();
        edit.selected = Some(addr);
        edit.old = old;
        edit.pending = None;
        *self.status.lock().unwrap() = status;
        self.view_invalidated.store(true, Ordering::Release);
    }

    pub(crate) fn take_pending_write(&self) -> Option<(u32, u64)> {
        let mut edit = self.edit.lock().unwrap();
        let (addr, _, value) = edit.pending.take()?;
        edit.input.clear();
        self.view_invalidated.store(true, Ordering::Release);
        Some((addr, value))
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
    RowSelected(u32),
    EditInputChanged(String),
    EditWrite,
    ConfirmWrite,
    CancelWrite,
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
                *bridge.mem_input.lock().unwrap() = format!("{next:08X}");
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
            SpikeDebugMessage::RowSelected(addr) => {
                bridge.push(SpikeDebugRequest::EditSelect(addr));
            }
            SpikeDebugMessage::EditInputChanged(text) => {
                bridge.edit.lock().unwrap().input = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            SpikeDebugMessage::EditWrite => {
                let (selected, old, input) = {
                    let edit = bridge.edit.lock().unwrap();
                    (edit.selected, edit.old, edit.input.clone())
                };
                match (selected, spike_parse_hex_addr(&input).filter(|v| *v <= 0xFF)) {
                    (Some(addr), Some(value)) => {
                        bridge.edit.lock().unwrap().pending = Some((addr, old, u64::from(value)));
                        bridge.view_invalidated.store(true, Ordering::Release);
                    }
                    _ => {
                        *bridge.status.lock().unwrap() = format!("parse failed: {input}");
                        bridge.view_invalidated.store(true, Ordering::Release);
                    }
                }
            }
            SpikeDebugMessage::ConfirmWrite => {
                bridge.push(SpikeDebugRequest::WriteConfirm);
            }
            SpikeDebugMessage::CancelWrite => {
                bridge.edit.lock().unwrap().pending = None;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{button, column, image, row, scrollable, text, text_input};
        let regs = state.bridge.regs.lock().unwrap().clone();
        let dump_rows = state.bridge.dump_rows.lock().unwrap().clone();
        let disasm = state.bridge.disasm.lock().unwrap().clone();
        let panels = state.bridge.panels.lock().unwrap().clone();
        let images = state.bridge.images.lock().unwrap().clone();
        let status = state.bridge.status.lock().unwrap().clone();
        let mem_input = state.bridge.mem_input.lock().unwrap().clone();
        let dis_input = state.bridge.dis_input.lock().unwrap().clone();
        let edit = state.bridge.edit.lock().unwrap().clone();
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
        // Dump rows are buttons OUTSIDE the scrollable (scrollable
        // children never yield messages under manual UI driving).
        // Row press selects the edit address; the host reads the old
        // byte for the confirm row.
        let mut dump_col = column![].spacing(2);
        for (addr, line) in &dump_rows {
            let marker = if edit.selected == Some(*addr) {
                "> "
            } else {
                "  "
            };
            dump_col = dump_col.push(
                button(text(format!("{marker}{line}")).size(14).font(iced::Font::MONOSPACE))
                    .on_press(SpikeDebugMessage::RowSelected(*addr)),
            );
        }
        let old_text = match (edit.selected, edit.old) {
            (Some(addr), Some(old)) => format!("Edit {addr:08X} (was {old:02X})"),
            (Some(addr), None) => format!("Edit {addr:08X} (was ??)"),
            (None, _) => "Edit: select a dump row".to_string(),
        };
        let edit_row = row![
            text(old_text).size(14),
            text_input("new hex byte", &edit.input)
                .on_input(SpikeDebugMessage::EditInputChanged)
                .on_submit(SpikeDebugMessage::EditWrite)
                .width(120),
            button(text("Write")).on_press(SpikeDebugMessage::EditWrite),
        ]
        .spacing(8);
        let mut content = column![
            text("SPIKE debugger probe (paused only)").size(18),
            toolbar,
            text("Registers").size(16),
            text(regs).size(14).font(iced::Font::MONOSPACE),
            text("Memory").size(16),
            space_row,
            mem_nav,
            dump_col,
            edit_row,
        ]
        .spacing(8)
        .padding(12)
        .width(iced::Length::Fill);
        if let Some((addr, old, value)) = edit.pending {
            let old_text = match old {
                Some(old) => format!("{old:02X}"),
                None => "??".to_string(),
            };
            content = content.push(
                row![
                    text(format!("write {value:02X} to {addr:08X} (was {old_text})?")).size(14),
                    button(text("Confirm")).on_press(SpikeDebugMessage::ConfirmWrite),
                    button(text("Cancel")).on_press(SpikeDebugMessage::CancelWrite),
                ]
                .spacing(8),
            );
        }
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
        let mut ppu_col = column![text("PPU").size(16)].spacing(8);
        if images.is_empty() {
            ppu_col = ppu_col.push(text("(no images)").size(14));
        }
        for (label, width, height, rgba) in &images {
            ppu_col = ppu_col.push(text(label.clone()).size(14));
            ppu_col = ppu_col.push(image(image::Handle::from_rgba(
                *width,
                *height,
                rgba.clone(),
            )));
        }
        ppu_col = ppu_col.push(text("Panels").size(16));
        ppu_col = ppu_col.push(text(panels).size(14).font(iced::Font::MONOSPACE));
        content = content.push(
            scrollable(
                column![
                    dis_nav,
                    text(disasm).size(14).font(iced::Font::MONOSPACE),
                    ppu_col,
                ]
                .spacing(8),
            )
            .height(iced::Length::Fill),
        );
        content = content.push(button("Refresh").on_press(SpikeDebugMessage::RefreshPressed));
        content.into()
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        regs: String,
        dump_rows: Vec<(u32, String)>,
        disasm: String,
        panels: String,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        let bridge = Arc::new(SpikeDebugBridge::new(
            regs, dump_rows, disasm, panels, images, spaces, mem_addr,
        ));
        let window = Arc::new(
            WindowBuilder::new()
                .with_title("Debugger (spike)")
                .with_inner_size(tao::dpi::LogicalSize::new(700.0, 1000.0))
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
        dump_rows: Vec<(u32, String)>,
        disasm: String,
        panels: String,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        status: String,
    ) {
        self.bridge
            .set_all(regs, dump_rows, disasm, panels, images, status);
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
