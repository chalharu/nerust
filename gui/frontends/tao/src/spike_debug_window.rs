// SPIKE (iteration 10, DO NOT MERGE): cross-emulator survey probe.
// Two-pane layout (disassembly left, state right), PC-row highlight,
// line-click navigation with follow-target (no$ style) and Back undo,
// changed-row `*` marks (Mesen style), bookmarks, watch/freeze
// (FCEUX/BGB style). PPU lives in `spike_ppu_window`. No per-system
// branches. Deleted with the branch.

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
use nerust_core_traits::debugger::{DisasmLine, SpaceId};
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
    // SPIKE (iteration 11): stage a parsed value into the
    // shell-owned transaction; the drain decides keep-or-clear.
    EditStage(u64),
    EditCancel,
    WriteConfirm,
}

/// In-window edit input: dump-row selection, old-byte display, and
/// the new-value input string. The pending write itself lives in
/// the shell transaction (iteration 11 split).
#[derive(Debug, Clone, Default)]
pub(crate) struct SpikeEditState {
    pub(crate) selected: Option<u32>,
    pub(crate) old: Option<u8>,
    pub(crate) input: String,
}

/// SPIKE (iteration 11): navigation state, split out of the bridge
/// god-struct (implementation review §14). Movement only; display
/// cache and user lists stay on the bridge. Deleted with the spike.
pub(crate) struct SpikeNavState {
    pub(crate) spaces: Vec<(SpaceId, String, u32)>,
    pub(crate) space_idx: Mutex<usize>,
    pub(crate) mem_addr: Mutex<u32>,
    pub(crate) mem_input: Mutex<String>,
    pub(crate) dis_addr: Mutex<u32>,
    pub(crate) dis_input: Mutex<String>,
    /// Disassembly navigation undo stack (no$ Back), newest last.
    pub(crate) dis_back: Mutex<Vec<u32>>,
    pub(crate) follow_pc: AtomicBool,
}

/// Shared bridge between the iced program and the host drain point.
/// Display cache + user lists; navigation lives in `nav`, the write
/// transaction in the shell (iteration 11 split).
pub(crate) struct SpikeDebugBridge {
    pub(crate) regs: Mutex<String>,
    pub(crate) dump_rows: Mutex<Vec<(u32, String)>>,
    /// Changed dump-row addresses since the previous re-read (Mesen
    /// access marks, row-level; per-byte is production work).
    pub(crate) diff: Mutex<Vec<u32>>,
    pub(crate) disasm_lines: Mutex<Vec<DisasmLine>>,
    pub(crate) panels: Mutex<String>,
    #[allow(clippy::type_complexity)]
    pub(crate) images: Mutex<Vec<(String, u32, u32, Vec<u8>)>>,
    /// In-window bookmarks (FCEUX style); window-local, no file yet.
    pub(crate) bookmarks: Mutex<Vec<(u32, String)>>,
    pub(crate) bm_input: Mutex<String>,
    /// Read-only live watch (BGB style); value fills on each re-read.
    pub(crate) watch: Mutex<Option<u32>>,
    pub(crate) watch_value: Mutex<Option<u8>>,
    /// Pinned value re-applied by the host on every drain (FCEUX
    /// freeze style). Silent: the rows are the evidence.
    pub(crate) freeze: Mutex<Option<(u32, u8)>>,
    /// PPU-window hover line, set by the PPU program.
    pub(crate) ppu_hover: Mutex<String>,
    pub(crate) status: Mutex<String>,
    /// Confirm-row text from the shell transaction, if fully staged.
    pub(crate) pending_text: Mutex<Option<String>>,
    pub(crate) edit: Mutex<SpikeEditState>,
    pub(crate) nav: SpikeNavState,
    pub(crate) outbox: Mutex<Vec<SpikeDebugRequest>>,
    pub(crate) view_invalidated: AtomicBool,
    /// PPU window rebuild flag (the main window clears only its own).
    pub(crate) ppu_invalidated: AtomicBool,
}

