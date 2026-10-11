//! Debugger window: two-pane memory/disassembly probe plus register,
//! watch, freeze, and bookmark state.
//!
//! Read-only in Phase C; Phase E adds the two-phase write (select,
//! stage, confirm/cancel) with the shell-owned transaction. The
//! program stays pure; the host drain executes requests against the
//! session and refreshes the display cache.

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
use nerust_gui_settings::language::AppLanguage;
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{self as vm, DisplayCache, NavState};
use nerust_settings_core::i18n::{UiText, text as ui_text};

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

/// In-window debugger message. Pure: update touches the bridge only.
#[derive(Debug, Clone)]
pub(crate) enum DebugMessage {
    RefreshPressed,
    SpacePrev,
    SpaceNext,
    TogglePausePressed,
    StepFramePressed,
    StepInstrPressed,
    PpuOpenPressed,
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
    EditFreezeSet,
    FreezeClear,
    RowSelected(u32),
    EditInputChanged(String),
    EditWrite,
    ConfirmWrite,
    CancelWrite,
    RegSelected(&'static str),
    RegInputChanged(String),
    RegSet,
}

/// Shared bridge between the iced program and the host drain.
/// Movement state and display cache are owned separately; user lists
/// (bookmarks, watch, freeze, selection) are presentation state.
pub(crate) struct DebugBridge {
    pub(crate) nav: Mutex<NavState>,
    pub(crate) display: Mutex<DisplayCache>,
    pub(crate) spaces: Vec<(SpaceId, String, u32)>,
    /// UI language snapshot at open time.
    pub(crate) language: AppLanguage,
    pub(crate) bookmarks: Mutex<Vec<(u32, String)>>,
    pub(crate) bm_input: Mutex<String>,
    pub(crate) watch: Mutex<Option<u32>>,
    pub(crate) freeze: Mutex<Option<(u32, u8)>>,
    pub(crate) freeze_input: Mutex<String>,
    pub(crate) selected: Mutex<Option<(u32, Option<u8>)>>,
    /// New-value input for the two-phase write (Phase E).
    pub(crate) edit_input: Mutex<String>,
    /// Register write target (kernel name) plus hex input. Mirrors
    /// the memory edit row: pick a register, type a value, Set.
    pub(crate) reg_target: Mutex<Option<&'static str>>,
    pub(crate) reg_input: Mutex<String>,
    pub(crate) ppu_hover: Mutex<String>,
    pub(crate) outbox: Mutex<Vec<DebugAction>>,
    pub(crate) view_invalidated: AtomicBool,
    pub(crate) ppu_invalidated: AtomicBool,
    /// Toolbar PPU-viewer request: host-owned window, so the iced
    /// program only raises the flag (same split as F4 handling).
    pub(crate) open_ppu: AtomicBool,
}

impl DebugBridge {
    pub(crate) fn new(
        display: DisplayCache,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        language: AppLanguage,
    ) -> Self {
        Self {
            nav: Mutex::new(NavState::new(spaces.len(), mem_addr)),
            display: Mutex::new(display),
            spaces,
            language,
            bookmarks: Mutex::new(Vec::new()),
            bm_input: Mutex::new(String::new()),
            watch: Mutex::new(None),
            freeze: Mutex::new(None),
            freeze_input: Mutex::new(String::new()),
            selected: Mutex::new(None),
            edit_input: Mutex::new(String::new()),
            reg_target: Mutex::new(None),
            reg_input: Mutex::new(String::new()),
            ppu_hover: Mutex::new(String::new()),
            outbox: Mutex::new(Vec::new()),
            view_invalidated: AtomicBool::new(false),
            ppu_invalidated: AtomicBool::new(false),
            open_ppu: AtomicBool::new(false),
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

    pub(crate) fn take_requests(&self) -> Vec<DebugAction> {
        std::mem::take(&mut *self.outbox.lock().unwrap())
    }

    fn push(&self, request: DebugAction) {
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

/// Shared freeze staging: selection plus a hex byte. The top
/// Freeze row and the edit-row Freeze button differ only in which
/// input they read; the rules live here, not in the widgets.
fn apply_freeze(bridge: &DebugBridge, input: &str) {
    let selected = bridge.selected.lock().unwrap().map(|(addr, _)| addr);
    match (selected, vm::parse_hex_addr(input).filter(|v| *v <= 0xFF)) {
        (Some(addr), Some(value)) => {
            *bridge.freeze.lock().unwrap() = Some((addr, value as u8));
            bridge.set_status(format!("frozen {addr:08X}={value:02X}"));
            bridge.push(DebugAction::Refresh);
        }
        _ => bridge.set_status("freeze needs a selected row and hex byte".to_string()),
    }
}

// Space stepper (not a dropdown): overlay menus are undrivable on
// headless Xvfb, and fixed spaces suit Prev/Next cycling. The
// dropdown decision is revisited with evidence in the design notes.
fn cycle_space(bridge: &DebugBridge, delta: i32) {
    let mut nav = bridge.nav.lock().unwrap();
    if let Some(idx) = nav.cycle_space(delta) {
        let start = bridge.spaces[idx].2;
        nav.set_space(idx, start);
        drop(nav);
        *bridge.selected.lock().unwrap() = None;
        bridge.push(DebugAction::Refresh);
    }
}

fn mem_go(bridge: &DebugBridge) {
    let input = bridge.nav.lock().unwrap().mem_input().to_string();
    match vm::parse_hex_addr(&input) {
        Some(addr) => {
            bridge.nav.lock().unwrap().go_mem(addr);
            bridge.push(DebugAction::MemNav);
        }
        None => bridge.set_status(format!("parse failed: {input}")),
    }
}

fn mem_page(bridge: &DebugBridge, delta: i32) {
    bridge
        .nav
        .lock()
        .unwrap()
        .page_mem(delta * DUMP_PAGE_ROWS * DUMP_ROW_BYTES as i32);
    bridge.push(DebugAction::MemNav);
}

fn dis_go(bridge: &DebugBridge) {
    let input = bridge.nav.lock().unwrap().dis_input().to_string();
    match vm::parse_hex_addr(&input) {
        Some(addr) => navigate_to(bridge, addr),
        None => bridge.set_status(format!("parse failed: {input}")),
    }
}

fn navigate_to(bridge: &DebugBridge, addr: u32) {
    let anchor = bridge.anchor();
    bridge.nav.lock().unwrap().navigate(anchor, addr);
    bridge.push(DebugAction::Refresh);
}

fn dis_back(bridge: &DebugBridge) {
    match bridge.nav.lock().unwrap().go_back() {
        Some(_) => bridge.push(DebugAction::Refresh),
        None => bridge.set_status("back stack is empty".to_string()),
    }
}

fn bookmark_add(bridge: &DebugBridge) {
    let label = bridge.bm_input.lock().unwrap().trim().to_string();
    let mut marks = bridge.bookmarks.lock().unwrap();
    if marks.len() >= BOOKMARK_CAP {
        bridge.set_status("bookmark list full".to_string());
        return;
    }
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

fn watch_set(bridge: &DebugBridge) {
    let selected = bridge.selected.lock().unwrap().map(|(addr, _)| addr);
    match selected {
        Some(addr) => {
            *bridge.watch.lock().unwrap() = Some(addr);
            bridge.push(DebugAction::Refresh);
        }
        None => bridge.set_status("watch needs a selection".to_string()),
    }
}

fn edit_write(bridge: &DebugBridge) {
    let input = bridge.edit_input.lock().unwrap().trim().to_string();
    let selected = *bridge.selected.lock().unwrap();
    match (
        selected,
        vm::parse_hex_addr(&input).filter(|value| *value <= 0xFF),
    ) {
        (Some((addr, _)), Some(value)) => {
            // Re-select first: the drain prepares fresh state
            // (old byte) before staging, so Write also works
            // after a cancel (which empties the transaction).
            // The confirm row comes back authoritatively from
            // the drain (same as GTK): no optimistic text.
            bridge.push(DebugAction::SelectRow(addr));
            bridge.push(DebugAction::EditStage(value as u64));
        }
        (None, _) => bridge.set_status("write needs a selection".to_string()),
        _ => bridge.set_status(format!("parse failed: {input}")),
    }
}

/// Register write: the target rides in bridge state (picked by row
/// click), the value parses as a full-width hex number; the backend
/// masks to the register width.
fn reg_set(bridge: &DebugBridge) {
    let input = bridge.reg_input.lock().unwrap().trim().to_string();
    let target = *bridge.reg_target.lock().unwrap();
    match (target, vm::parse_hex_addr(&input)) {
        (Some(_), Some(value)) => {
            bridge.push(DebugAction::RegisterWrite(u64::from(value)));
        }
        (None, _) => bridge.set_status("register write needs a register".to_string()),
        _ => bridge.set_status(format!("parse failed: {input}")),
    }
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
            DebugMessage::RefreshPressed => bridge.push(DebugAction::Refresh),
            DebugMessage::SpacePrev => cycle_space(bridge, -1),
            DebugMessage::SpaceNext => cycle_space(bridge, 1),
            DebugMessage::TogglePausePressed => bridge.push(DebugAction::TogglePause),
            DebugMessage::StepFramePressed => bridge.push(DebugAction::StepFrame),
            DebugMessage::StepInstrPressed => bridge.push(DebugAction::StepInstr),
            DebugMessage::PpuOpenPressed => {
                bridge.open_ppu.store(true, Ordering::Release);
                bridge.push(DebugAction::Refresh);
            }
            DebugMessage::MemInputChanged(text) => {
                bridge.nav.lock().unwrap().set_mem_input(text);
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::MemGo => mem_go(bridge),
            DebugMessage::MemPage(delta) => mem_page(bridge, delta),
            DebugMessage::DisInputChanged(text) => {
                bridge.nav.lock().unwrap().set_dis_input(text);
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::DisGo => dis_go(bridge),
            DebugMessage::FollowPcToggle => {
                bridge.nav.lock().unwrap().follow();
                bridge.push(DebugAction::Refresh);
            }
            DebugMessage::DisLineClicked(addr) => navigate_to(bridge, addr),
            DebugMessage::FollowTarget(addr) => navigate_to(bridge, addr),
            DebugMessage::DisBack => dis_back(bridge),
            DebugMessage::BookmarkInputChanged(text) => {
                *bridge.bm_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::BookmarkAdd => bookmark_add(bridge),
            DebugMessage::BookmarkJump(addr) => navigate_to(bridge, addr),
            DebugMessage::BookmarkDelete(addr) => {
                bridge.bookmarks.lock().unwrap().retain(|(a, _)| *a != addr);
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::WatchSet => watch_set(bridge),
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
                let input = bridge.freeze_input.lock().unwrap().clone();
                apply_freeze(bridge, &input);
            }
            DebugMessage::EditFreezeSet => {
                let input = bridge.edit_input.lock().unwrap().trim().to_string();
                apply_freeze(bridge, &input);
            }
            DebugMessage::FreezeClear => {
                *bridge.freeze.lock().unwrap() = None;
                bridge.set_status("freeze cleared".to_string());
            }
            DebugMessage::RowSelected(addr) => {
                bridge.push(DebugAction::SelectRow(addr));
            }
            DebugMessage::EditInputChanged(text) => {
                *bridge.edit_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::EditWrite => edit_write(bridge),
            DebugMessage::ConfirmWrite => {
                bridge.push(DebugAction::WriteConfirm);
                // Commit consumes the transaction; the input clears.
                *bridge.edit_input.lock().unwrap() = String::new();
            }
            DebugMessage::RegSelected(name) => {
                *bridge.reg_target.lock().unwrap() = Some(name);
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::RegInputChanged(text) => {
                *bridge.reg_input.lock().unwrap() = text;
                bridge.view_invalidated.store(true, Ordering::Release);
            }
            DebugMessage::RegSet => reg_set(bridge),
            DebugMessage::CancelWrite => {
                bridge.push(DebugAction::EditCancel);
                bridge.display.lock().unwrap().pending_text = None;
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
        use iced::widget::{button, column, row, scrollable, text};
        // Single coherent capture per rebuild; widgets read locals only.
        // Images stay out: the debugger view never renders them.
        let bridge = &state.bridge;
        let language = bridge.language;
        let nav = bridge.nav.lock().unwrap().clone();
        let display = bridge.display.lock().unwrap().clone_without_images();
        let bookmarks = bridge.bookmarks.lock().unwrap().clone();
        let bm_input = bridge.bm_input.lock().unwrap().clone();
        let watch = *bridge.watch.lock().unwrap();
        let freeze = *bridge.freeze.lock().unwrap();
        let freeze_input = bridge.freeze_input.lock().unwrap().clone();
        let selected = *bridge.selected.lock().unwrap();
        let edit_input = bridge.edit_input.lock().unwrap().clone();
        let pending_text = display.pending_text;
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
        let paused = display.paused;
        let anchor = bridge.anchor();
        // Button hierarchy: one primary per flow (the Phase E
        // Confirm), secondary for toolbar/nav, text for list rows
        // and escape actions.
        let toolbar = view_toolbar(paused, language);
        // Settings form language: fixed label column + stepper.
        let space_row = view_space_row(selected_name.as_deref().unwrap_or(""), language);
        let mem_nav = view_mem_nav(&mem_input, language);
        let dump_col = view_dump_col(&dump_rows, &diff, selected);
        let reg_target = *bridge.reg_target.lock().unwrap();
        let reg_input = bridge.reg_input.lock().unwrap().clone();
        let regs_view =
            view_regs_section(&regs, &display.reg_names, reg_target, &reg_input, language);
        // Two-phase write: select a dump row, type the new byte,
        // Write stages it, and the confirm row commits or cancels.
        let edit_row = view_edit_row(selected, &edit_input, language);
        // Confirm row: the one primary button in the window; Cancel is
        // a text escape. The drain owns the authoritative text.
        let confirm_row = view_confirm_row(pending_text, language);
        let dis_nav = view_dis_nav(follow_pc, back_len, anchor, &dis_input, language);
        // Per-line disassembly: line click pins the anchor, the PC row
        // highlights, and structured targets offer follow buttons.
        let dis_col = view_dis_col(&disasm_lines, language);
        // Cross-emulator canon: execution above disassembly on the
        // left, state on the right.
        let left_col = column![
            text(ui_text(language, UiText::Debugger)).size(18),
            toolbar,
            dis_nav,
            dis_col
        ]
        .spacing(12)
        .width(Length::Fill);
        let right_col = view_right_col(
            status,
            regs_view,
            watch,
            watch_value,
            freeze,
            &freeze_input,
            &bm_input,
            &bookmarks,
            language,
        );
        // Tall state (GBA: 17 registers) scrolls inside the panes
        // instead of pushing Bookmarks and Memory out of view. The
        // panes row takes the remaining window height so the scroll
        // viewport is bounded; Refresh stays pinned at the bottom.
        let right_scroll = scrollable(right_col).height(Length::Fill);
        let content = column![
            row![left_col, right_scroll]
                .spacing(16)
                .height(Length::Fill),
            text(ui_text(language, UiText::Memory)).size(16),
            space_row,
            mem_nav,
            dump_col,
            edit_row,
        ]
        .spacing(10)
        .padding(16)
        .width(Length::Fill)
        .height(Length::Fill);
        let mut content = content;
        if let Some(confirm) = confirm_row {
            content = content.push(confirm);
        }
        let content = content.push(
            button(text(ui_text(language, UiText::Refresh)))
                .style(button::secondary)
                .width(Length::Fill)
                .on_press(DebugMessage::RefreshPressed),
        );
        content.into()
    }
}

/// One pane builders: each renders a single debugger section from
/// plain locals. `view` stays a capture-and-assemble pass.
///
/// The toolbar carries a single Pause/Resume toggle (not two
/// buttons): the label follows the drained pause state.
fn view_toolbar(
    paused: bool,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text};
    row![
        button(text(if paused {
            ui_text(language, UiText::Resume)
        } else {
            ui_text(language, UiText::Pause)
        }))
        .style(button::secondary)
        .on_press(DebugMessage::TogglePausePressed),
        button(text(ui_text(language, UiText::StepFrame)))
            .style(button::secondary)
            .on_press(DebugMessage::StepFramePressed),
        button(text(ui_text(language, UiText::StepInstr)))
            .style(button::secondary)
            .on_press(DebugMessage::StepInstrPressed),
        button(text(ui_text(language, UiText::PpuViewer)))
            .style(button::secondary)
            .on_press(DebugMessage::PpuOpenPressed),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

fn view_space_row(
    selected_name: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, row, text};
    row![
        text(ui_text(language, UiText::Space)).width(Length::Fixed(150.0)),
        button(text(ui_text(language, UiText::Prev)))
            .style(button::secondary)
            .on_press(DebugMessage::SpacePrev),
        text(selected_name.to_owned()).size(14).width(Length::Fill),
        button(text(ui_text(language, UiText::Next)))
            .style(button::secondary)
            .on_press(DebugMessage::SpaceNext),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

fn view_mem_nav(
    mem_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, row, text, text_input};
    row![
        text(ui_text(language, UiText::Page)).width(Length::Fixed(150.0)),
        button(text(ui_text(language, UiText::Prev)))
            .style(button::secondary)
            .on_press(DebugMessage::MemPage(-1)),
        button(text(ui_text(language, UiText::Next)))
            .style(button::secondary)
            .on_press(DebugMessage::MemPage(1)),
        text_input(ui_text(language, UiText::AddrHex), mem_input)
            .on_input(DebugMessage::MemInputChanged)
            .on_submit(DebugMessage::MemGo)
            .width(120),
        button(text(ui_text(language, UiText::Go)))
            .style(button::secondary)
            .on_press(DebugMessage::MemGo),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Dump rows are text-style buttons; selection inverts to
/// secondary. Changed rows carry a `*` mark (theme-safe, no
/// new colors). Selection wins the secondary style.
///
/// The list rides in a scrollable that fills its share: window
/// growth lands in the viewport instead of stranding dead space,
/// and shrunken windows scroll natively. (Wheel paging is a GTK
/// affordance: iced reports scroll position, not wheel deltas, so
/// an exact-fit page cannot tell the direction.)
fn view_dump_col(
    dump_rows: &[(u32, String)],
    diff: &[u32],
    selected: Option<(u32, Option<u8>)>,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, scrollable, text};
    let mut dump_col = column![].spacing(2);
    for (addr, line) in dump_rows {
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
    scrollable(dump_col).height(Length::Fill).into()
}

/// Registers flow into two clickable columns so tall lists (GBA:
/// 17) cost half the height. Pure text split, no system branches.
/// Row click picks the write target (highlighted); the Set row
/// below writes the parsed hex value through the drain. Names come
/// from the snapshot, never scraped from text.
fn view_regs_section(
    regs: &str,
    reg_names: &[&'static str],
    reg_target: Option<&'static str>,
    reg_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, text, text_input};
    let lines: Vec<&str> = regs.lines().collect();
    let reg_mid = lines.len().div_ceil(2);
    let mid = reg_mid.min(lines.len());
    let mut halves_row = row![].spacing(16).width(Length::Fill);
    for (base, half) in [(0, &lines[..mid]), (mid, &lines[mid..])] {
        let mut col = column![].spacing(2);
        for (i, line) in half.iter().enumerate() {
            let name = reg_names.get(base + i).copied();
            let row_button = button(
                text((*line).to_owned())
                    .size(14)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 8])
            .width(Length::Fill);
            col = col.push(match name {
                Some(target) => row_button
                    .style(if Some(target) == reg_target {
                        button::secondary
                    } else {
                        button::text
                    })
                    .on_press(DebugMessage::RegSelected(target)),
                // Unnamed degenerate lines stay plain clickable-free text.
                None => row_button.style(button::text),
            });
        }
        halves_row = halves_row.push(col);
    }
    let target_text = match reg_target {
        Some(name) => format!("{} {name}", ui_text(language, UiText::Set)),
        None => ui_text(language, UiText::PickRegister).to_string(),
    };
    let edit = row![
        text(target_text).size(14).width(Length::Fixed(140.0)),
        text_input(ui_text(language, UiText::Hex), reg_input)
            .on_input(DebugMessage::RegInputChanged)
            .on_submit(DebugMessage::RegSet)
            .width(100),
        button(text(ui_text(language, UiText::Set)))
            .style(button::secondary)
            .on_press(DebugMessage::RegSet),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center);
    column![halves_row, edit].spacing(4).into()
}

fn view_edit_row(
    selected: Option<(u32, Option<u8>)>,
    edit_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, row, text, text_input};
    let select_text = match selected {
        Some((addr, Some(old))) => format!("Selected {addr:08X} (was {old:02X})"),
        Some((addr, None)) => format!("Selected {addr:08X} (was ??)"),
        None => ui_text(language, UiText::SelectDumpRow).to_string(),
    };
    // The target rides on the button: writes always land on the
    // selected row, never on the page anchor.
    let write_label = match selected {
        Some((addr, _)) => format!("{} {addr:08X}", ui_text(language, UiText::Write)),
        None => ui_text(language, UiText::Write).to_string(),
    };
    row![
        text(select_text).size(14).width(Length::Fixed(240.0)),
        text_input(ui_text(language, UiText::HexByte), edit_input)
            .on_input(DebugMessage::EditInputChanged)
            .width(100),
        button(text(write_label))
            .style(button::secondary)
            .on_press(DebugMessage::EditWrite),
        button(text(ui_text(language, UiText::Watch)))
            .style(button::secondary)
            .on_press(DebugMessage::WatchSet),
        button(text(ui_text(language, UiText::Freeze)))
            .style(button::secondary)
            .on_press(DebugMessage::EditFreezeSet),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

fn view_confirm_row(
    pending_text: Option<String>,
    language: AppLanguage,
) -> Option<iced::Element<'static, DebugMessage, iced::Theme, Renderer>> {
    use iced::Length;
    use iced::widget::{button, row, text};
    pending_text.map(|pending| {
        row![
            text(pending).size(14).width(Length::Fill),
            button(text(ui_text(language, UiText::Confirm)))
                .style(button::primary)
                .on_press(DebugMessage::ConfirmWrite),
            button(text(ui_text(language, UiText::Cancel)))
                .style(button::text)
                .on_press(DebugMessage::CancelWrite),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center)
        .into()
    })
}
fn view_dis_nav(
    follow_pc: bool,
    back_len: usize,
    anchor: u32,
    dis_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::widget::{button, row, text, text_input};
    let follow_button =
        button(text(ui_text(language, UiText::FollowPc))).on_press(DebugMessage::FollowPcToggle);
    let back_button = button(text(format!(
        "{} ({back_len})",
        ui_text(language, UiText::Back)
    )))
    .on_press(DebugMessage::DisBack);
    // The title names the anchor: clicks pin it visibly instead of
    // silently moving the window.
    let title = if follow_pc {
        format!(
            "{} {}",
            ui_text(language, UiText::Disassembly),
            ui_text(language, UiText::FollowPcTag)
        )
    } else {
        format!(
            "{} @{anchor:08X} {}",
            ui_text(language, UiText::Disassembly),
            ui_text(language, UiText::FixedTag)
        )
    };
    row![
        text(title),
        if follow_pc {
            follow_button.style(button::secondary)
        } else {
            follow_button.style(button::text)
        },
        text_input(ui_text(language, UiText::AddrHex), dis_input)
            .on_input(DebugMessage::DisInputChanged)
            .on_submit(DebugMessage::DisGo)
            .width(100),
        button(text(ui_text(language, UiText::Go)))
            .style(button::secondary)
            .on_press(DebugMessage::DisGo),
        if back_len > 0 {
            back_button.style(button::secondary)
        } else {
            back_button.style(button::text)
        },
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

fn view_dis_col(
    disasm_lines: &[nerust_core_traits::debugger::DisasmLine],
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, scrollable, text};
    let mut dis_col = column![].spacing(2);
    if disasm_lines.is_empty() {
        dis_col = dis_col.push(text(ui_text(language, UiText::NoDisassembly)).size(14));
        dis_col = dis_col.push(text(ui_text(language, UiText::DisassemblyHint)).size(14));
    }
    for line in disasm_lines {
        // The PC row carries a `>` marker like the dump selection:
        // the current position reads at a glance.
        let marker = if line.is_pc { "> " } else { "  " };
        let line_button = button(
            text(format!("{marker}{}", vm::format_disasm_line(line)))
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
    scrollable(dis_col).height(Length::Fill).into()
}

#[allow(clippy::too_many_arguments)]
fn view_right_col(
    status: String,
    regs_view: iced::Element<'static, DebugMessage, iced::Theme, Renderer>,
    watch: Option<u32>,
    watch_value: Option<u8>,
    freeze: Option<(u32, u8)>,
    freeze_input: &str,
    bm_input: &str,
    bookmarks: &[(u32, String)],
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, text, text_input};
    let watch_text = match (watch, watch_value) {
        (Some(addr), Some(value)) => format!("watch {addr:08X} = {value:02X}"),
        (Some(addr), None) => format!("watch {addr:08X} = ??"),
        (None, _) => ui_text(language, UiText::WatchNone).to_string(),
    };
    let freeze_text = match freeze {
        Some((addr, value)) => format!("frozen {addr:08X}={value:02X}"),
        None => ui_text(language, UiText::FreezeNone).to_string(),
    };
    let mut bookmarks_col = column![].spacing(2);
    for (addr, label) in bookmarks {
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
                    .padding([4, 10])
                    .style(button::text)
                    .on_press(DebugMessage::BookmarkDelete(*addr)),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
        );
    }
    column![
        text(status).size(14).width(Length::Fill),
        text(ui_text(language, UiText::Registers)).size(16),
        regs_view,
        text(ui_text(language, UiText::Watch)).size(16),
        row![
            button(text(ui_text(language, UiText::Watch)))
                .style(button::secondary)
                .on_press(DebugMessage::WatchSet),
            text(watch_text).size(14).width(Length::Fill),
            button(text(ui_text(language, UiText::Clear)))
                .style(button::text)
                .on_press(DebugMessage::WatchClear),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
        text(ui_text(language, UiText::Freeze)).size(16),
        row![
            text_input(ui_text(language, UiText::HexByte), freeze_input)
                .on_input(DebugMessage::FreezeInputChanged)
                .width(100),
            button(text(ui_text(language, UiText::Freeze)))
                .style(button::secondary)
                .on_press(DebugMessage::FreezeSet),
            text(freeze_text).size(14).width(Length::Fill),
            button(text(ui_text(language, UiText::Clear)))
                .style(button::text)
                .on_press(DebugMessage::FreezeClear),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
        text(ui_text(language, UiText::Bookmarks)).size(16),
        row![
            text_input(ui_text(language, UiText::Label), bm_input)
                .on_input(DebugMessage::BookmarkInputChanged)
                .on_submit(DebugMessage::BookmarkAdd)
                .width(Length::Fill),
            button(text(ui_text(language, UiText::Add)))
                .style(button::secondary)
                .on_press(DebugMessage::BookmarkAdd),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
        bookmarks_col,
    ]
    .spacing(12)
    .width(Length::Fixed(STATE_COLUMN_WIDTH))
    .into()
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
        language: AppLanguage,
        event_loop: &EventLoopWindowTarget<crate::app_menu::UserEvent>,
        position: Option<(i32, i32)>,
    ) -> Option<Self> {
        let should_close = Arc::new(AtomicBool::new(false));
        let bridge = Arc::new(DebugBridge::new(display, spaces, mem_addr, language));
        let mut builder = WindowBuilder::new()
            .with_title(ui_text(language, UiText::Debugger))
            .with_inner_size(tao::dpi::LogicalSize::new(
                DEBUG_WINDOW_SIZE.0,
                DEBUG_WINDOW_SIZE.1,
            ));
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

    pub(crate) fn take_requests(&self) -> Vec<DebugAction> {
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
                Physical::Code(Code::F6) => Some(DebugAction::StepFrame),
                Physical::Code(Code::F7) => Some(DebugAction::StepInstr),
                Physical::Code(Code::F9) => Some(DebugAction::TogglePause),
                _ => None,
            };
            if let Some(request) = request {
                self.bridge.push(request);
            }
        }
        let mut messages = Vec::new();
        // Cursor keys page the dump (Up = toward lower addresses).
        // Left/Right stay with the widgets: hijacking them would break
        // caret movement in every text input. Computed before `mapped`
        // moves into the UI update below.
        let page = if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            physical_key,
            repeat: false,
            ..
        }) = &mapped
        {
            use iced::keyboard::key::{Code, Physical};
            match *physical_key {
                Physical::Code(Code::ArrowUp) => Some(DebugMessage::MemPage(-1)),
                Physical::Code(Code::ArrowDown) => Some(DebugMessage::MemPage(1)),
                _ => None,
            }
        } else {
            None
        };
        self.ui_state.ui_mut().update(
            &[mapped],
            self.cursor,
            &mut self.renderer.backend,
            &mut self.clipboard,
            &mut messages,
        );
        messages.extend(page);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn freeze_bridge() -> Arc<DebugBridge> {
        let bridge = Arc::new(DebugBridge::new(
            DisplayCache::default(),
            Vec::new(),
            0,
            AppLanguage::English,
        ));
        *bridge.selected.lock().unwrap() = Some((0x0200_0010, Some(0x00)));
        *bridge.freeze_input.lock().unwrap() = "FF".to_string();
        *bridge.edit_input.lock().unwrap() = "AA".to_string();
        bridge
    }

    fn drive(bridge: &Arc<DebugBridge>, message: DebugMessage) {
        let program = DebugProgram {
            bridge: Arc::clone(bridge),
        };
        let mut state = DebugState {
            bridge: Arc::clone(bridge),
        };
        let _ = program.update(&mut state, message);
    }

    #[test]
    fn edit_freeze_set_reads_edit_input() {
        let bridge = freeze_bridge();
        drive(&bridge, DebugMessage::EditFreezeSet);
        assert_eq!(*bridge.freeze.lock().unwrap(), Some((0x0200_0010, 0xAA)));
        assert_eq!(bridge.display.lock().unwrap().status, "frozen 02000010=AA");
    }

    #[test]
    fn freeze_set_reads_top_input() {
        let bridge = freeze_bridge();
        drive(&bridge, DebugMessage::FreezeSet);
        assert_eq!(*bridge.freeze.lock().unwrap(), Some((0x0200_0010, 0xFF)));
        assert_eq!(bridge.display.lock().unwrap().status, "frozen 02000010=FF");
    }
}
