//! Debugger window: two-pane memory/disassembly probe plus register,
//! watch, freeze, and bookmark state.
//!
//! Read-only in Phase C: row selection drives watch/freeze targets,
//! and the write flow arrives in Phase E. The program stays pure;
//! the host drain executes requests against the session and refreshes
//! the display cache.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use iced::{Size, Task, advanced::renderer, keyboard, mouse, theme};
use iced_tiny_skia::{
    Renderer,
    graphics::compositor::Compositor as _,
    window::{Compositor, Surface, compositor},
};
use iced_winit::{
    Clipboard,
    graphics::Viewport,
    program::{self, Program},
    runtime::user_interface::{Cache, UserInterface},
};
use nerust_core_traits::debugger::{DUMP_ROW_BYTES, SpaceId};
use nerust_gui_viewmodel::debugger::{self as vm, DisplayCache, NavState};

#[cfg(target_os = "macos")]
use tao::platform::macos::WindowBuilderExtMacOS;
use tao::{
    event_loop::EventLoopWindowTarget,
    keyboard::ModifiersState as TaoModifiers,
    window::{Window as TaoWindow, WindowBuilder},
};

use crate::tao_conversions::{convert_tao_window_event, default_font};

/// Initial debugger window size (verified Xvfb fit).
pub(crate) const DEBUG_WINDOW_SIZE: (f64, f64) = (1100.0, 800.0);
/// Right state column width.
const STATE_COLUMN_WIDTH: f32 = 340.0;
/// Dump rows per page; paging moves by a full page.
const DUMP_PAGE_ROWS: i32 = 8;
/// Bookmark cap (window-local, no file yet).
const BOOKMARK_CAP: usize = 8;

/// Request from the debugger program to the host drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DebugRequest {
    Refresh,
    Pause,
    Resume,
    TogglePause,
    StepFrame,
    StepInstr,
    MemNav,
    SelectRow(u32),
}

/// In-window debugger message. Pure: update touches the bridge only.
#[derive(Debug, Clone)]
pub(crate) enum DebugMessage {
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
    DisLineClicked(u32),
    FollowTarget(u32),
    DisBack,
    BookmarkInputChanged(String),
    BookmarkAdd,
    BookmarkJump(u32),
    BookmarkDelete(u32),
    WatchSet,
    WatchClear,
    FreezeInputChanged(String),
    FreezeSet,
    FreezeClear,
    RowSelected(u32),
}

/// Shared bridge between the iced program and the host drain.
/// Movement state and display cache are owned separately; user lists
/// (bookmarks, watch, freeze, selection) are presentation state.
pub(crate) struct DebugBridge {
    pub(crate) nav: Mutex<NavState>,
    pub(crate) display: Mutex<DisplayCache>,
    pub(crate) spaces: Vec<(SpaceId, String, u32)>,
    pub(crate) bookmarks: Mutex<Vec<(u32, String)>>,
    pub(crate) bm_input: Mutex<String>,
    pub(crate) watch: Mutex<Option<u32>>,
    pub(crate) freeze: Mutex<Option<(u32, u8)>>,
    pub(crate) freeze_input: Mutex<String>,
    pub(crate) selected: Mutex<Option<(u32, Option<u8>)>>,
    pub(crate) ppu_hover: Mutex<String>,
    pub(crate) outbox: Mutex<Vec<DebugRequest>>,
    pub(crate) view_invalidated: AtomicBool,
    pub(crate) ppu_invalidated: AtomicBool,
}

impl DebugBridge {
    pub(crate) fn new(
        display: DisplayCache,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
    ) -> Self {
        Self {
            nav: Mutex::new(NavState::new(spaces.len(), mem_addr)),
            display: Mutex::new(display),
            spaces,
            bookmarks: Mutex::new(Vec::new()),
            bm_input: Mutex::new(String::new()),
            watch: Mutex::new(None),
            freeze: Mutex::new(None),
            freeze_input: Mutex::new(String::new()),
            selected: Mutex::new(None),
            ppu_hover: Mutex::new(String::new()),
            outbox: Mutex::new(Vec::new()),
            view_invalidated: AtomicBool::new(false),
            ppu_invalidated: AtomicBool::new(false),
        }
    }