impl SpikeDebugBridge {
    pub(crate) fn new(
        regs: String,
        dump_rows: Vec<(u32, String)>,
        disasm_lines: Vec<DisasmLine>,
        panels: String,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
    ) -> Self {
        Self {
            regs: Mutex::new(regs),
            dump_rows: Mutex::new(dump_rows),
            diff: Mutex::new(Vec::new()),
            disasm_lines: Mutex::new(disasm_lines),
            panels: Mutex::new(panels),
            images: Mutex::new(images),
            bookmarks: Mutex::new(Vec::new()),
            bm_input: Mutex::new(String::new()),
            watch: Mutex::new(None),
            watch_value: Mutex::new(None),
            freeze: Mutex::new(None),
            ppu_hover: Mutex::new(String::new()),
            status: Mutex::new("paused".to_string()),
            pending_text: Mutex::new(None),
            edit: Mutex::new(SpikeEditState::default()),
            nav: SpikeNavState {
                spaces,
                space_idx: Mutex::new(0),
                mem_addr: Mutex::new(mem_addr),
                mem_input: Mutex::new(format!("{mem_addr:08X}")),
                dis_addr: Mutex::new(0),
                dis_input: Mutex::new(String::new()),
                dis_back: Mutex::new(Vec::new()),
                follow_pc: AtomicBool::new(true),
            },
            outbox: Mutex::new(Vec::new()),
            view_invalidated: AtomicBool::new(false),
            ppu_invalidated: AtomicBool::new(false),
        }
    }

    pub(crate) fn selected_space(&self) -> Option<SpaceId> {
        let idx = *self.nav.space_idx.lock().unwrap();
        self.nav.spaces.get(idx).map(|(id, _, _)| *id)
    }

    pub(crate) fn selected_name(&self) -> Option<String> {
        let idx = *self.nav.space_idx.lock().unwrap();
        self.nav.spaces.get(idx).map(|(_, name, _)| name.clone())
    }

