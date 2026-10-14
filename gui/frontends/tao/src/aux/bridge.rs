//! Shared bridge between the iced programs and the host drain.
//! Used by both [`super::debug`] and [`super::ppu`] so the viewers stay
//! siblings instead of one depending on the other.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use nerust_core_traits::debugger::SpaceId;
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{DisplayCache, NavState, StatusMessage};
use nerust_settings_core::model::language::AppLanguage;

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

    pub(crate) fn push(&self, request: DebugAction) {
        self.outbox.lock().unwrap().push(request);
        self.view_invalidated.store(true, Ordering::Release);
    }

    /// Displayed disassembly anchor: first visible line in follow-PC
    /// mode (the stored pin is stale there), else the pin.
    pub(crate) fn anchor(&self) -> u32 {
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