    pub(crate) fn selected_space(&self) -> Option<SpaceId> {
        let idx = self.nav.lock().unwrap().space_idx();
        self.spaces.get(idx).map(|(id, _, _)| *id)
    }

    pub(crate) fn selected_name(&self) -> Option<String> {
        let idx = self.nav.lock().unwrap().space_idx();
        self.spaces.get(idx).map(|(_, name, _)| name.clone())
    }

    pub(crate) fn set_status(&self, status: String) {
        self.display.lock().unwrap().status = status;
        self.view_invalidated.store(true, Ordering::Release);
    }

    pub(crate) fn take_requests(&self) -> Vec<DebugRequest> {
        std::mem::take(&mut *self.outbox.lock().unwrap())
    }

    fn push(&self, request: DebugRequest) {
        self.outbox.lock().unwrap().push(request);
        self.view_invalidated.store(true, Ordering::Release);
    }

    /// Displayed disassembly anchor: first visible line in follow-PC
    /// mode (the stored pin is stale there), else the pin.
    fn anchor(&self) -> u32 {
        let nav = self.nav.lock().unwrap();
        if nav.follow_pc() {
            self.display
                .lock()
                .unwrap()
                .disasm_lines
                .first()
                .map(|line| line.addr)
                .unwrap_or(0)
        } else {
            nav.dis_addr().unwrap_or(0)
        }
    }
}

pub(crate) struct DebugState {
    bridge: Arc<DebugBridge>,
}

pub(crate) struct DebugProgram {
    pub(crate) bridge: Arc<DebugBridge>,
}

