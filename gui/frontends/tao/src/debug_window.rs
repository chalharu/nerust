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

use iced::Task;
use iced_tiny_skia::Renderer;
use iced_winit::program::{self, Program};
use nerust_core_traits::debugger::{DEBUG_DUMP_PAGE_ROWS, DUMP_ROW_BYTES, HexRow, SpaceId};
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{self as vm, DisplayCache, NavState, StatusMessage};
use nerust_settings_core::i18n::{self as ui_msg, UiText, text as ui_text};
use nerust_settings_core::model::language::AppLanguage;
use tao::event_loop::EventLoopWindowTarget;

use crate::iced_shell::{WindowScaffold, aux_settings, aux_theme};

/// Initial debugger window size (verified Xvfb fit).
pub(crate) const DEBUG_WINDOW_SIZE: (f64, f64) = (1100.0, 800.0);
/// Minimum debugger size: below this the fixed label columns and
/// the register halves collide instead of reflowing.
pub(crate) const DEBUG_WINDOW_MIN_SIZE: (f64, f64) = (900.0, 650.0);
/// Right state column width.
const STATE_COLUMN_WIDTH: f32 = 340.0;
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
        // Frontend-local wording (parse errors, prompts): stored
        // verbatim. Drain outcomes arrive structured and never pass
        // through here.
        self.display.lock().unwrap().status = StatusMessage::Local(status);
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
    let language = bridge.language;
    let selected = bridge.selected.lock().unwrap().map(|(addr, _)| addr);
    match (selected, vm::parse_hex_addr(input).filter(|v| *v <= 0xFF)) {
        (Some(addr), Some(value)) => {
            *bridge.freeze.lock().unwrap() = Some((addr, value as u8));
            bridge.set_status(ui_msg::frozen(language, addr, value as u8));
            bridge.push(DebugAction::Refresh);
        }
        _ => bridge.set_status(ui_text(language, UiText::FreezeNeedsSelection).to_string()),
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
    let language = bridge.language;
    let input = bridge.nav.lock().unwrap().mem_input().to_string();
    match vm::parse_hex_addr(&input) {
        Some(addr) => {
            bridge.nav.lock().unwrap().go_mem(addr);
            bridge.push(DebugAction::MemNav);
        }
        None => bridge.set_status(ui_msg::parse_failed(language, &input)),
    }
}

fn mem_page(bridge: &DebugBridge, delta: i32) {
    bridge
        .nav
        .lock()
        .unwrap()
        .page_mem(delta * DEBUG_DUMP_PAGE_ROWS as i32 * DUMP_ROW_BYTES as i32);
    bridge.push(DebugAction::MemNav);
}

fn dis_go(bridge: &DebugBridge) {
    let language = bridge.language;
    let input = bridge.nav.lock().unwrap().dis_input().to_string();
    match vm::parse_hex_addr(&input) {
        Some(addr) => navigate_to(bridge, addr),
        None => bridge.set_status(ui_msg::parse_failed(language, &input)),
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
        None => bridge.set_status(ui_text(bridge.language, UiText::BackEmpty).to_string()),
    }
}

