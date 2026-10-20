//! Shared bridge between the iced programs and the host drain.
//! Used by both [`super::debug`] and [`super::ppu`] so the viewers stay
//! siblings instead of one depending on the other.
//!
//! # Lock order
//!
//! `nav` before `display` before any small field mutex (`selected`,
//! `watch`, `freeze`, `reg_target`, `ppu_hover`, `outbox`). Dual
//! access goes through [`DebugBridge::nav_display`]; single-field
//! access uses [`lock_recovered`]. No path may hold a `display` guard
//! while acquiring `nav`, and no path may hold two small-field guards
//! at once. The host drain only touches the bridge through
//! [`BridgeLock`], so the order is enforced at one seam.
//! Blocking `lock()` is never used: [`lock_recovered`] recovers from
//! poison and `try_lock` audits below assert no contention path can
//! wedge the UI thread (all bridge critical sections are short field
//! copies).

use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};

use nerust_core_traits::debugger::SpaceId;
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{DisplayCache, NavState, StatusMessage};
use nerust_settings_core::model::language::AppLanguage;

/// Poison-safe mutex lock: recovers the inner value after a panic
/// instead of propagating the poison. Recovered state may be stale;
/// the next drain refresh overwrites it.
pub(crate) fn lock_recovered<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| {
        log::warn!("mutex recovered after poisoning; state may be stale");
        e.into_inner()
    })
}

/// Shared bridge between the iced program and the host drain.
/// Movement state and display cache are owned separately; user lists
/// (bookmarks, watch, freeze, selection) are presentation state.
/// Snapshot returned by [`DebugBridge::drain_snapshot`]: nav space,
/// dump/disassembly anchors, watch/freeze/reg-target, previous rows,
/// and the PPU-open flag. Named so the 8-tuple does not sprawl at
/// every signature.
pub(crate) type DrainSnapshot = (
    Option<SpaceId>,
    u32,
    Option<u32>,
    Option<u32>,
    Option<(u32, u8)>,
    Option<&'static str>,
    std::sync::Arc<[nerust_core_traits::debugger::HexRow]>,
    bool,
);

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

    /// Lock nav-then-display in one place. Every dual access must
    /// go through here so the order never inverts and deadlocks.
    pub(crate) fn nav_display(&self) -> (MutexGuard<'_, NavState>, MutexGuard<'_, DisplayCache>) {
        (lock_recovered(&self.nav), lock_recovered(&self.display))
    }

    pub(crate) fn selected_space(&self) -> Option<SpaceId> {
        let idx = lock_recovered(&self.nav).space_idx();
        self.spaces.get(idx).map(|(id, _, _)| *id)
    }

    /// Narrow drain snapshot: nav (space/mem/dis) plus the small
    /// watch/freeze/reg-target fields plus the previous dump rows for
    /// the diff gate. Single method so `drain.rs` never locks
    /// `handle.bridge.*` directly. Lock order inside: nav, then
    /// display (rows), then one small field at a time.
    pub(crate) fn drain_snapshot(&self, ppu_open: bool) -> DrainSnapshot {
        let nav = lock_recovered(&self.nav);
        let mem_addr = nav.mem_addr();
        let dis_addr = nav.dis_addr();
        drop(nav);
        let space = self.selected_space();
        let prev: std::sync::Arc<[nerust_core_traits::debugger::HexRow]> =
            lock_recovered(&self.display).dump_rows.clone().into();
        let watch = *lock_recovered(&self.watch);
        let freeze = *lock_recovered(&self.freeze);
        let reg_target = *lock_recovered(&self.reg_target);
        (
            space, mem_addr, dis_addr, watch, freeze, reg_target, prev, ppu_open,
        )
    }

    /// Apply a finished live-images fetch: replaces images when newer
    /// regardless of pause; running chrome only applies while still
    /// running. Returns whether the viewer needs a rebuild.
    pub(crate) fn apply_live_images(
        &self,
        generation: u64,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        running: bool,
    ) -> bool {
        let mut display = lock_recovered(&self.display);
        if display.images_gen == generation {
            return false;
        }
        display.images = images;
        display.images_gen = generation;
        if running {
            display.paused = false;
            display.status = nerust_gui_viewmodel::debugger::StatusMessage::Running;
        }
        true
    }

    /// Apply a finished drain display: selection write plus the
    /// paused-section restore live here so the host never holds a
    /// `display` guard itself.
    pub(crate) fn apply_drain_display(
        &self,
        mut display: DisplayCache,
        reselected: Option<(u32, Option<u8>)>,
    ) {
        if let Some(selected) = reselected {
            *lock_recovered(&self.selected) = Some(selected);
        }
        {
            let old = lock_recovered(&self.display).clone_without_images();
            display.restore_paused_sections(&old);
        }
        *lock_recovered(&self.display) = display;
    }

    /// Non-blocking contention audit: every bridge mutex must be
    /// immediately available on the UI thread. Returns the names of
    /// contended locks (empty when healthy). All critical sections
    /// are short field copies, so contention indicates a bug. The
    /// host drain asserts this before snapshotting.
    pub(crate) fn try_lock_audit(&self) -> Vec<&'static str> {
        let mut contended = Vec::new();
        if self.nav.try_lock().is_err() {
            contended.push("nav");
        }
        if self.display.try_lock().is_err() {
            contended.push("display");
        }
        if self.watch.try_lock().is_err() {
            contended.push("watch");
        }
        if self.freeze.try_lock().is_err() {
            contended.push("freeze");
        }
        if self.selected.try_lock().is_err() {
            contended.push("selected");
        }
        if self.outbox.try_lock().is_err() {
            contended.push("outbox");
        }
        contended
    }

    pub(crate) fn selected_name(&self) -> Option<String> {
        let idx = lock_recovered(&self.nav).space_idx();
        self.spaces.get(idx).map(|(_, name, _)| name.clone())
    }

    pub(crate) fn set_status(&self, status: String) {
        // Frontend-local wording (parse errors, prompts): stored
        // verbatim. Drain outcomes arrive structured and never pass
        // through here.
        lock_recovered(&self.display).status = StatusMessage::Local(status);
        self.view_invalidated.store(true, Ordering::Release);
    }

    pub(crate) fn take_requests(&self) -> Vec<DebugAction> {
        std::mem::take(&mut *lock_recovered(&self.outbox))
    }

    pub(crate) fn push(&self, request: DebugAction) {
        lock_recovered(&self.outbox).push(request);
        self.view_invalidated.store(true, Ordering::Release);
    }

    /// Displayed disassembly anchor: first visible line in follow-PC
    /// mode (the stored pin is stale there), else the pin.
    pub(crate) fn anchor(&self) -> u32 {
        // Nav-then-display order via `nav_display`; never nest a
        // display guard under a nav guard elsewhere.
        let (nav, display) = self.nav_display();
        if nav.follow_pc() {
            display
                .disasm_lines
                .first()
                .map(|line| line.addr)
                .unwrap_or(0)
        } else {
            nav.dis_addr().unwrap_or(0)
        }
    }
}