    pub(crate) fn space_names(&self) -> Vec<String> {
        self.nav
            .spaces
            .iter()
            .map(|(_, name, _)| name.clone())
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_all(
        &self,
        regs: String,
        dump_rows: Vec<(u32, String)>,
        disasm_lines: Vec<DisasmLine>,
        panels: String,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        status: String,
        watch_value: Option<u8>,
        diff: Vec<u32>,
    ) {
        *self.regs.lock().unwrap() = regs;
        *self.dump_rows.lock().unwrap() = dump_rows;
        *self.disasm_lines.lock().unwrap() = disasm_lines;
        *self.panels.lock().unwrap() = panels;
        *self.images.lock().unwrap() = images;
        *self.status.lock().unwrap() = status;
        *self.watch_value.lock().unwrap() = watch_value;
        *self.diff.lock().unwrap() = diff;
        // SPIKE (iteration 11): no pending kill here. The shell
        // transaction dies on re-read in the host drain; the bridge
        // only caches the confirm-row text below.
        self.view_invalidated.store(true, Ordering::Release);
        self.ppu_invalidated.store(true, Ordering::Release);
    }

    /// Program-side status line (no host roundtrip needed).
    pub(crate) fn set_status(&self, status: String) {
        *self.status.lock().unwrap() = status;
        self.view_invalidated.store(true, Ordering::Release);
    }

    /// Push the current disassembly anchor for Back undo (cap 32,
    /// no consecutive duplicates), then pin `next`.
    pub(crate) fn navigate_dis(&self, next: u32) {
        // SPIKE (iteration 10): always push so the first hop out of
        // follow-PC mode is undoable too. In follow mode `dis_addr`
        // is stale (0); the address the user was looking at is the
        // displayed anchor (first disasm line, i.e. the PC). Back
        // returns there as a fixed address; follow-PC itself is
        // re-enabled via the Follow PC button.
        let follow = self.nav.follow_pc.load(Ordering::Acquire);
        let current = if follow {
            self.disasm_lines
                .lock()
                .unwrap()
                .first()
                .map(|line| line.addr)
                .unwrap_or(*self.nav.dis_addr.lock().unwrap())
        } else {
            *self.nav.dis_addr.lock().unwrap()
        };
        let mut back = self.nav.dis_back.lock().unwrap();
        if back.last() != Some(&current) {
            back.push(current);
            if back.len() > 32 {
                back.remove(0);
            }
        }
        *self.nav.dis_addr.lock().unwrap() = next;
        *self.nav.dis_input.lock().unwrap() = format!("{next:08X}");
        self.nav.follow_pc.store(false, Ordering::Release);
        self.push(SpikeDebugRequest::Refresh);
    }

    pub(crate) fn set_edit_selected(&self, addr: u32, old: Option<u8>, status: String) {
        let mut edit = self.edit.lock().unwrap();
        edit.selected = Some(addr);
        edit.old = old;
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
    SpacePicked(String),
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
    DisLineClicked(u32),
    FollowTarget(u32),
    DisBack,
    BookmarkInputChanged(String),
    BookmarkAdd,
    BookmarkJump(u32),
    // SPIKE (iteration 11): per-row removal (list caps at 8).
    BookmarkDelete(u32),
    WatchSet,
    WatchClear,
    FreezeSet,
    FreezeClear,
    RowSelected(u32),
    EditInputChanged(String),
    EditWrite,
    ConfirmWrite,
    CancelWrite,
}

pub(crate) struct SpikeDebugState {
    bridge: Arc<SpikeDebugBridge>,
}

/// SPIKE (iteration 11): one rebuild's worth of display data.
/// Deleted with the spike branch.
#[derive(Debug, Clone, Default)]
pub(crate) struct SpikeViewData {
    pub(crate) regs: String,
    pub(crate) dump_rows: Vec<(u32, String)>,
    pub(crate) diff: Vec<u32>,
    pub(crate) disasm_lines: Vec<DisasmLine>,
    pub(crate) status: String,
    pub(crate) mem_input: String,
    pub(crate) dis_input: String,
    pub(crate) edit: SpikeEditState,
    pub(crate) follow_pc: bool,
    pub(crate) back_len: usize,
    pub(crate) bookmarks: Vec<(u32, String)>,
    pub(crate) bm_input: String,
    pub(crate) watch: Option<u32>,
    pub(crate) watch_value: Option<u8>,
    pub(crate) freeze: Option<(u32, u8)>,
    pub(crate) pending_text: Option<String>,
    pub(crate) space_names: Vec<String>,
    pub(crate) selected_name: Option<String>,
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

    // SPIKE (iteration 9): Light instance theme so widget catalogs
    // resolve light styles. `render` draws with the same theme.
    fn theme(&self, _state: &Self::State, _window: iced::window::Id) -> Option<Self::Theme> {
        Some(iced::Theme::Light)
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
            SpikeDebugMessage::SpacePicked(name) => {
                let idx = bridge.nav.spaces.iter().position(|(_, n, _)| *n == name);
                if let Some(idx) = idx {
                    *bridge.nav.space_idx.lock().unwrap() = idx;
                    let start = bridge.nav.spaces[idx].2;
                    *bridge.nav.mem_addr.lock().unwrap() = start;
                    *bridge.nav.mem_input.lock().unwrap() = format!("{start:08X}");
                    *bridge.edit.lock().unwrap() = SpikeEditState::default();
                    bridge.push(SpikeDebugRequest::Refresh);
                }
            }
            SpikeDebugMessage::PausePressed => bridge.push(SpikeDebugRequest::Pause),
            SpikeDebugMessage::ResumePressed => bridge.push(SpikeDebugRequest::Resume),
            SpikeDebugMessage::StepFramePressed => bridge.push(SpikeDebugRequest::StepFrame),
            SpikeDebugMessage::StepInstrPressed => bridge.push(SpikeDebugRequest::StepInstr),
            SpikeDebugMessage::MemInputChanged(text) => {
                *bridge.nav.mem_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            SpikeDebugMessage::MemGo => {
                let input = bridge.nav.mem_input.lock().unwrap().clone();
                match spike_parse_hex_addr(&input) {
                    Some(addr) => {
                        *bridge.nav.mem_addr.lock().unwrap() = addr;
                        bridge.push(SpikeDebugRequest::MemNav);
                    }
                    None => {
                        *bridge.status.lock().unwrap() = format!("parse failed: {input}");
                        bridge.view_invalidated.store(true, Ordering::Release);
                    }
                }
            }
            SpikeDebugMessage::MemPage(delta) => {
                let addr = *bridge.nav.mem_addr.lock().unwrap();
                // Page = 8 rows of 16 bytes (iteration 10 fits 800px).
                let next = if delta < 0 {
                    addr.saturating_sub(128)
                } else {
                    addr.saturating_add(128)
                };
                *bridge.nav.mem_addr.lock().unwrap() = next;
                *bridge.nav.mem_input.lock().unwrap() = format!("{next:08X}");
                bridge.push(SpikeDebugRequest::MemNav);
            }
            SpikeDebugMessage::DisInputChanged(text) => {
                *bridge.nav.dis_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            SpikeDebugMessage::DisGo => {
                let input = bridge.nav.dis_input.lock().unwrap().clone();
                match spike_parse_hex_addr(&input) {
                    Some(addr) => {
                        *bridge.nav.dis_addr.lock().unwrap() = addr;
                        bridge.nav.follow_pc.store(false, Ordering::Release);
                        bridge.push(SpikeDebugRequest::Refresh);
                    }
                    None => {
                        *bridge.status.lock().unwrap() = format!("parse failed: {input}");
                        bridge.view_invalidated.store(true, Ordering::Release);
                    }
                }
            }
            SpikeDebugMessage::FollowPcToggle => {
                bridge.nav.follow_pc.store(true, Ordering::Release);
                bridge.push(SpikeDebugRequest::Refresh);
            }
            SpikeDebugMessage::DisLineClicked(addr) => {
                bridge.navigate_dis(addr);
            }
            SpikeDebugMessage::FollowTarget(addr) => {
                bridge.navigate_dis(addr);
            }
            SpikeDebugMessage::DisBack => {
                let prev = bridge.nav.dis_back.lock().unwrap().pop();
                match prev {
                    Some(addr) => {
                        *bridge.nav.dis_addr.lock().unwrap() = addr;
                        *bridge.nav.dis_input.lock().unwrap() = format!("{addr:08X}");
                        bridge.nav.follow_pc.store(false, Ordering::Release);
                        bridge.push(SpikeDebugRequest::Refresh);
                    }
                    None => bridge.set_status("no history".to_string()),
                }
            }
            SpikeDebugMessage::BookmarkInputChanged(text) => {
                *bridge.bm_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            SpikeDebugMessage::BookmarkAdd => {
                let label = bridge.bm_input.lock().unwrap().trim().to_string();
                let mut marks = bridge.bookmarks.lock().unwrap();
                if marks.len() >= 8 {
                    bridge.set_status("bookmark list full".to_string());
                } else {
                    // Anchor on the PC row when following, else the pin.
                    let addr = bridge
                        .disasm_lines
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|line| line.is_pc)
                        .map(|line| line.addr)
                        .unwrap_or(*bridge.nav.dis_addr.lock().unwrap());
                    let n = marks.len() + 1;
                    let label = if label.is_empty() {
                        format!("BM{n}")
                    } else {
                        label
                    };
                    marks.push((addr, label));
                    bridge.bm_input.lock().unwrap().clear();
                    bridge.view_invalidated.store(true, Ordering::Release);
                }
            }
            SpikeDebugMessage::BookmarkJump(addr) => {
                bridge.navigate_dis(addr);
            }
            SpikeDebugMessage::BookmarkDelete(addr) => {
                bridge.bookmarks.lock().unwrap().retain(|(a, _)| *a != addr);
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            SpikeDebugMessage::WatchSet => {
                let selected = bridge.edit.lock().unwrap().selected;
                match selected {
                    Some(addr) => {
                        *bridge.watch.lock().unwrap() = Some(addr);
                        bridge.push(SpikeDebugRequest::Refresh);
                    }
                    None => bridge.set_status("watch needs a selection".to_string()),
                }
            }
            SpikeDebugMessage::WatchClear => {
                *bridge.watch.lock().unwrap() = None;
                *bridge.watch_value.lock().unwrap() = None;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            SpikeDebugMessage::FreezeSet => {
                let (selected, input) = {
                    let edit = bridge.edit.lock().unwrap();
                    (edit.selected, edit.input.clone())
                };
                match (
                    selected,
                    spike_parse_hex_addr(&input).filter(|v| *v <= 0xFF),
                ) {
                    (Some(addr), Some(value)) => {
                        *bridge.freeze.lock().unwrap() = Some((addr, value as u8));
                        bridge.set_status(format!("frozen {addr:08X}={value:02X}"));
                        bridge.push(SpikeDebugRequest::Refresh);
                    }
                    _ => bridge.set_status("freeze needs a selected row and hex byte".to_string()),
                }
            }
            SpikeDebugMessage::FreezeClear => {
                *bridge.freeze.lock().unwrap() = None;
                bridge.set_status("freeze cleared".to_string());
            }
            SpikeDebugMessage::RowSelected(addr) => {
                bridge.push(SpikeDebugRequest::EditSelect(addr));
            }
            SpikeDebugMessage::EditInputChanged(text) => {
                bridge.edit.lock().unwrap().input = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            SpikeDebugMessage::EditWrite => {
                // SPIKE (iteration 11): parse here (pure), stage in
                // the shell transaction via the drain. The confirm
                // text shows optimistically at once (drains run on
                // later events); the drain recomputes the
                // authoritative text from the transaction.
                let (selected, old, input) = {
                    let edit = bridge.edit.lock().unwrap();
                    (edit.selected, edit.old, edit.input.clone())
                };
                match (
                    selected,
                    spike_parse_hex_addr(&input).filter(|v| *v <= 0xFF),
                ) {
                    (Some(addr), Some(value)) => {
                        let old_text = match old {
                            Some(old) => format!("{old:02X}"),
                            None => "??".to_string(),
                        };
                        *bridge.pending_text.lock().unwrap() =
                            Some(format!("write {value:02X} to {addr:08X} (was {old_text})?"));
                        bridge.push(SpikeDebugRequest::EditStage(u64::from(value)));
                    }
                    (None, _) => {
                        bridge.set_status("write needs a selection".to_string());
                    }
                    (Some(_), None) => {
                        bridge.set_status(format!("parse failed: {input}"));
                    }
                }
            }
            SpikeDebugMessage::ConfirmWrite => {
                bridge.push(SpikeDebugRequest::WriteConfirm);
            }
            SpikeDebugMessage::CancelWrite => {
                bridge.push(SpikeDebugRequest::EditCancel);
            }
        }
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::Length;
        use iced::widget::{button, column, pick_list, row, text, text_input};
        // SPIKE (iteration 11): single coherent display capture per
        // rebuild (implementation review §14). Widgets below read
        // `data` only; no further bridge locks in view code.
        let data = {
            let bridge = &state.bridge;
            SpikeViewData {
                regs: bridge.regs.lock().unwrap().clone(),
                dump_rows: bridge.dump_rows.lock().unwrap().clone(),
                diff: bridge.diff.lock().unwrap().clone(),
                disasm_lines: bridge.disasm_lines.lock().unwrap().clone(),
                status: bridge.status.lock().unwrap().clone(),
                mem_input: bridge.nav.mem_input.lock().unwrap().clone(),
                dis_input: bridge.nav.dis_input.lock().unwrap().clone(),
                edit: bridge.edit.lock().unwrap().clone(),
                follow_pc: bridge.nav.follow_pc.load(Ordering::Acquire),
                back_len: bridge.nav.dis_back.lock().unwrap().len(),
                bookmarks: bridge.bookmarks.lock().unwrap().clone(),
                bm_input: bridge.bm_input.lock().unwrap().clone(),
                watch: *bridge.watch.lock().unwrap(),
                watch_value: *bridge.watch_value.lock().unwrap(),
                freeze: *bridge.freeze.lock().unwrap(),
                pending_text: bridge.pending_text.lock().unwrap().clone(),
                space_names: bridge.space_names(),
                selected_name: bridge.selected_name(),
            }
        };
        let regs = data.regs;
        let dump_rows = data.dump_rows;
        let diff = data.diff;
        let disasm_lines = data.disasm_lines;
        let status = data.status;
        let mem_input = data.mem_input;
        let dis_input = data.dis_input;
        let edit = data.edit;
        let follow_pc = data.follow_pc;
        let back_len = data.back_len;
        let bookmarks = data.bookmarks;
        let bm_input = data.bm_input;
        let watch = data.watch;
        let watch_value = data.watch_value;
        let freeze = data.freeze;
        // Button hierarchy (M3/Fluent): one primary per flow
        // (Confirm), secondary for toolbar/nav, text for list rows
        // and escape actions. Settings migrates to this language.
        let toolbar = row![
            button(text("Pause"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::PausePressed),
            button(text("Resume"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::ResumePressed),
            button(text("Step Frame"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::StepFramePressed),
            button(text("Step Instr"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::StepInstrPressed),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        // Settings form language: fixed label column + Fill control.
        let space_row = row![
            text("space").width(Length::Fixed(150.0)),
            pick_list(
                data.space_names,
                data.selected_name,
                SpikeDebugMessage::SpacePicked,
            )
            .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        let mem_nav = row![
            button(text("Prev"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::MemPage(-1)),
            button(text("Next"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::MemPage(1)),
            text_input("addr hex", &mem_input)
                .on_input(SpikeDebugMessage::MemInputChanged)
                .on_submit(SpikeDebugMessage::MemGo)
                .width(120),
            button(text("Go"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::MemGo),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        // Dump rows are text-style buttons OUTSIDE the scrollable
        // (scrollable children never yield messages under manual UI
        // driving). Selected row inverts to secondary; the rest stay
        // quiet so the list does not shout (M3: no high emphasis in
        // lists). Compact padding keeps 12 rows affordable.
        // Changed rows carry a `*` mark (Mesen access marks adapted
        // to text: theme-safe, no new colors; per-byte marks are
        // production work). Selection wins the secondary style.
        let mut dump_col = column![].spacing(2);
        for (addr, line) in &dump_rows {
            let selected = edit.selected == Some(*addr);
            let changed = diff.contains(addr);
            let marker = match (selected, changed) {
                (true, true) => ">*",
                (true, false) => "> ",
                (false, true) => " *",
                (false, false) => "  ",
            };
            let row_button = button(
                text(format!("{marker}{line}"))
                    .size(14)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 8])
            .width(Length::Fill)
            .on_press(SpikeDebugMessage::RowSelected(*addr));
            dump_col = dump_col.push(if edit.selected == Some(*addr) {
                row_button.style(button::secondary)
            } else {
                row_button.style(button::text)
            });
        }
        // Registers flow into two columns so tall lists (GBA: 17)
        // cost half the height. Pure text split, no system branches.
        let reg_lines: Vec<String> = regs.lines().map(str::to_string).collect();
        let reg_mid = reg_lines.len().div_ceil(2);
        let mut reg_left = column![].spacing(2);
        for line in &reg_lines[..reg_mid.min(reg_lines.len())] {
            reg_left = reg_left.push(text(line.clone()).size(14).font(iced::Font::MONOSPACE));
        }
        let mut reg_right = column![].spacing(2);
        for line in &reg_lines[reg_mid.min(reg_lines.len())..] {
            reg_right = reg_right.push(text(line.clone()).size(14).font(iced::Font::MONOSPACE));
        }
        let regs_view = row![reg_left, reg_right].spacing(16).width(Length::Fill);
        let old_text = match (edit.selected, edit.old) {
            (Some(addr), Some(old)) => format!("Edit {addr:08X} (was {old:02X})"),
            (Some(addr), None) => format!("Edit {addr:08X} (was ??)"),
            (None, _) => "Edit: select a dump row".to_string(),
        };
        let edit_row = row![
            text(old_text).size(14).width(Length::Fixed(240.0)),
            text_input("new hex byte", &edit.input)
                .on_input(SpikeDebugMessage::EditInputChanged)
                .on_submit(SpikeDebugMessage::EditWrite)
                .width(100),
            button(text("Write"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::EditWrite),
            button(text("Watch"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::WatchSet),
            button(text("Freeze"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::FreezeSet),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        let follow_button = button(text("Follow PC")).on_press(SpikeDebugMessage::FollowPcToggle);
        let back_button =
            button(text(format!("Back ({back_len})"))).on_press(SpikeDebugMessage::DisBack);
        let dis_nav = row![
            text(if follow_pc {
                "Disassembly (follow PC)"
            } else {
                "Disassembly (fixed)"
            }),
            if follow_pc {
                follow_button.style(button::secondary)
            } else {
                follow_button.style(button::text)
            },
            text_input("addr hex", &dis_input)
                .on_input(SpikeDebugMessage::DisInputChanged)
                .on_submit(SpikeDebugMessage::DisGo)
                .width(100),
            button(text("Go"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::DisGo),
            if back_len > 0 {
                back_button.style(button::secondary)
            } else {
                back_button.style(button::text)
            },
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        // Per-line disassembly: line click navigates (sets the pin),
        // the PC row highlights (Mesen/FCEUX canon), and lines with a
        // trailing `$XXXX` operand offer a follow button (no$ follow
        // adapted to buttons; cursor-key follow is production work).
        let mut dis_col = column![].spacing(2);
        if disasm_lines.is_empty() {
            dis_col = dis_col.push(text("(no disassembly)").size(14));
        }
        for line in &disasm_lines {
            let line_button = button(
                text(nerust_gui_shell::session::spike_format_disasm_line(line))
                    .size(14)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 8])
            .width(Length::Fill)
            .on_press(SpikeDebugMessage::DisLineClicked(line.addr));
            let line_button = if line.is_pc {
                line_button.style(button::secondary)
            } else {
                line_button.style(button::text)
            };
            let mut line_row = row![line_button].spacing(4);
            // SPIKE (iteration 11): structured target from the core;
            // text is never scraped (implementation review §14).
            if let Some(target) = line.target {
                line_row = line_row.push(
                    button(text("→").size(14))
                        .padding([2, 8])
                        .width(Length::Fixed(40.0))
                        .style(button::secondary)
                        .on_press(SpikeDebugMessage::FollowTarget(target)),
                );
            }
            dis_col = dis_col.push(line_row);
        }
        // Cross-emulator canon: execution above disassembly on the
        // left, state on the right.
        let left_col = column![
            text("SPIKE debugger probe (paused only)").size(18),
            toolbar,
            dis_nav,
            dis_col,
        ]
        .spacing(12)
        .width(Length::Fill);
        let watch_text = match (watch, watch_value) {
            (Some(addr), Some(value)) => format!("watch {addr:08X} = {value:02X}"),
            (Some(addr), None) => format!("watch {addr:08X} = ??"),
            (None, _) => "watch: none".to_string(),
        };
        let freeze_text = match freeze {
            Some((addr, value)) => format!("frozen {addr:08X}={value:02X}"),
            None => "freeze: none".to_string(),
        };
        let mut bookmarks_col = column![].spacing(2);
        for (addr, label) in &bookmarks {
            // SPIKE (iteration 11): jump text-button plus a text
            // delete (list caps at 8; full without removal leaks).
            bookmarks_col = bookmarks_col.push(
                row![
                    button(
                        text(format!("{label} {addr:08X}"))
                            .size(14)
                            .font(iced::Font::MONOSPACE),
                    )
                    .padding([2, 8])
                    .style(button::text)
                    .on_press(SpikeDebugMessage::BookmarkJump(*addr)),
                    button(text("×").size(14))
                        .padding([2, 8])
                        .style(button::text)
                        .on_press(SpikeDebugMessage::BookmarkDelete(*addr)),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
            );
        }
        let right_col = column![
            text(status).size(14).width(Length::Fill),
            text("Registers").size(16),
            regs_view,
            text("Watch").size(16),
            row![
                text(watch_text).size(14).width(Length::Fill),
                button(text("Clear"))
                    .style(button::text)
                    .on_press(SpikeDebugMessage::WatchClear),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            text("Freeze").size(16),
            row![
                text(freeze_text).size(14).width(Length::Fill),
                button(text("Clear"))
                    .style(button::text)
                    .on_press(SpikeDebugMessage::FreezeClear),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            text("Bookmarks").size(16),
            row![
                text_input("label", &bm_input)
                    .on_input(SpikeDebugMessage::BookmarkInputChanged)
                    .on_submit(SpikeDebugMessage::BookmarkAdd)
                    .width(140),
                button(text("Add"))
                    .style(button::secondary)
                    .on_press(SpikeDebugMessage::BookmarkAdd),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            bookmarks_col,
        ]
        .spacing(12)
        .width(Length::Fixed(340.0));
        let mut content = column![
            row![left_col, right_col].spacing(16),
            text("Memory").size(16),
            space_row,
            mem_nav,
            dump_col,
            edit_row,
        ]
        .spacing(10)
        .padding(16)
        .width(Length::Fill);
        // SPIKE (iteration 11): confirm text is display cache from
        // the shell transaction; the bridge holds no pending write.
        if let Some(pending) = data.pending_text {
            content = content.push(
                row![
                    text(pending).size(14),
                    button(text("Confirm"))
                        .style(button::primary)
                        .on_press(SpikeDebugMessage::ConfirmWrite),
                    button(text("Cancel"))
                        .style(button::text)
                        .on_press(SpikeDebugMessage::CancelWrite),
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center),
            );
        }
        content = content.push(
            button(text("Refresh"))
                .style(button::secondary)
                .on_press(SpikeDebugMessage::RefreshPressed),
        );
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
        disasm_lines: Vec<DisasmLine>,
        panels: String,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        let bridge = Arc::new(SpikeDebugBridge::new(
            regs,
            dump_rows,
            disasm_lines,
            panels,
            images,
            spaces,
            mem_addr,
        ));
        let window = Arc::new(
            WindowBuilder::new()
                .with_title("Debugger (spike)")
                .with_inner_size(tao::dpi::LogicalSize::new(1100.0, 800.0))
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

    /// SPIKE (iteration 10): PPU sync gate for the runtime. The main
    /// window steals the shared PPU flag so the PPU window rebuilds
    /// exactly when fresh pixels land (hover rebuilds from its own
    /// window events instead).
    pub(crate) fn take_ppu_dirty(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.bridge.ppu_invalidated.swap(false, Ordering::AcqRel)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_all(
        &mut self,
        regs: String,
        dump_rows: Vec<(u32, String)>,
        disasm_lines: Vec<DisasmLine>,
        panels: String,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        status: String,
        watch_value: Option<u8>,
        diff: Vec<u32>,
    ) {
        self.bridge.set_all(
            regs,
            dump_rows,
            disasm_lines,
            panels,
            images,
            status,
            watch_value,
            diff,
        );
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
        // SPIKE (iteration 9): Light theme for OS light-theme
        // nativeness. The production settings window migrates to the
        // same shared language; this probe leads it.
        let theme = iced::Theme::Light;
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
        if let Err(e) = self.renderer.present(&vp, style.background_color) {
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