fn bookmark_add(bridge: &DebugBridge) {
    let label = bridge.bm_input.lock().unwrap().trim().to_string();
    // Anchor on the PC row when following, else the pin. The PC
    // row is snapshotted first and the guard dropped before
    // `anchor()` runs: it locks nav→display, so nesting it under
    // the display guard would invert the order and risk deadlock.
    let pc_addr = {
        bridge
            .display
            .lock()
            .unwrap()
            .disasm_lines
            .iter()
            .find(|line| line.is_pc)
            .map(|line| line.addr)
    };
    let addr = pc_addr.unwrap_or_else(|| bridge.anchor());
    let mut marks = bridge.bookmarks.lock().unwrap();
    if marks.len() >= BOOKMARK_CAP {
        bridge.set_status(ui_text(bridge.language, UiText::BookmarkFull).to_string());
        return;
    }
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
        None => {
            bridge.set_status(ui_text(bridge.language, UiText::WatchNeedsSelection).to_string())
        }
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
        (None, _) => {
            bridge.set_status(ui_text(bridge.language, UiText::WriteNeedsSelection).to_string())
        }
        _ => bridge.set_status(ui_msg::parse_failed(bridge.language, &input)),
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
        (None, _) => {
            bridge.set_status(ui_text(bridge.language, UiText::RegisterNeedsSelection).to_string())
        }
        _ => bridge.set_status(ui_msg::parse_failed(bridge.language, &input)),
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
        aux_theme()
    }

    fn settings(&self) -> iced::Settings {
        aux_settings()
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
                bridge.set_status(ui_text(bridge.language, UiText::FreezeCleared).to_string());
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
                bridge.display.lock().unwrap().pending = None;
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
        use iced::widget::lazy;
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
        let pending = display.pending;
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
        // Coreless windows never drain, so the default Empty status
        // would sit blank forever and the toggle would offer Pause
        // against nothing. Read coreless as paused with an explicit
        // "no core" status — the same agreement as the gtk twin.
        let coreless = bridge.spaces.is_empty();
        let paused = paused || coreless;
        let status = if coreless && matches!(status, vm::StatusMessage::Empty) {
            vm::StatusMessage::NoCore
        } else {
            status
        };
        // Same rule as `anchor()`, computed from the snapshot above
        // instead of re-locking nav and display for a second time.
        let anchor = if follow_pc {
            disasm_lines.first().map(|line| line.addr).unwrap_or(0)
        } else {
            nav.dis_addr().unwrap_or(0)
        };
        // Button hierarchy: one primary per flow (the Phase E
        // Confirm), secondary for toolbar/nav, text for list rows
        // and escape actions.
        let toolbar = view_toolbar(paused, language);
        // Settings form language: fixed label column + stepper.
        let space_row = view_space_row(selected_name.as_deref().unwrap_or(""), language);
        let mem_nav = view_mem_nav(&mem_input, language);
        // The three heavy lists ride `lazy`: keystrokes elsewhere
        // (mem/dis/bookmark/freeze/edit inputs) leave their
        // dependencies untouched, so the ~600-widget rebuild collapses
        // to the header rows. Every visual input lives in the
        // dependency tuple — a forgotten field would freeze its
        // highlight, so keep the tuples exhaustive.
        let dump_col = lazy((dump_rows, diff, selected), |(rows, diff, selected)| {
            view_dump_col(rows, diff, *selected)
        });
        let reg_target = *bridge.reg_target.lock().unwrap();
        let reg_input = bridge.reg_input.lock().unwrap().clone();
        let regs_view = lazy(
            (regs, reg_target, reg_input, language),
            |(regs, target, input, language)| view_regs_section(regs, *target, input, *language),
        )
        .into();
        // Two-phase write: select a dump row, type the new byte,
        // Write stages it, and the confirm row commits or cancels.
        let edit_row = view_edit_row(selected, &edit_input, language);
        // Confirm row: the one primary button in the window; Cancel is
        // a text escape. The drain owns the authoritative text.
        let confirm_row = view_confirm_row(pending, language);
        let dis_nav = view_dis_nav(follow_pc, back_len, anchor, &dis_input, language);
        // Per-line disassembly: line click pins the anchor (the
        // pinned row carries `@`, the PC row `>`), and structured
        // targets offer follow buttons. In follow-PC mode nothing
        // is pinned, so no `@` shows.
        let pinned = if follow_pc { None } else { Some(anchor) };
        let dis_col = lazy(
            (disasm_lines, pinned, language),
            |(lines, pinned, language)| view_dis_col(lines, *pinned, *language),
        );
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
            // Wide enough for the longest placeholder in either
            // language (JA "アドレス16進" clips at 120).
            .width(150),
        button(text(ui_text(language, UiText::Go)))
            .style(button::secondary)
            .on_press(DebugMessage::MemGo),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Dump rows are text-style buttons; selection inverts, changed rows
/// carry a `*` mark. The list fills its share in a scrollable;
/// shrunken windows scroll natively.
fn view_dump_col(
    dump_rows: &[HexRow],
    diff: &[u32],
    selected: Option<(u32, Option<u8>)>,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, scrollable, text};
    // Byte-precise selection: every byte is its own button, so the
    // write target reads exactly (the address head keeps the old
    // row-base pick). The drain, transaction, and commit path were
    // already byte-exact; only the click granularity was row-wide.
    let mut dump_col = column![].spacing(2);
    for hex_row in dump_rows {
        let selected_addr = selected.map(|(a, _)| a);
        // Containing-row match: a mid-row pick still lights its row.
        let row_hit = selected_addr
            .is_some_and(|a| a >= hex_row.addr && a < hex_row.addr + u32::from(hex_row.valid));
        let changed = diff.contains(&hex_row.addr);
        let marker = match (row_hit, changed) {
            (true, true) => ">*",
            (true, false) => "> ",
            (false, true) => " *",
            (false, false) => "  ",
        };
        let mut line_row = row![
            button(
                text(format!("{marker}0x{:08X}:", hex_row.addr))
                    .size(14)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 4])
            .style(if row_hit {
                button::secondary
            } else {
                button::text
            })
            .on_press(DebugMessage::RowSelected(hex_row.addr)),
        ]
        .spacing(2);
        for i in 0..hex_row.valid {
            let addr = hex_row.addr + u32::from(i);
            let byte = hex_row.bytes[usize::from(i)];
            let is_selected = selected_addr == Some(addr);
            line_row = line_row.push(
                button(
                    text(format!("{byte:02X}"))
                        .size(14)
                        .font(iced::Font::MONOSPACE),
                )
                .padding([2, 4])
                .style(if is_selected {
                    button::secondary
                } else {
                    button::text
                })
                .on_press(DebugMessage::RowSelected(addr)),
            );
        }
        dump_col = dump_col.push(line_row.width(Length::Fill));
    }
    scrollable(dump_col).height(Length::Fill).into()
}

