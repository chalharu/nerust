//! One reducer for owned native models and the shared split-lock bridge.

use std::{
    ops::{Deref, DerefMut},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use nerust_core_traits::debugger::{DEBUG_DUMP_PAGE_ROWS, DUMP_ROW_BYTES, SpaceId};
use nerust_gui_shell::{debug::DebugAction, session::SessionHandle};
use nerust_gui_viewmodel::debugger::{
    DebugImageRgba, DisplayCache, NavState, StatusMessage, parse_hex_addr,
};
use nerust_settings_core::{
    i18n::{self as ui_msg, UiText, text},
    model::language::AppLanguage,
};

use super::{DebuggerEvent, DebuggerScope, DebuggerUpdate};

const BOOKMARK_CAP: usize = 8;

/// Small mutable state only. Images are owned by the caller's display storage.
#[derive(Debug, Clone)]
pub struct DebuggerState {
    pub nav: NavState,
    pub bookmarks: Vec<(u32, String)>,
    pub bm_input: String,
    pub watch: Option<u32>,
    pub freeze: Option<(u32, u8)>,
    pub freeze_input: String,
    pub selected: Option<(u32, Option<u8>)>,
    pub edit_input: String,
    pub reg_target: Option<&'static str>,
    pub reg_input: String,
    pub ppu_hover: String,
}

/// Reducer output before applying its cold display patch. Keeping the patch
/// separate lets the bridge avoid `Arc::make_mut` on input-only events.
#[derive(Debug, Default)]
pub struct DebuggerReduction {
    pub update: DebuggerUpdate,
    status: Option<StatusMessage>,
    clear_watch: bool,
    clear_pending: bool,
}

impl DebuggerReduction {
    pub(crate) fn changes_display(&self, display: &DisplayCache) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| *status != display.status)
            || (self.clear_watch && display.watch_value.is_some())
            || (self.clear_pending && display.pending.is_some())
    }

    /// Apply only the display fields changed by the intent; no images are read or cloned.
    pub fn apply(self, display: &mut DisplayCache) -> DebuggerUpdate {
        if let Some(status) = self.status {
            display.status = status;
        }
        if self.clear_watch {
            display.watch_value = None;
        }
        if self.clear_pending {
            display.pending = None;
        }
        self.update
    }

    fn action(&mut self, action: DebugAction, scope: DebuggerScope) {
        self.update.actions.push(action);
        self.update.scope = Some(scope);
    }

    fn status(&mut self, status: String) {
        self.status = Some(StatusMessage::Local(status));
        self.update.scope = Some(DebuggerScope::Header);
    }
}

impl DebuggerState {
    pub fn new(space_count: usize, mem_addr: u32) -> Self {
        Self {
            nav: NavState::new(space_count, mem_addr),
            bookmarks: Vec::new(),
            bm_input: String::new(),
            watch: None,
            freeze: None,
            freeze_input: String::new(),
            selected: None,
            edit_input: String::new(),
            reg_target: None,
            reg_input: String::new(),
            ppu_hover: String::new(),
        }
    }

    /// Navigation undo anchors on the first visible line while following.
    pub fn anchor(&self, display: &DisplayCache) -> u32 {
        if self.nav.follow_pc() {
            display
                .disasm_lines
                .first()
                .map(|line| line.addr)
                .unwrap_or(0)
        } else {
            self.nav.dis_addr().unwrap_or(0)
        }
    }