impl Program for DebugProgram {
    type State = DebugState;
    type Message = DebugMessage;
    type Theme = iced::Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_debug"
    }

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
            DebugState {
                bridge: Arc::clone(&self.bridge),
            },
            Task::none(),
        )
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        let bridge = &state.bridge;
        match message {
            DebugMessage::RefreshPressed => bridge.push(DebugRequest::Refresh),
            // Space stepper (not a dropdown): overlay menus are
            // undrivable on headless Xvfb, and four fixed spaces suit
            // Prev/Next cycling. The dropdown decision is revisited
            // with evidence in the design notes.
            DebugMessage::SpacePrev => {
                let mut nav = bridge.nav.lock().unwrap();
                if let Some(idx) = nav.cycle_space(-1) {
                    let start = bridge.spaces[idx].2;
                    nav.set_space(idx, start);
                    drop(nav);
                    *bridge.selected.lock().unwrap() = None;
                    bridge.push(DebugRequest::Refresh);
                }
            }
            DebugMessage::SpaceNext => {
                let mut nav = bridge.nav.lock().unwrap();
                if let Some(idx) = nav.cycle_space(1) {
                    let start = bridge.spaces[idx].2;
                    nav.set_space(idx, start);
                    drop(nav);
                    *bridge.selected.lock().unwrap() = None;
                    bridge.push(DebugRequest::Refresh);
                }
            }
            DebugMessage::PausePressed => bridge.push(DebugRequest::Pause),
            DebugMessage::ResumePressed => bridge.push(DebugRequest::Resume),
            DebugMessage::StepFramePressed => bridge.push(DebugRequest::StepFrame),
            DebugMessage::StepInstrPressed => bridge.push(DebugRequest::StepInstr),
            DebugMessage::MemInputChanged(text) => {
                bridge.nav.lock().unwrap().set_mem_input(text);
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::MemGo => {
                let input = bridge.nav.lock().unwrap().mem_input().to_string();
                match vm::parse_hex_addr(&input) {
                    Some(addr) => {
                        bridge.nav.lock().unwrap().go_mem(addr);
                        bridge.push(DebugRequest::MemNav);
                    }
                    None => bridge.set_status(format!("parse failed: {input}")),
                }
            }
            DebugMessage::MemPage(delta) => {
                bridge
                    .nav
                    .lock()
                    .unwrap()
                    .page_mem(delta * DUMP_PAGE_ROWS * DUMP_ROW_BYTES as i32);
                bridge.push(DebugRequest::MemNav);
            }
            DebugMessage::DisInputChanged(text) => {
                bridge.nav.lock().unwrap().set_dis_input(text);
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::DisGo => {
                let input = bridge.nav.lock().unwrap().dis_input().to_string();
                match vm::parse_hex_addr(&input) {
                    Some(addr) => {
                        let anchor = bridge.anchor();
                        bridge.nav.lock().unwrap().navigate(anchor, addr);
                        bridge.push(DebugRequest::Refresh);
                    }
                    None => bridge.set_status(format!("parse failed: {input}")),
                }
            }
            DebugMessage::FollowPcToggle => {
                bridge.nav.lock().unwrap().follow();
                bridge.push(DebugRequest::Refresh);
            }
            DebugMessage::DisLineClicked(addr) => {
                let anchor = bridge.anchor();
                bridge.nav.lock().unwrap().navigate(anchor, addr);
                bridge.push(DebugRequest::Refresh);
            }
            DebugMessage::FollowTarget(addr) => {
                let anchor = bridge.anchor();
                bridge.nav.lock().unwrap().navigate(anchor, addr);
                bridge.push(DebugRequest::Refresh);
            }
            DebugMessage::DisBack => match bridge.nav.lock().unwrap().go_back() {
                Some(_) => bridge.push(DebugRequest::Refresh),
                None => bridge.set_status("back stack is empty".to_string()),
            },
            DebugMessage::BookmarkInputChanged(text) => {
                *bridge.bm_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::BookmarkAdd => {
                let label = bridge.bm_input.lock().unwrap().trim().to_string();
                let mut marks = bridge.bookmarks.lock().unwrap();
                if marks.len() >= BOOKMARK_CAP {
                    bridge.set_status("bookmark list full".to_string());
                } else {
                    // Anchor on the PC row when following, else the pin.
                    let addr = bridge
                        .display
                        .lock()
                        .unwrap()
                        .disasm_lines
                        .iter()
                        .find(|line| line.is_pc)
                        .map(|line| line.addr)
                        .unwrap_or_else(|| bridge.anchor());
                    let n = marks.len() + 1;
                    marks.push((
                        addr,
                        if label.is_empty() {
                            format!("BM{n}")
                        } else {
                            label
                        },
                    ));
                    bridge.bm_input.lock().unwrap().clear();
                    bridge.view_invalidated.store(true, Ordering::Release);
                }
            }
            DebugMessage::BookmarkJump(addr) => {
                let anchor = bridge.anchor();
                bridge.nav.lock().unwrap().navigate(anchor, addr);
                bridge.push(DebugRequest::Refresh);
            }
            DebugMessage::BookmarkDelete(addr) => {
                bridge.bookmarks.lock().unwrap().retain(|(a, _)| *a != addr);
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::WatchSet => {
                let selected = bridge.selected.lock().unwrap().map(|(addr, _)| addr);
                match selected {
                    Some(addr) => {
                        *bridge.watch.lock().unwrap() = Some(addr);
                        bridge.push(DebugRequest::Refresh);
                    }
                    None => bridge.set_status("watch needs a selection".to_string()),
                }
            }
            DebugMessage::WatchClear => {
                *bridge.watch.lock().unwrap() = None;
                bridge.display.lock().unwrap().watch_value = None;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::FreezeInputChanged(text) => {
                *bridge.freeze_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::FreezeSet => {
                let selected = bridge.selected.lock().unwrap().map(|(addr, _)| addr);
                let input = bridge.freeze_input.lock().unwrap().clone();
                match (selected, vm::parse_hex_addr(&input).filter(|v| *v <= 0xFF)) {
                    (Some(addr), Some(value)) => {
                        *bridge.freeze.lock().unwrap() = Some((addr, value as u8));
                        bridge.set_status(format!("frozen {addr:08X}={value:02X}"));
                        bridge.push(DebugRequest::Refresh);
                    }
                    _ => bridge.set_status("freeze needs a selected row and hex byte".to_string()),
                }
            }
            DebugMessage::FreezeClear => {
                *bridge.freeze.lock().unwrap() = None;
                bridge.set_status("freeze cleared".to_string());
            }
            DebugMessage::RowSelected(addr) => {
                bridge.push(DebugRequest::SelectRow(addr));
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
        use iced::widget::{button, column, row, text, text_input};
        // Single coherent capture per rebuild; widgets read locals only.
        let bridge = &state.bridge;
        let nav = bridge.nav.lock().unwrap().clone();
        let display = bridge.display.lock().unwrap().clone();
        let bookmarks = bridge.bookmarks.lock().unwrap().clone();
        let bm_input = bridge.bm_input.lock().unwrap().clone();
        let watch = *bridge.watch.lock().unwrap();
        let freeze = *bridge.freeze.lock().unwrap();
        let freeze_input = bridge.freeze_input.lock().unwrap().clone();
        let selected = *bridge.selected.lock().unwrap();
        let selected_name = bridge.selected_name();
        let regs = display.regs;
        let dump_rows = display.dump_rows;
        let diff = display.diff;
        let disasm_lines = display.disasm_lines;
        let status = display.status;
        let mem_input = nav.mem_input().to_string();
        let dis_input = nav.dis_input().to_string();
        let follow_pc = nav.follow_pc();
        let back_len = nav.back_len();
        let watch_value = display.watch_value;
        // Button hierarchy: one primary per flow (reserved for the
        // Phase E Confirm), secondary for toolbar/nav, text for list
        // rows and escape actions.
        let toolbar = row![
            button(text("Pause"))
                .style(button::secondary)
                .on_press(DebugMessage::PausePressed),
            button(text("Resume"))
                .style(button::secondary)
                .on_press(DebugMessage::ResumePressed),
            button(text("Step Frame"))
                .style(button::secondary)
                .on_press(DebugMessage::StepFramePressed),
            button(text("Step Instr"))
                .style(button::secondary)
                .on_press(DebugMessage::StepInstrPressed),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        // Settings form language: fixed label column + stepper.
        let space_row = row![
            text("space").width(Length::Fixed(150.0)),
            button(text("Prev"))
                .style(button::secondary)
                .on_press(DebugMessage::SpacePrev),
            text(selected_name.unwrap_or_default())
                .size(14)
                .width(Length::Fill),
            button(text("Next"))
                .style(button::secondary)
                .on_press(DebugMessage::SpaceNext),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        let mem_nav = row![
            button(text("Prev"))
                .style(button::secondary)
                .on_press(DebugMessage::MemPage(-1)),
            button(text("Next"))
                .style(button::secondary)
                .on_press(DebugMessage::MemPage(1)),
            text_input("addr hex", &mem_input)
                .on_input(DebugMessage::MemInputChanged)
                .on_submit(DebugMessage::MemGo)
                .width(120),
            button(text("Go"))
                .style(button::secondary)
                .on_press(DebugMessage::MemGo),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        // Dump rows are text-style buttons; selection inverts to
        // secondary. Changed rows carry a `*` mark (theme-safe, no
        // new colors). Selection wins the secondary style.
        let mut dump_col = column![].spacing(2);
        for (addr, line) in &dump_rows {
            let is_selected = selected.map(|(a, _)| a) == Some(*addr);
            let changed = diff.contains(addr);
            let marker = match (is_selected, changed) {
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
            .on_press(DebugMessage::RowSelected(*addr));
            dump_col = dump_col.push(if is_selected {
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
        for line in reg_lines.iter().take(reg_mid.min(reg_lines.len())) {
            reg_left = reg_left.push(text(line.clone()).size(14).font(iced::Font::MONOSPACE));
        }
        let mut reg_right = column![].spacing(2);
        for line in reg_lines.iter().skip(reg_mid.min(reg_lines.len())) {
            reg_right = reg_right.push(text(line.clone()).size(14).font(iced::Font::MONOSPACE));
        }
        let regs_view = row![reg_left, reg_right].spacing(16).width(Length::Fill);
        // Phase C selection display (Phase E adds the write input and
        // confirm row here).
        let select_text = match selected {
            Some((addr, Some(old))) => format!("Selected {addr:08X} (was {old:02X})"),
            Some((addr, None)) => format!("Selected {addr:08X} (was ??)"),
            None => "Select a dump row".to_string(),
        };
        let edit_row = row![
            text(select_text).size(14).width(Length::Fixed(240.0)),
            button(text("Watch"))
                .style(button::secondary)
                .on_press(DebugMessage::WatchSet),
            button(text("Freeze"))
                .style(button::secondary)
                .on_press(DebugMessage::FreezeSet),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        let follow_button = button(text("Follow PC")).on_press(DebugMessage::FollowPcToggle);
        let back_button =
            button(text(format!("Back ({back_len})"))).on_press(DebugMessage::DisBack);
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
                .on_input(DebugMessage::DisInputChanged)
                .on_submit(DebugMessage::DisGo)
                .width(100),
            button(text("Go"))
                .style(button::secondary)
                .on_press(DebugMessage::DisGo),
            if back_len > 0 {
                back_button.style(button::secondary)
            } else {
                back_button.style(button::text)
            },
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center);
        // Per-line disassembly: line click pins the anchor, the PC row
        // highlights, and structured targets offer follow buttons.
        let mut dis_col = column![].spacing(2);
        if disasm_lines.is_empty() {
            dis_col = dis_col.push(text("(no disassembly)").size(14));
        }
        for line in &disasm_lines {
            let line_button = button(
                text(vm::format_disasm_line(line))
                    .size(14)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 8])
            .width(Length::Fill)
            .on_press(DebugMessage::DisLineClicked(line.addr));
            let line_button = if line.is_pc {
                line_button.style(button::secondary)
            } else {
                line_button.style(button::text)
            };
            let mut line_row = row![line_button].spacing(4);
            if let Some(target) = line.target {
                line_row = line_row.push(
                    button(text("→").size(14))
                        .padding([2, 8])
                        .width(Length::Fixed(40.0))
                        .style(button::secondary)
                        .on_press(DebugMessage::FollowTarget(target)),
                );
            }
            dis_col = dis_col.push(line_row);
        }
        // Cross-emulator canon: execution above disassembly on the
        // left, state on the right.
        let left_col = column![text("Debugger").size(18), toolbar, dis_nav, dis_col]
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
            bookmarks_col = bookmarks_col.push(
                row![
                    button(
                        text(format!("{label} {addr:08X}"))
                            .size(14)
                            .font(iced::Font::MONOSPACE),
                    )
                    .padding([2, 8])
                    .style(button::text)
                    .on_press(DebugMessage::BookmarkJump(*addr)),
                    button(text("×").size(14))
                        .padding([2, 8])
                        .style(button::text)
                        .on_press(DebugMessage::BookmarkDelete(*addr)),
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
                button(text("Watch"))
                    .style(button::secondary)
                    .on_press(DebugMessage::WatchSet),
                text(watch_text).size(14).width(Length::Fill),
                button(text("Clear"))
                    .style(button::text)
                    .on_press(DebugMessage::WatchClear),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            text("Freeze").size(16),
            row![
                text_input("hex byte", &freeze_input)
                    .on_input(DebugMessage::FreezeInputChanged)
                    .width(100),
                button(text("Freeze"))
                    .style(button::secondary)
                    .on_press(DebugMessage::FreezeSet),
                text(freeze_text).size(14).width(Length::Fill),
                button(text("Clear"))
                    .style(button::text)
                    .on_press(DebugMessage::FreezeClear),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            text("Bookmarks").size(16),
            row![
                text_input("label", &bm_input)
                    .on_input(DebugMessage::BookmarkInputChanged)
                    .on_submit(DebugMessage::BookmarkAdd)
                    .width(Length::Fill),
                button(text("Add"))
                    .style(button::secondary)
                    .on_press(DebugMessage::BookmarkAdd),
            ]
            .spacing(12)
            .align_y(iced::Alignment::Center),
            bookmarks_col,
        ]
        .spacing(12)
        .width(Length::Fixed(STATE_COLUMN_WIDTH));
        let content = column![
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
        let content = content.push(
            button(text("Refresh"))
                .style(button::secondary)
                .on_press(DebugMessage::RefreshPressed),
        );
        content.into()
    }
}

/// Owns Instance + Cache + UI, mirroring `settings_window::UiState`.
pub(crate) struct DebugUiState {
    ui: std::mem::ManuallyDrop<
        UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
    >,
    instance: program::Instance<DebugProgram>,
    bridge: Arc<DebugBridge>,
}

impl DebugUiState {
    fn build_ui(
        instance: &program::Instance<DebugProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        cache: Cache,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer> {
        unsafe {
            std::mem::transmute::<
                UserInterface<'_, DebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
                UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer>,
            >(UserInterface::build(
                instance.view(window_id),
                bounds,
                cache,
                renderer,
            ))
        }
    }

    fn new(
        instance: program::Instance<DebugProgram>,
        window_id: iced::window::Id,
        bounds: Size,
        renderer: &mut iced_tiny_skia::Renderer,
        bridge: Arc<DebugBridge>,
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
    ) -> &mut UserInterface<'static, DebugMessage, iced::Theme, iced_tiny_skia::Renderer> {
        &mut self.ui
    }

    fn process_messages(
        &mut self,
        messages: Vec<DebugMessage>,
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

impl Drop for DebugUiState {
    fn drop(&mut self) {
        unsafe { std::mem::ManuallyDrop::drop(&mut self.ui) };
    }
}

pub(crate) struct DebugWindowHandle {
    pub(crate) window: Arc<TaoWindow>,
    window_id: iced::window::Id,
    ui_state: DebugUiState,
    renderer: DebugRenderer,
    viewport_physical: (u32, u32),
    pub(crate) scale_factor: f32,
    pub(crate) modifiers: keyboard::Modifiers,
    pub(crate) should_close: Arc<AtomicBool>,
    pub(crate) bridge: Arc<DebugBridge>,
    cursor: mouse::Cursor,
    clipboard: Clipboard,
}

pub(crate) struct DebugRenderer {
    compositor: Compositor,
    surface: Surface,
    backend: Renderer,
}

impl DebugRenderer {
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

impl DebugWindowHandle {
    pub(crate) fn new(
        display: DisplayCache,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
        position: Option<(i32, i32)>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        let bridge = Arc::new(DebugBridge::new(display, spaces, mem_addr));
        let mut builder = WindowBuilder::new().with_title("Debugger").with_inner_size(
            tao::dpi::LogicalSize::new(DEBUG_WINDOW_SIZE.0, DEBUG_WINDOW_SIZE.1),
        );
        #[cfg(target_os = "macos")]
        {
            builder = builder.with_automatic_window_tabbing(false);
        }
        if let Some((x, y)) = position {
            builder = builder.with_position(tao::dpi::LogicalPosition::new(x as f64, y as f64));
        }
        let window = Arc::new(match builder.build(event_loop) {
            Ok(window) => window,
            Err(error) => {
                log::error!("failed to create debugger window: {error}");
                return None;
            }
        });
        let window_id = iced::window::Id::unique();
        let program = DebugProgram {
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
        let ui_state = DebugUiState::new(
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
            renderer: DebugRenderer {
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

    pub(crate) fn set_display(&mut self, display: DisplayCache) {
        *self.bridge.display.lock().unwrap() = display;
        self.sync_from_bridge();
    }

    pub(crate) fn take_requests(&self) -> Vec<DebugRequest> {
        self.bridge.take_requests()
    }

    pub(crate) fn render(&mut self) {
        // Light theme for OS light-theme nativeness.
        let theme = iced::Theme::Light;
        let style = <iced::Theme as theme::Base>::base(&theme);
        let viewport = Viewport::with_physical_size(
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
        if let Err(error) = self.renderer.present(&viewport, style.background_color) {
            log::warn!("debugger render present failed: {error}");
        }
    }

    pub(crate) fn handle_event(&mut self, mapped: iced::Event) {
        // Window-scoped shortcuts on physical codes (F6 step frame,
        // F7 step instruction, F9 pause toggle). Slot switching owns
        // no F-keys, so no schema conflict; the main window keeps F4.
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
                Physical::Code(Code::F6) => Some(DebugRequest::StepFrame),
                Physical::Code(Code::F7) => Some(DebugRequest::StepInstr),
                Physical::Code(Code::F9) => Some(DebugRequest::TogglePause),
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

    pub(crate) fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor;
    }

    pub(crate) fn set_modifiers(&mut self, modifiers: TaoModifiers) {
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

    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.viewport_physical = (width, height);
        self.renderer.resize(width, height);
    }

    pub(crate) fn sync_from_bridge(&mut self) {
        let bounds = Viewport::with_physical_size(
            Size::new(self.viewport_physical.0, self.viewport_physical.1),
            self.scale_factor,
        )
        .logical_size();
        self.ui_state
            .sync_from_bridge(self.window_id, bounds, &mut self.renderer.backend);
        self.window.request_redraw();
    }
}