/// Registers flow into two clickable columns so tall lists (GBA:
/// 17) cost half the height. Pure text split, no system branches.
/// Row click picks the write target (highlighted); the Set row
/// below writes the parsed hex value through the drain. Lines are
/// formatted here from the structured snapshot, never scraped.
fn view_regs_section(
    regs: &[(&'static str, u64)],
    reg_target: Option<&'static str>,
    reg_input: &str,
    language: AppLanguage,
) -> iced::Element<'static, DebugMessage, iced::Theme, Renderer> {
    use iced::Length;
    use iced::widget::{button, column, row, text, text_input};
    let mid = regs.len().div_ceil(2).min(regs.len());
    let mut halves_row = row![].spacing(16).width(Length::Fill);
    for half in [&regs[..mid], &regs[mid..]] {
        let mut col = column![].spacing(2);
        for (name, value) in half.iter() {
            let row_button = button(
                text(vm::format_register(name, *value))
                    .size(14)
                    .font(iced::Font::MONOSPACE),
            )
            .padding([2, 8])
            .width(Length::Fill);
            col = col.push(
                row_button
                    .style(if Some(*name) == reg_target {
                        button::secondary
                    } else {
                        button::text
                    })
                    .on_press(DebugMessage::RegSelected(name)),
            );
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
    // The target rides on the label and the button: writes always
    // land on the selected row, never on the page anchor.
    let select_text = match selected {
        Some((addr, old)) => ui_msg::selected_row(language, addr, old),
        None => ui_text(language, UiText::SelectDumpRow).to_string(),
    };
    // The target rides on the button: writes always land on the
    // selected row, never on the page anchor.
    let write_label = match selected {
        Some((addr, _)) => format!("{} 0x{addr:08X}", ui_text(language, UiText::Write)),
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
    pending: Option<vm::PendingWrite>,
    language: AppLanguage,
) -> Option<iced::Element<'static, DebugMessage, iced::Theme, Renderer>> {
    use iced::Length;
    use iced::widget::{button, row, text};
    pending.map(|pending| {
        row![
            text(pending.render(language)).size(14).width(Length::Fill),
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
            "{} @0x{anchor:08X} {}",
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
            // Same JA placeholder width as the memory input above.
            .width(150),
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
    pinned: Option<u32>,
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
        // Marker column is two chars wide for every row. The PC `>`
        // already comes from `format_disasm_line`, so the PC row
        // contributes blanks here; the pinned anchor row carries `@`.
        // (Prepending another `"> "` double-marked the PC row.)
        let is_pinned = pinned == Some(line.addr);
        let marker = disasm_marker(line.is_pc, is_pinned);
        let line_button = button(
            text(format!("{marker}{}", vm::format_disasm_line(line)))
                .size(14)
                .font(iced::Font::MONOSPACE),
        )
        .padding([2, 8])
        .width(Length::Fill)
        .on_press(DebugMessage::DisLineClicked(line.addr));
        let line_button = if line.is_pc || is_pinned {
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
    status: StatusMessage,
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
        (Some(addr), Some(value)) => ui_msg::watch_value(language, addr, Some(value)),
        (Some(addr), None) => ui_msg::watch_value(language, addr, None),
        (None, _) => ui_text(language, UiText::WatchNone).to_string(),
    };
    let freeze_text = match freeze {
        Some((addr, value)) => ui_msg::frozen(language, addr, value),
        None => ui_text(language, UiText::FreezeNone).to_string(),
    };
    let mut bookmarks_col = column![].spacing(2);
    for (addr, label) in bookmarks {
        bookmarks_col = bookmarks_col.push(
            row![
                button(
                    text(format!("{label} 0x{addr:08X}"))
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
        text(status.render(language)).size(14).width(Length::Fill),
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
    // Right breathing room for the overlay scrollbar: without it the
    // rail sits on top of the register text (tall GBA lists scroll).
    .padding(iced::Padding {
        right: 16.0,
        ..Default::default()
    })
    .into()
}

pub(crate) struct DebugWindowHandle {
    pub(crate) shell: crate::iced_shell::AuxShell<DebugProgram>,
    pub(crate) bridge: Arc<DebugBridge>,
}

/// Window-scoped shortcuts on physical codes: F6 steps a frame,
/// F7 an instruction, F9 toggles pause. Pure mapping shared by the
/// event handler and the tests below.
fn shortcut_request(physical: &iced::keyboard::key::Physical) -> Option<DebugAction> {
    use iced::keyboard::key::{Code, Physical};
    match *physical {
        Physical::Code(Code::F6) => Some(DebugAction::StepFrame),
        Physical::Code(Code::F7) => Some(DebugAction::StepInstr),
        Physical::Code(Code::F9) => Some(DebugAction::TogglePause),
        _ => None,
    }
}

/// Two-char marker prefix for a disassembly row, kept out of the
/// view so the width contract stays unit-tested. The PC `>` itself
/// lives in [`vm::format_disasm_line`]; this only adds the pinned
/// `@` (or blanks), so composing both never double-marks.
fn disasm_marker(is_pc: bool, is_pinned: bool) -> &'static str {
    if !is_pc && is_pinned { "@ " } else { "  " }
}

/// Cursor keys page the dump (Up = toward lower addresses). Pure
/// mapping; Left/Right stay `None` for the text inputs.
fn cursor_page(physical: &iced::keyboard::key::Physical) -> Option<DebugMessage> {
    use iced::keyboard::key::{Code, Physical};
    match *physical {
        Physical::Code(Code::ArrowUp) => Some(DebugMessage::MemPage(-1)),
        Physical::Code(Code::ArrowDown) => Some(DebugMessage::MemPage(1)),
        _ => None,
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
        let bridge = Arc::new(DebugBridge::new(display, spaces, mem_addr, language));
        let title = ui_text(language, UiText::Debugger).to_string();
        let scaffold = WindowScaffold::open(
            event_loop,
            &title,
            DEBUG_WINDOW_SIZE,
            Some(DEBUG_WINDOW_MIN_SIZE),
            position,
            "debugger window",
        )?;
        let program = DebugProgram {
            bridge: Arc::clone(&bridge),
        };
        let (instance, _task) = program::Instance::new(program);
        let shell = crate::iced_shell::AuxShell::assemble(scaffold, instance);
        Some(Self { shell, bridge })
    }

    pub(crate) fn set_display(&mut self, display: DisplayCache) {
        *self.bridge.display.lock().unwrap() = display;
        self.sync_from_bridge();
    }

    pub(crate) fn take_requests(&self) -> Vec<DebugAction> {
        self.bridge.take_requests()
    }

    pub(crate) fn render(&mut self) {
        self.shell.render("debugger");
    }

    pub(crate) fn handle_event(&mut self, mapped: iced::Event) {
        // Window-scoped shortcuts on physical codes (F6 step frame,
        // F7 step instruction, F9 pause toggle). Slot switching owns
        // no F-keys; the debugger open rides the settings schema
        // (default F4), not a hardcoded key, so no conflict either.
        // NOTE: `physical_key` binds by reference through `&mapped`;
        // dereference before matching or no arm ever hits.
        if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            physical_key,
            repeat: false,
            ..
        }) = &mapped
            && let Some(request) = shortcut_request(physical_key)
        {
            self.bridge.push(request);
        }
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
            cursor_page(physical_key)
        } else {
            None
        };
        let (mut messages, outcome) = self.shell.ui_state.pump(
            mapped,
            self.shell.cursor,
            &mut self.shell.clipboard,
            &mut self.shell.renderer.backend,
        );
        messages.extend(page);
        match outcome {
            crate::iced_shell::PumpOutcome::Rebuild => {
                self.bridge.view_invalidated.store(true, Ordering::Release);
            }
            crate::iced_shell::PumpOutcome::Repaint => {
                self.shell.window.request_redraw();
            }
            crate::iced_shell::PumpOutcome::Idle => {}
        }
        if !messages.is_empty() || matches!(outcome, crate::iced_shell::PumpOutcome::Rebuild) {
            let bounds = self.shell.logical_bounds();
            self.shell.ui_state.process_messages(
                messages,
                self.shell.window_id,
                bounds,
                &mut self.shell.renderer.backend,
                &self.bridge.view_invalidated,
            );
            self.shell.window.request_redraw();
        }
    }

    pub(crate) fn handle_tao_event(&mut self, event: tao::event::WindowEvent) {
        if let Some(iced_event) = self.shell.feed_tao_event(event) {
            self.handle_event(iced_event);
        }
    }

    pub(crate) fn sync_from_bridge(&mut self) {
        self.bridge.view_invalidated.store(true, Ordering::Release);
        let bounds = self.shell.logical_bounds();
        self.shell.ui_state.process_messages(
            Vec::new(),
            self.shell.window_id,
            bounds,
            &mut self.shell.renderer.backend,
            &self.bridge.view_invalidated,
        );
        self.shell.window.request_redraw();
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
    fn input_echo_never_queues_drain_traffic() {
        // Typing must stay on the UI thread: input-echo messages
        // re-render locally and never queue emu-thread round-trips
        // (a drain would block typing on the thread reply).
        let bridge = full_bridge();
        for message in [
            DebugMessage::MemInputChanged("0x100".to_string()),
            DebugMessage::DisInputChanged("0x200".to_string()),
            DebugMessage::BookmarkInputChanged("tag".to_string()),
            DebugMessage::FreezeInputChanged("FF".to_string()),
            DebugMessage::EditInputChanged("42".to_string()),
            DebugMessage::RegInputChanged("7".to_string()),
            DebugMessage::RegSelected("r0"),
            DebugMessage::WatchClear,
            DebugMessage::BookmarkDelete(0x999),
        ] {
            drive(&bridge, message);
        }
        assert!(
            bridge.outbox.lock().unwrap().is_empty(),
            "typing never blocks on the emu thread"
        );
        assert!(bridge.view_invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn edit_freeze_set_reads_edit_input() {
        let bridge = freeze_bridge();
        drive(&bridge, DebugMessage::EditFreezeSet);
        assert_eq!(*bridge.freeze.lock().unwrap(), Some((0x0200_0010, 0xAA)));
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("frozen 0x02000010=AA".to_string())
        );
    }

    #[test]
    fn freeze_set_reads_top_input() {
        let bridge = freeze_bridge();
        drive(&bridge, DebugMessage::FreezeSet);
        assert_eq!(*bridge.freeze.lock().unwrap(), Some((0x0200_0010, 0xFF)));
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("frozen 0x02000010=FF".to_string())
        );
    }

    fn full_bridge() -> Arc<DebugBridge> {
        use nerust_core_traits::debugger::DisasmLine;
        let display = DisplayCache {
            disasm_lines: vec![
                DisasmLine {
                    addr: 0x0800_27D2,
                    bytes: [0xF0, 0xBC, 0, 0],
                    len: 2,
                    text: "POP {r4, r5, r6, r7}".to_string(),
                    is_pc: true,
                    target: None,
                },
                DisasmLine {
                    addr: 0x0800_27D2E,
                    bytes: [0x02, 0xBC, 0, 0],
                    len: 2,
                    text: "POP {r1}".to_string(),
                    is_pc: false,
                    target: Some(0x0800_27CA8),
                },
            ],
            regs: vec![("r0", 1), ("r1", 2)],
            reg_names: vec!["r0", "r1"],
            dump_rows: vec![
                HexRow {
                    addr: 0x100,
                    valid: 16,
                    bytes: [0xAB; 16],
                },
                HexRow {
                    addr: 0x110,
                    valid: 3,
                    bytes: [1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                },
            ],
            diff: vec![0x100],
            ..DisplayCache::default()
        };
        let bridge = Arc::new(DebugBridge::new(
            display,
            vec![(SpaceId(0), "Memory".to_string(), 0)],
            0,
            AppLanguage::English,
        ));
        *bridge.selected.lock().unwrap() = Some((0x100, Some(0xAB)));
        *bridge.edit_input.lock().unwrap() = "42".to_string();
        *bridge.reg_input.lock().unwrap() = "7".to_string();
        *bridge.freeze_input.lock().unwrap() = "FF".to_string();
        bridge
    }

    #[test]
    fn update_drives_toolbar_and_nav() {
        let bridge = full_bridge();
        for message in [
            DebugMessage::RefreshPressed,
            DebugMessage::TogglePausePressed,
            DebugMessage::StepFramePressed,
            DebugMessage::StepInstrPressed,
            DebugMessage::PpuOpenPressed,
            DebugMessage::MemPage(1),
            DebugMessage::MemPage(-1),
            DebugMessage::FollowPcToggle,
            DebugMessage::RowSelected(0x110),
        ] {
            drive(&bridge, message);
        }
        assert!(bridge.open_ppu.load(Ordering::Acquire));
        assert!(!bridge.outbox.lock().unwrap().is_empty());
        // Space stepper with one space cycles and clears selection.
        drive(&bridge, DebugMessage::SpaceNext);
        assert!(bridge.selected.lock().unwrap().is_none());
        drive(&bridge, DebugMessage::SpacePrev);
        // Memory Go resolves valid hex, rejects the rest.
        drive(&bridge, DebugMessage::MemInputChanged("100".to_string()));
        drive(&bridge, DebugMessage::MemGo);
        assert_eq!(bridge.nav.lock().unwrap().mem_addr(), 0x100);
        drive(&bridge, DebugMessage::MemInputChanged("zz".to_string()));
        drive(&bridge, DebugMessage::MemGo);
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("parse failed: zz".to_string())
        );
    }

    #[test]
    fn update_drives_disassembly_pin_and_back() {
        let bridge = full_bridge();
        // Line click pins the anchor and leaves follow mode.
        drive(&bridge, DebugMessage::DisLineClicked(0x300));
        assert!(!bridge.nav.lock().unwrap().follow_pc());
        drive(&bridge, DebugMessage::FollowTarget(0x308));
        // Back pops twice, then reports the empty stack.
        drive(&bridge, DebugMessage::DisBack);
        drive(&bridge, DebugMessage::DisBack);
        drive(&bridge, DebugMessage::DisBack);
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("back stack is empty".to_string())
        );
        // Go resolves valid hex, rejects the rest.
        drive(&bridge, DebugMessage::DisInputChanged("200".to_string()));
        drive(&bridge, DebugMessage::DisGo);
        assert!(!bridge.nav.lock().unwrap().follow_pc());
        drive(&bridge, DebugMessage::DisInputChanged("zz".to_string()));
        drive(&bridge, DebugMessage::DisGo);
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("parse failed: zz".to_string())
        );
        drive(&bridge, DebugMessage::FollowPcToggle);
        assert!(bridge.nav.lock().unwrap().follow_pc());
    }

    #[test]
    fn update_drives_bookmarks_watch_and_freeze() {
        let bridge = full_bridge();
        drive(
            &bridge,
            DebugMessage::BookmarkInputChanged("start".to_string()),
        );
        drive(&bridge, DebugMessage::BookmarkAdd);
        assert_eq!(bridge.bookmarks.lock().unwrap().len(), 1);
        // PC row anchors the bookmark while following.
        assert_eq!(bridge.bookmarks.lock().unwrap()[0].0, 0x0800_27D2);
        drive(&bridge, DebugMessage::BookmarkJump(0x0800_27D2));
        drive(&bridge, DebugMessage::BookmarkDelete(0x0800_27D2));
        assert!(bridge.bookmarks.lock().unwrap().is_empty());
        // A full list refuses loudly.
        bridge
            .bookmarks
            .lock()
            .unwrap()
            .extend((0..8).map(|i| (0x100 + i, format!("BM{i}"))));
        drive(&bridge, DebugMessage::BookmarkAdd);
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("bookmark list full".to_string())
        );
        // Watch follows the selection, then clears.
        drive(&bridge, DebugMessage::WatchSet);
        assert_eq!(*bridge.watch.lock().unwrap(), Some(0x100));
        drive(&bridge, DebugMessage::WatchClear);
        assert!(bridge.watch.lock().unwrap().is_none());
        *bridge.selected.lock().unwrap() = None;
        drive(&bridge, DebugMessage::WatchSet);
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("watch needs a selection".to_string())
        );
        // Freeze reads the top input, the edit input, then clears.
        *bridge.selected.lock().unwrap() = Some((0x100, Some(0xAB)));
        drive(&bridge, DebugMessage::FreezeInputChanged("00".to_string()));
        drive(&bridge, DebugMessage::FreezeSet);
        assert_eq!(*bridge.freeze.lock().unwrap(), Some((0x100, 0x00)));
        drive(&bridge, DebugMessage::EditFreezeSet);
        assert_eq!(*bridge.freeze.lock().unwrap(), Some((0x100, 0x42)));
        drive(&bridge, DebugMessage::FreezeClear);
        assert!(bridge.freeze.lock().unwrap().is_none());
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("freeze cleared".to_string())
        );
        // Rejections stay loud.
        *bridge.selected.lock().unwrap() = None;
        drive(&bridge, DebugMessage::FreezeSet);
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("freeze needs a selected row and hex byte".to_string())
        );
    }

    #[test]
    fn update_drives_write_and_register_flow() {
        let bridge = full_bridge();
        // EditWrite stages SelectRow plus EditStage in one batch.
        drive(&bridge, DebugMessage::EditInputChanged("42".to_string()));
        drive(&bridge, DebugMessage::EditWrite);
        assert_eq!(bridge.outbox.lock().unwrap().len(), 2);
        // Confirm consumes the transaction and clears the input.
        drive(&bridge, DebugMessage::ConfirmWrite);
        assert!(bridge.edit_input.lock().unwrap().is_empty());
        // Cancel clears the optimistic text.
        drive(&bridge, DebugMessage::CancelWrite);
        assert!(bridge.display.lock().unwrap().pending.is_none());
        // Register pick then Set pushes one register write.
        drive(&bridge, DebugMessage::RegSelected("r0"));
        assert_eq!(*bridge.reg_target.lock().unwrap(), Some("r0"));
        drive(&bridge, DebugMessage::RegInputChanged("7".to_string()));
        drive(&bridge, DebugMessage::RegSet);
        assert_eq!(bridge.outbox.lock().unwrap().len(), 5);
        // Rejections stay loud.
        *bridge.reg_target.lock().unwrap() = None;
        drive(&bridge, DebugMessage::RegSet);
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("register write needs a register".to_string())
        );
        *bridge.selected.lock().unwrap() = None;
        drive(&bridge, DebugMessage::EditWrite);
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("write needs a selection".to_string())
        );
        drive(&bridge, DebugMessage::RegInputChanged("zz".to_string()));
        *bridge.reg_target.lock().unwrap() = Some("r0");
        drive(&bridge, DebugMessage::RegSet);
        assert_eq!(
            bridge.display.lock().unwrap().status,
            StatusMessage::Local("parse failed: zz".to_string())
        );
    }

    #[test]
    fn view_builders_construct_every_section() {
        let bridge = full_bridge();
        let language = AppLanguage::English;
        let display = bridge.display.lock().unwrap().clone_without_images();
        let selected = *bridge.selected.lock().unwrap();
        let _ = view_toolbar(true, language);
        let _ = view_toolbar(false, language);
        let _ = view_space_row("Memory", language);
        let _ = view_mem_nav("0x00000100", language);
        // Mid-row byte selection still lights its row.
        let _ = view_dump_col(&display.dump_rows, &display.diff, Some((0x102, Some(9))));
        let _ = view_dump_col(&display.dump_rows, &[], None);
        let _ = view_regs_section(&display.regs, Some("r0"), "7", language);
        let _ = view_regs_section(&display.regs, None, "", language);
        let _ = view_regs_section(&[], None, "", language);
        let _ = view_edit_row(selected, "42", language);
        let _ = view_edit_row(None, "", language);
        let _ = view_confirm_row(
            Some(vm::PendingWrite {
                addr: 0x100,
                old: Some(0xAB),
                value: 0x42,
            }),
            language,
        );
        let _ = view_confirm_row(None, language);
        let _ = view_dis_nav(true, 0, 0x0800_27D2, "", language);
        let _ = view_dis_nav(false, 2, 0x300, "0x00000300", language);
        let _ = view_dis_col(&[], None, language);
        let _ = view_dis_col(&display.disasm_lines, None, language);
        let _ = view_dis_col(&display.disasm_lines, Some(0x0800_27D2E), language);
        let _ = view_right_col(
            StatusMessage::Paused,
            view_regs_section(&display.regs, None, "", language),
            Some(0x100),
            Some(0xAB),
            Some((0x100, 0xFF)),
            "FF",
            "tag",
            &[(0x200, "BM1".to_string())],
            language,
        );
        let _ = view_right_col(
            StatusMessage::Empty,
            view_regs_section(&[], None, "", language),
            None,
            None,
            None,
            "",
            "",
            &[],
            language,
        );
        // Disassembly lines exercise the target follow button.
        assert_eq!(display.disasm_lines.len(), 2);
    }

    #[test]
    fn view_assembles_follow_and_fixed_modes() {
        let bridge = full_bridge();
        let program = DebugProgram {
            bridge: Arc::clone(&bridge),
        };
        let state = DebugState {
            bridge: Arc::clone(&bridge),
        };
        // Follow-PC mode anchors on the first visible line.
        let _follow = program.view(&state, iced::window::Id::unique());
        drop(_follow);
        // Fixed mode pins the anchor and shows the Back depth.
        drive(&bridge, DebugMessage::DisLineClicked(0x300));
        let _fixed = program.view(&state, iced::window::Id::unique());
        drop(_fixed);
    }

    #[test]
    fn disasm_marker_never_double_marks_pc() {
        use nerust_core_traits::debugger::DisasmLine;

        let pc_line = DisasmLine {
            addr: 0xC293,
            bytes: [0x60, 0, 0, 0],
            len: 1,
            text: "RTS".to_string(),
            is_pc: true,
            target: None,
        };
        // The composed row carries exactly one `>`: the format owns
        // it, the view prefix contributes blanks.
        let composed = format!(
            "{}{}",
            disasm_marker(pc_line.is_pc, true),
            vm::format_disasm_line(&pc_line)
        );
        assert!(composed.starts_with("  >0x"), "got {composed:?}");
        // Pinned non-PC rows keep the `@` in the same column.
        assert_eq!(disasm_marker(false, true), "@ ");
        assert_eq!(disasm_marker(false, false), "  ");
        assert_eq!(disasm_marker(true, false), "  ");
    }

    #[test]
    fn shortcut_and_cursor_keys_map_physical_codes() {
        use iced::keyboard::key::{Code, Physical};
        assert!(matches!(
            shortcut_request(&Physical::Code(Code::F6)),
            Some(DebugAction::StepFrame)
        ));
        assert!(matches!(
            shortcut_request(&Physical::Code(Code::F7)),
            Some(DebugAction::StepInstr)
        ));
        assert!(matches!(
            shortcut_request(&Physical::Code(Code::F9)),
            Some(DebugAction::TogglePause)
        ));
        assert!(shortcut_request(&Physical::Code(Code::KeyA)).is_none());
        assert!(matches!(
            cursor_page(&Physical::Code(Code::ArrowUp)),
            Some(DebugMessage::MemPage(-1))
        ));
        assert!(matches!(
            cursor_page(&Physical::Code(Code::ArrowDown)),
            Some(DebugMessage::MemPage(1))
        ));
        assert!(cursor_page(&Physical::Code(Code::ArrowLeft)).is_none());
    }

    /// Headless UI-shell lifecycle test (build/pump/rebuild/relayout/
    /// draw). Software rasterization only, so it runs without a display
    /// and never contacts the emulator. Lives here to keep `iced_shell`
    /// free of `debug_window` imports.
    #[test]
    fn uishell_headless_build_pump_relayout_draw() {
        use crate::iced_shell::UiShell;

        let bridge = Arc::new(DebugBridge::new(
            DisplayCache::default(),
            Vec::new(),
            0,
            AppLanguage::English,
        ));
        let (instance, _task) = program::Instance::new(DebugProgram {
            bridge: Arc::clone(&bridge),
        });
        let mut backend = Renderer::new(crate::tao_conversions::default_font(), iced::Pixels(16.0));
        let bounds = iced::Size::new(1100.0, 800.0);
        let mut shell = UiShell::new(instance, iced::window::Id::unique(), bounds, &mut backend);
        let mut clipboard = iced_winit::Clipboard::unconnected();
        let (messages, _) = shell.pump(
            iced::Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            )),
            iced::mouse::Cursor::default(),
            &mut clipboard,
            &mut backend,
        );
        assert!(messages.is_empty());
        shell.push_message(DebugMessage::RefreshPressed);
        let invalidated = AtomicBool::new(true);
        shell.process_messages(
            vec![DebugMessage::FollowPcToggle],
            iced::window::Id::unique(),
            bounds,
            &mut backend,
            &invalidated,
        );
        assert!(!invalidated.load(Ordering::Acquire));
        shell.relayout(iced::Size::new(900.0, 650.0), &mut backend);
        let theme = iced::Theme::Light;
        let style = <iced::Theme as iced::theme::Base>::base(&theme);
        shell.ui_mut().draw(
            &mut backend,
            &theme,
            &iced::advanced::renderer::Style {
                text_color: style.text_color,
            },
            iced::mouse::Cursor::default(),
        );
    }
}
