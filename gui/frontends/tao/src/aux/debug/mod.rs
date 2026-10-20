//! Debugger window: memory/disassembly probe plus register, watch, freeze, bookmarks.
//! Pure view; the host drain runs requests and refreshes the display cache.

#[cfg(test)]
mod tests;
mod update;
mod views;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use iced::Task;
use iced_tiny_skia::Renderer;
use iced_winit::program::{self, Program};
use nerust_core_traits::debugger::SpaceId;
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::DisplayCache;
use nerust_settings_core::i18n::{UiText, text as ui_text};
use nerust_settings_core::model::language::AppLanguage;
use tao::event_loop::EventLoopWindowTarget;

use crate::aux::shell::{AuxSurface, aux_settings, aux_theme};

use super::bridge::DebugBridge;

/// Initial debugger window size (verified Xvfb fit).
pub(crate) const DEBUG_WINDOW_SIZE: (f64, f64) = (1100.0, 800.0);
/// Minimum debugger size: below this the fixed label columns and
/// the register halves collide instead of reflowing.
pub(crate) const DEBUG_WINDOW_MIN_SIZE: (f64, f64) = (900.0, 650.0);
/// Right state column width.
const STATE_COLUMN_WIDTH: f32 = 340.0;

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
        update::update_message(&state.bridge, message);
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
        let views::ViewSnapshot {
            language,
            mem_input,
            dis_input,
            follow_pc,
            back_len,
            anchor,
            bookmarks,
            bm_input,
            watch,
            watch_value,
            freeze,
            freeze_input,
            selected,
            edit_input,
            pending,
            selected_name,
            regs,
            dump_rows,
            diff,
            disasm_lines,
            status,
            paused,
            reg_target,
            reg_input,
        } = views::capture_view(&state.bridge);
        // Button hierarchy: one primary per flow, secondary for toolbar/nav.
        let toolbar = views::view_toolbar(paused, language);
        // Settings form language: fixed label column + stepper.
        let space_row = views::view_space_row(selected_name.as_deref().unwrap_or(""), language);
        let mem_nav = views::view_mem_nav(&mem_input, language);
        // Heavy lists ride `lazy`; keep every visual input in the dependency tuples.
        let dump_col = lazy((dump_rows, diff, selected), |(rows, diff, selected)| {
            views::view_dump_col(rows, diff, *selected)
        });
        let regs_view: iced::Element<'static, DebugMessage, iced::Theme, Renderer> = lazy(
            (regs, reg_target, reg_input, language),
            |(regs, target, input, language)| {
                views::view_regs_section(regs, *target, input, *language)
            },
        )
        .into();
        // Two-phase write: select a dump row, type the new byte,
        // Write stages it, and the confirm row commits or cancels.
        let edit_row = views::view_edit_row(selected, &edit_input, language);
        // Confirm row: the one primary button in the window; Cancel is
        // a text escape. The drain owns the authoritative text.
        let confirm_row = views::view_confirm_row(pending, language);
        let dis_nav = views::view_dis_nav(follow_pc, back_len, anchor, &dis_input, language);
        // Per-line disassembly: line click pins the anchor; targets offer follow buttons.
        let pinned = if follow_pc { None } else { Some(anchor) };
        let dis_col = lazy(
            (disasm_lines, pinned, language),
            |(lines, pinned, language)| views::view_dis_col(lines, *pinned, *language),
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
        let right_col = column![
            text(status.render(language)).size(14).width(Length::Fill),
            text(ui_text(language, UiText::Registers)).size(16),
            regs_view,
            text(ui_text(language, UiText::Watch)).size(16),
            views::view_watch_row(watch, watch_value, language),
            text(ui_text(language, UiText::Freeze)).size(16),
            views::view_freeze_row(freeze, &freeze_input, language),
            views::view_bookmark_section(&bm_input, &bookmarks, language),
        ]
        .spacing(12)
        .width(Length::Fixed(STATE_COLUMN_WIDTH))
        // Right breathing room for the overlay scrollbar: without it the
        // rail sits on top of the register text (tall GBA lists scroll).
        .padding(iced::Padding {
            right: 16.0,
            ..Default::default()
        });
        // Tall state scrolls inside the panes; Refresh stays pinned at the bottom.
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

pub(crate) struct DebugWindowHandle {
    shell: crate::aux::shell::AuxShell<DebugProgram>,
    bridge: Arc<DebugBridge>,
}

impl DebugWindowHandle {
    /// Narrow live-images apply for the PPU follow tick: images land when newer.
    /// Returns whether the viewer needs a rebuild.
    pub(crate) fn apply_live(
        &mut self,
        generation: u64,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        running: bool,
    ) -> bool {
        self.bridge.apply_live_images(generation, images, running)
    }

    /// Share the bridge for PPU-viewer creation. Narrow: creation only, never display locks.
    pub(crate) fn shared_bridge(&self) -> &Arc<DebugBridge> {
        &self.bridge
    }

    /// Narrow selection write + paused-section restore for a finished
    /// drain. The host never holds a `display` guard itself.
    pub(crate) fn apply_drain_display(
        &mut self,
        display: DisplayCache,
        reselected: Option<(u32, Option<u8>)>,
    ) {
        self.bridge.apply_drain_display(display, reselected);
    }
}

/// Window-scoped shortcuts on physical codes: F6 frame, F7 instruction, F9 pause.
fn shortcut_request(physical: &iced::keyboard::key::Physical) -> Option<DebugAction> {
    use iced::keyboard::key::{Code, Physical};
    match *physical {
        Physical::Code(Code::F6) => Some(DebugAction::StepFrame),
        Physical::Code(Code::F7) => Some(DebugAction::StepInstr),
        Physical::Code(Code::F9) => Some(DebugAction::TogglePause),
        _ => None,
    }
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
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        position: Option<(i32, i32)>,
    ) -> Option<Self> {
        let bridge = Arc::new(DebugBridge::new(display, spaces, mem_addr, language));
        let title = ui_text(language, UiText::Debugger).to_string();
        let surface = AuxSurface::open(
            event_loop,
            &title,
            DEBUG_WINDOW_SIZE,
            Some(DEBUG_WINDOW_MIN_SIZE),
            position,
            "debugger window",
            Arc::new(AtomicBool::new(false)),
        )
        .map_err(|error| {
            // Typed scaffold failure: surface the tagged native detail here.
            log::error!("{error}");
            error
        })
        .ok()?;
        let program = DebugProgram {
            bridge: Arc::clone(&bridge),
        };
        let (instance, _task) = program::Instance::new(program);
        let shell = crate::aux::shell::AuxShell::assemble(surface, instance);
        Some(Self { shell, bridge })
    }

    pub(crate) fn sync_from_bridge(&mut self) {
        self.bridge.view_invalidated.store(true, Ordering::Release);
        self.shell
            .process(Vec::new(), &self.bridge.view_invalidated);
    }
}