/// Narrow host-drain port over [`DebugBridge`]. `drain.rs` takes
/// `&dyn BridgeLock` and never locks `handle.bridge.display` directly.
pub(crate) trait BridgeLock {
    fn drain_snapshot(&self, ppu_open: bool) -> DrainSnapshot;
    fn take_debug_batch(&self) -> (bool, Vec<DebugAction>);
    /// Contended lock names, if any (see `try_lock_audit`).
    fn try_lock_audit(&self) -> Vec<&'static str>;
}

impl BridgeLock for DebugBridge {
    fn drain_snapshot(&self, ppu_open: bool) -> DrainSnapshot {
        DebugBridge::drain_snapshot(self, ppu_open)
    }

    fn take_debug_batch(&self) -> (bool, Vec<DebugAction>) {
        (
            self.open_ppu
                .swap(false, std::sync::atomic::Ordering::SeqCst),
            self.take_requests(),
        )
    }

    fn try_lock_audit(&self) -> Vec<&'static str> {
        DebugBridge::try_lock_audit(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_bridge() -> DebugBridge {
        DebugBridge::new(
            DisplayCache::default(),
            vec![(SpaceId(0), "Memory".to_string(), 0)],
            0,
            AppLanguage::English,
        )
    }

    #[test]
    fn try_lock_audit_is_empty_when_uncontended() {
        let bridge = test_bridge();
        assert!(bridge.try_lock_audit().is_empty());
    }

    #[test]
    fn lock_recovered_returns_guard_on_clean_mutex() {
        let mutex = Mutex::new(42u32);
        assert_eq!(*lock_recovered(&mutex), 42);
    }

    #[test]
    fn lock_recovered_recovers_value_after_poison() {
        let mutex = Mutex::new(7u32);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = mutex.lock().unwrap();
            panic!("intentional poison");
        }));
        assert!(mutex.is_poisoned());
        // Recovery keeps the pre-panic value instead of panicking.
        assert_eq!(*lock_recovered(&mutex), 7);
    }

    #[test]
    fn outbox_push_and_take_round_trip() {
        let bridge = test_bridge();
        assert!(bridge.take_requests().is_empty());
        bridge.push(DebugAction::Refresh);
        bridge.push(DebugAction::TogglePause);
        let requests = bridge.take_requests();
        assert_eq!(requests.len(), 2);
        // take drains: second call sees nothing.
        assert!(bridge.take_requests().is_empty());
    }

    #[test]
    fn set_status_stores_local_message_and_invalidates() {
        let bridge = test_bridge();
        bridge.set_status("parse failed: zz".to_string());
        assert!(matches!(
            lock_recovered(&bridge.display).status,
            StatusMessage::Local(_)
        ));
        assert!(bridge.view_invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn selected_space_and_name_follow_nav_index() {
        let bridge = test_bridge();
        assert_eq!(bridge.selected_space(), Some(SpaceId(0)));
        assert_eq!(bridge.selected_name(), Some("Memory".to_string()));
    }

    #[test]
    fn anchor_uses_pin_without_follow() {
        use nerust_core_traits::debugger::DisasmLine;
        let display = DisplayCache {
            disasm_lines: vec![DisasmLine {
                addr: 0x1234,
                bytes: [0; 4],
                len: 2,
                text: "NOP".to_string(),
                is_pc: true,
                target: None,
            }],
            ..DisplayCache::default()
        };
        let bridge = DebugBridge::new(display, vec![], 0, AppLanguage::English);
        // Pinning an address leaves follow mode: the stored pin wins
        // over the first visible line.
        lock_recovered(&bridge.nav).navigate(0x1234, 0x0300);
        assert_eq!(bridge.anchor(), 0x0300);
    }

    #[test]
    fn anchor_uses_first_line_in_follow_mode() {
        use nerust_core_traits::debugger::DisasmLine;
        let display = DisplayCache {
            disasm_lines: vec![DisasmLine {
                addr: 0x0800_27D2,
                bytes: [0; 4],
                len: 2,
                text: "POP".to_string(),
                is_pc: true,
                target: None,
            }],
            ..DisplayCache::default()
        };
        let bridge = DebugBridge::new(display, vec![], 0, AppLanguage::English);
        lock_recovered(&bridge.nav).follow();
        assert_eq!(bridge.anchor(), 0x0800_27D2);
    }
}