    /// Reduces an intent without toolkit, transport, locks, or image copies.
    /// Selection bytes and pending transactions remain authoritative drain results.
    pub fn reduce(
        &mut self,
        event: DebuggerEvent,
        spaces: &[(SpaceId, String, u32)],
        language: AppLanguage,
        display: &DisplayCache,
    ) -> DebuggerReduction {
        use DebuggerEvent::*;
        use DebuggerScope::*;
        let mut result = DebuggerReduction::default();
        match event {
            MemInputChanged(input) => self.nav.set_mem_input(input),
            DisInputChanged(input) => self.nav.set_dis_input(input),
            BookmarkInputChanged(input) => self.bm_input = input,
            FreezeInputChanged(input) => self.freeze_input = input,
            EditInputChanged(input) => self.edit_input = input,
            RegInputChanged(input) => self.reg_input = input,
            RefreshPressed => result.action(DebugAction::Refresh, All),
            TogglePausePressed => result.action(DebugAction::TogglePause, All),
            StepFramePressed => result.action(DebugAction::StepFrame, All),
            StepInstrPressed => result.action(DebugAction::StepInstr, All),
            PpuOpenPressed => {
                result.update.open_ppu = true;
                result.action(DebugAction::Refresh, All);
            }
            SpacePrev | SpaceNext => {
                let delta = if matches!(event, SpacePrev) { -1 } else { 1 };
                if let Some(idx) = self.nav.cycle_space(delta)
                    && let Some((_, _, start)) = spaces.get(idx)
                {
                    self.nav.set_space(idx, *start);
                    self.selected = None;
                    result.action(DebugAction::Refresh, Dump);
                }
            }
            MemGo => match parse_hex_addr(self.nav.mem_input()) {
                Some(addr) => {
                    self.nav.go_mem(addr);
                    result.action(DebugAction::MemNav, Dump);
                }
                None => result.status(ui_msg::parse_failed(language, self.nav.mem_input().trim())),
            },
            MemPage(delta) => {
                let page = DEBUG_DUMP_PAGE_ROWS as i32 * DUMP_ROW_BYTES as i32;
                self.nav.page_mem(delta.saturating_mul(page));
                result.action(DebugAction::MemNav, Dump);
            }
            RowSelected(addr) => result.action(DebugAction::SelectRow(addr), Dump),
            DisGo => match parse_hex_addr(self.nav.dis_input()) {
                Some(addr) => self.navigate(addr, display, &mut result),
                None => result.status(ui_msg::parse_failed(language, self.nav.dis_input().trim())),
            },
            DisLineClicked(addr) | FollowTarget(addr) | BookmarkJump(addr) => {
                self.navigate(addr, display, &mut result);
            }
            FollowPcToggle => {
                self.nav.follow();
                result.action(DebugAction::Refresh, Disasm);
            }
            DisBack => {
                if self.nav.go_back().is_some() {
                    result.action(DebugAction::Refresh, Disasm);
                } else {
                    result.status(text(language, UiText::BackEmpty).to_string());
                }
            }
            BookmarkAdd => {
                // Only follow mode prefers the PC row; a visible PC must never
                // override an explicitly pinned bookmark target.
                let addr = if self.nav.follow_pc() {
                    display
                        .disasm_lines
                        .iter()
                        .find(|line| line.is_pc)
                        .map(|line| line.addr)
                } else {
                    None
                }
                .unwrap_or_else(|| self.anchor(display));
                if self.bookmarks.iter().any(|(old, _)| *old == addr) {
                    self.bm_input.clear();
                    result.update.scope = Some(Bookmarks);
                } else if self.bookmarks.len() >= BOOKMARK_CAP {
                    result.status(text(language, UiText::BookmarkFull).to_string());
                } else {
                    let label = self.bm_input.trim();
                    let label = if label.is_empty() {
                        format!("BM{}", self.bookmarks.len() + 1)
                    } else {
                        label.to_string()
                    };
                    self.bookmarks.push((addr, label));
                    self.bm_input.clear();
                    result.update.scope = Some(Bookmarks);
                }
            }
            BookmarkDelete(addr) => {
                self.bookmarks.retain(|(old, _)| *old != addr);
                result.update.scope = Some(Bookmarks);
            }
            WatchSet => match self.selected {
                Some((addr, _)) => {
                    self.watch = Some(addr);
                    result.action(DebugAction::Refresh, State);
                }
                None => result.status(text(language, UiText::WatchNeedsSelection).to_string()),
            },
            WatchClear => {
                self.watch = None;
                result.clear_watch = true;
                result.update.scope = Some(State);
            }
            FreezeSet | EditFreezeSet => {
                let input = if matches!(event, FreezeSet) {
                    &self.freeze_input
                } else {
                    &self.edit_input
                };
                match (
                    self.selected,
                    parse_hex_addr(input).filter(|value| *value <= 0xff),
                ) {
                    (Some((addr, _)), Some(value)) => {
                        self.freeze = Some((addr, value as u8));
                        result.status(ui_msg::frozen(language, addr, value as u8));
                        result.action(DebugAction::Refresh, All);
                    }
                    _ => result.status(text(language, UiText::FreezeNeedsSelection).to_string()),
                }
            }
            FreezeClear => {
                self.freeze = None;
                result.status(text(language, UiText::FreezeCleared).to_string());
                result.update.scope = Some(State);
            }
            EditWrite => {
                let input = self.edit_input.trim();
                match (
                    self.selected,
                    parse_hex_addr(input).filter(|value| *value <= 0xff),
                ) {
                    (Some((addr, _)), Some(value)) => {
                        result.action(DebugAction::SelectRow(addr), Dump);
                        result.action(DebugAction::EditStage(u64::from(value)), Dump);
                    }
                    (None, _) => {
                        result.status(text(language, UiText::WriteNeedsSelection).to_string())
                    }
                    _ => result.status(ui_msg::parse_failed(language, input)),
                }
            }
            ConfirmWrite => {
                self.edit_input.clear();
                result.action(DebugAction::WriteConfirm, All);
            }
            CancelWrite => {
                result.clear_pending = true;
                result.action(DebugAction::EditCancel, Edit);
            }
            RegSelected(name) => {
                self.reg_target = Some(name);
                result.update.scope = Some(State);
            }
            RegSet => match (self.reg_target, parse_hex_addr(&self.reg_input)) {
                (Some(_), Some(value)) => {
                    result.action(DebugAction::RegisterWrite(u64::from(value)), State)
                }
                (None, _) => {
                    result.status(text(language, UiText::RegisterNeedsSelection).to_string())
                }
                _ => result.status(ui_msg::parse_failed(language, self.reg_input.trim())),
            },
        }
        result
    }