impl super::focus::AuxShellRef for DebugWindowHandle {
    fn kind(&self) -> super::focus::AuxKind {
        super::focus::AuxKind::Debug
    }

    fn shell(&self) -> &dyn crate::aux::shell::AuxShellOps {
        &self.shell
    }

    fn shell_mut(&mut self) -> &mut dyn crate::aux::shell::AuxShellOps {
        &mut self.shell
    }
}

impl super::registry::DebugDrainPort for DebugWindowHandle {
    fn take_debug_requests(&mut self) -> Option<(bool, Vec<DebugAction>)> {
        use crate::aux::bridge::BridgeLock;
        Some(self.bridge.take_debug_batch())
    }

    fn apply_drain(&mut self, display: DisplayCache, reselected: Option<(u32, Option<u8>)>) {
        self.apply_drain_display(display, reselected);
    }
}

impl super::registry::LiveImagesPort for DebugWindowHandle {
    fn apply_live(
        &mut self,
        generation: u64,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        running: bool,
    ) -> bool {
        self.apply_live(generation, images, running)
    }
}

impl super::registry::AuxController for DebugWindowHandle {
    fn sync_view(&mut self) {
        self.sync_from_bridge();
    }

    fn as_debug(&mut self) -> Option<&mut dyn super::registry::DebugDrainPort> {
        Some(self)
    }

    fn debug_snapshot_of(&self, ppu_open: bool) -> Option<super::bridge::DrainSnapshot> {
        use crate::aux::bridge::BridgeLock;
        let lock: &dyn BridgeLock = &*self.bridge;
        Some(lock.drain_snapshot(ppu_open))
    }

    fn as_live(&mut self) -> Option<&mut dyn super::registry::LiveImagesPort> {
        Some(self)
    }

    fn shared_bridge(&self) -> Option<std::sync::Arc<super::bridge::DebugBridge>> {
        Some(std::sync::Arc::clone(self.shared_bridge()))
    }

    fn debug_lock_audit(&self) -> Vec<&'static str> {
        self.bridge.try_lock_audit()
    }
}

impl super::focus::AuxEvents for DebugWindowHandle {
    fn handle(&mut self, mapped: iced::Event) -> bool {
        // Window-scoped shortcuts on physical codes (F6/F7/F9).
        // Dereference `physical_key` before matching (binds by ref).
        if let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            physical_key,
            repeat: false,
            ..
        }) = &mapped
            && let Some(request) = shortcut_request(physical_key)
        {
            self.bridge.push(request);
        }
        // Cursor keys page the dump; Left/Right stay with the widgets.
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
        self.shell
            .pump_finish(mapped, page, &self.bridge.view_invalidated)
    }
}