    fn navigate(&mut self, addr: u32, display: &DisplayCache, result: &mut DebuggerReduction) {
        self.nav.navigate(self.anchor(display), addr);
        result.action(DebugAction::Refresh, DebuggerScope::Disasm);
    }
}

/// Single-owner storage for native views. Deref preserves direct state access
/// without a shadow copy; the display (including images) is owned exactly once.
pub struct DebuggerModel {
    pub epoch: Arc<AtomicBool>,
    pub spaces: Vec<(SpaceId, String, u32)>,
    pub language: AppLanguage,
    pub display: DisplayCache,
    state: DebuggerState,
}

impl Deref for DebuggerModel {
    type Target = DebuggerState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for DebuggerModel {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl DebuggerModel {
    pub fn new(
        display: DisplayCache,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        language: AppLanguage,
        epoch: Arc<AtomicBool>,
    ) -> Self {
        let state = DebuggerState::new(spaces.len(), mem_addr);
        Self {
            epoch,
            spaces,
            language,
            display,
            state,
        }
    }

    pub fn from_session(session: &SessionHandle) -> Self {
        let spaces: Vec<_> = session
            .debug_spaces()
            .iter()
            .map(|info| (info.id, info.name.to_string(), *info.range.start()))
            .collect();
        let start = spaces.first().map(|(_, _, start)| *start).unwrap_or(0);
        Self::new(
            DisplayCache::default(),
            spaces,
            start,
            session.settings_snapshot().shared.general.language,
            session.debug_epoch(),
        )
    }

    /// Reject gestures against a retired session before even echoing inputs.
    pub fn dispatch(&mut self, event: DebuggerEvent) -> DebuggerUpdate {
        if !self.epoch.load(Ordering::Acquire) {
            return DebuggerUpdate::default();
        }
        self.state
            .reduce(event, &self.spaces, self.language, &self.display)
            .apply(&mut self.display)
    }

    pub fn selected_space(&self) -> Option<SpaceId> {
        self.spaces.get(self.nav.space_idx()).map(|(id, _, _)| *id)
    }

    pub fn selected_name(&self) -> String {
        self.spaces
            .get(self.nav.space_idx())
            .map(|(_, name, _)| name.clone())
            .unwrap_or_default()
    }

    pub fn anchor(&self) -> u32 {
        self.state.anchor(&self.display)
    }

    /// Pre-localized debugger chrome. Frontends render it verbatim.
    pub fn debugger_chrome(&self) -> super::DebuggerChrome {
        super::DebuggerChrome::new(self.language)
    }

    /// Pre-localized PPU chrome for the viewer opened from this model.
    pub fn ppu_chrome(&self) -> super::PpuChrome {
        super::PpuChrome::new(self.language)
    }

    /// Status line in the view language. Coreless windows read as
    /// paused with an explicit no-core status (gtk/tao agreement).
    pub fn status_line(&self) -> String {
        let coreless = self.spaces.is_empty();
        if coreless && matches!(self.display.status, StatusMessage::Empty) {
            StatusMessage::NoCore.render(self.language)
        } else {
            self.display.status.render(self.language)
        }
    }

    /// Paused-aware status: coreless windows read as paused.
    pub fn paused_line(&self) -> bool {
        self.display.paused || self.spaces.is_empty()
    }

    /// Pending-write line, if a write is staged.
    pub fn pending_line(&self) -> Option<String> {
        self.display
            .pending
            .map(|pending| pending.render(self.language))
    }

    /// Formatted debugger rows without exposing row types or formatters.
    pub fn debugger_rows(&self) -> super::view::DebuggerRows {
        let coreless = self.spaces.is_empty();
        let mut rows = {
            let status_text = self.display.status.render(self.language);
            let pending_text = self
                .display
                .pending
                .map(|pending| pending.render(self.language));
            super::view::rows_from_display(&self.display, status_text, pending_text)
        };
        if coreless {
            rows.paused = true;
            if matches!(self.display.status, StatusMessage::Empty) {
                rows.status_text = StatusMessage::NoCore.render(self.language);
            }
        }
        rows
    }

    /// 2x-scaled PPU pictures without exposing the scaler.
    pub fn ppu_pictures(&self) -> Vec<super::view::ScaledPicture> {
        super::view::scaled_from_display(&self.display)
    }

    /// Formatted panel text (`None` when the system has no tables).
    pub fn ppu_panels_text(&self) -> Option<String> {
        self.has_panels()
            .then(|| super::view::panels_text(&self.display))
    }

    /// Whether the system exposes observation tables.
    pub fn has_panels(&self) -> bool {
        !self.display.panels.is_empty()
    }

    /// Generation of the source images behind [`Self::ppu_pictures`].
    pub fn images_generation(&self) -> u64 {
        self.display.images_gen
    }

    pub fn apply_drain(
        &mut self,
        mut display: DisplayCache,
        reselected: Option<(u32, Option<u8>)>,
    ) {
        if let Some(selected) = reselected {
            self.state.selected = Some(selected);
        }
        display.restore_paused_sections(&self.display);
        self.display = display;
    }

    pub fn apply_live_images(
        &mut self,
        generation: u64,
        images: Vec<DebugImageRgba>,
        running: bool,
    ) -> bool {
        if self.display.images_gen == generation {
            return false;
        }
        apply_live(&mut self.display, generation, images, running);
        true
    }
}

pub(super) fn apply_live(
    display: &mut DisplayCache,
    generation: u64,
    images: Vec<DebugImageRgba>,
    running: bool,
) {
    display.images = images;
    display.images_gen = generation;
    if running {
        display.paused = false;
        display.status = StatusMessage::Running;
    }
}
