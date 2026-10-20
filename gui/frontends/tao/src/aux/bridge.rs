//! Shared bridge between the iced programs and the host drain.
//! One `RwLock` over the whole state: readers share, writers exclude.

use std::sync::{
    Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
    atomic::{AtomicBool, Ordering},
};

use nerust_core_traits::debugger::{HexRow, SpaceId};
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{DisplayCache, NavState, StatusMessage};
use nerust_settings_core::model::language::AppLanguage;

/// Poison-safe mutex lock: recovers the inner value after a panic.
/// Recovered state may be stale until the next drain refresh.
pub(crate) fn lock_recovered<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| {
        log::warn!("mutex recovered after poisoning; state may be stale");
        e.into_inner()
    })
}

/// Poison-safe read lock over the bridge state.
fn read_recovered(m: &RwLock<BridgeState>) -> RwLockReadGuard<'_, BridgeState> {
    m.read().unwrap_or_else(|e| {
        log::warn!("bridge read recovered after poisoning; state may be stale");
        e.into_inner()
    })
}

/// Poison-safe write lock over the bridge state.
fn write_recovered(m: &RwLock<BridgeState>) -> RwLockWriteGuard<'_, BridgeState> {
    m.write().unwrap_or_else(|e| {
        log::warn!("bridge write recovered after poisoning; state may be stale");
        e.into_inner()
    })
}

/// Narrow drain snapshot: nav, anchors, watch/freeze/reg-target,
/// previous rows, plus the PPU-open flag.
#[derive(Debug, Clone)]
pub(crate) struct DrainSnapshot {
    pub space: Option<SpaceId>,
    pub mem_addr: u32,
    pub dis_addr: Option<u32>,
    pub watch: Option<u32>,
    pub freeze: Option<(u32, u8)>,
    pub reg_target: Option<&'static str>,
    pub prev_rows: std::sync::Arc<[HexRow]>,
    pub ppu_open: bool,
}

/// All mutable bridge state under one lock: readers share it.
pub(crate) struct BridgeState {
    pub nav: NavState,
    pub display: DisplayCache,
    pub bookmarks: Vec<(u32, String)>,
    pub bm_input: String,
    pub watch: Option<u32>,
    pub freeze: Option<(u32, u8)>,
    pub freeze_input: String,
    pub selected: Option<(u32, Option<u8>)>,
    /// New-value input for the two-phase write (Phase E).
    pub edit_input: String,
    /// Register write target (kernel name) plus hex input. Mirrors
    /// the memory edit row: pick a register, type a value, Set.
    pub reg_target: Option<&'static str>,
    pub reg_input: String,
    pub ppu_hover: String,
}

/// Immutable bridge metadata: fixed at open time, lock-free.
struct BridgeMeta {
    spaces: Vec<(SpaceId, String, u32)>,
    language: AppLanguage,
}

pub(crate) struct DebugBridge {
    state: RwLock<BridgeState>,
    meta: BridgeMeta,
    outbox: Mutex<Vec<DebugAction>>,
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
            state: RwLock::new(BridgeState {
                nav: NavState::new(spaces.len(), mem_addr),
                display,
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
            }),
            meta: BridgeMeta { spaces, language },
            outbox: Mutex::new(Vec::new()),
            view_invalidated: AtomicBool::new(false),
            ppu_invalidated: AtomicBool::new(false),
            open_ppu: AtomicBool::new(false),
        }
    }

    /// Shared read access over the whole state.
    pub(crate) fn read(&self) -> RwLockReadGuard<'_, BridgeState> {
        read_recovered(&self.state)
    }

    /// Exclusive write access over the whole state.
    pub(crate) fn write(&self) -> RwLockWriteGuard<'_, BridgeState> {
        write_recovered(&self.state)
    }

    /// UI language snapshot at open time.
    pub(crate) fn language(&self) -> AppLanguage {
        self.meta.language
    }

    /// Address spaces for viewer creation.
    pub(crate) fn spaces(&self) -> &[(SpaceId, String, u32)] {
        &self.meta.spaces
    }

    /// Narrow drain snapshot: one shared read, then small field copies.
    pub(crate) fn drain_snapshot(&self, ppu_open: bool) -> DrainSnapshot {
        let state = self.read();
        let space = self
            .meta
            .spaces
            .get(state.nav.space_idx())
            .map(|(id, _, _)| *id);
        DrainSnapshot {
            space,
            mem_addr: state.nav.mem_addr(),
            dis_addr: state.nav.dis_addr(),
            watch: state.watch,
            freeze: state.freeze,
            reg_target: state.reg_target,
            prev_rows: state.display.dump_rows.clone().into(),
            ppu_open,
        }
    }

    /// Apply a finished live-images fetch: images land when newer, chrome only while running.
    /// Returns whether the viewer needs a rebuild.
    pub(crate) fn apply_live_images(
        &self,
        generation: u64,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        running: bool,
    ) -> bool {
        let mut state = self.write();
        if state.display.images_gen == generation {
            return false;
        }
        state.display.images = images;
        state.display.images_gen = generation;
        if running {
            state.display.paused = false;
            state.display.status = nerust_gui_viewmodel::debugger::StatusMessage::Running;
        }
        true
    }

    /// Apply a finished drain display: selection write plus paused-section restore.
    pub(crate) fn apply_drain_display(
        &self,
        mut display: DisplayCache,
        reselected: Option<(u32, Option<u8>)>,
    ) {
        let mut state = self.write();
        if let Some(selected) = reselected {
            state.selected = Some(selected);
        }
        let old = state.display.clone_without_images();
        display.restore_paused_sections(&old);
        state.display = display;
    }

    /// Non-blocking contention audit: names of contended locks (empty when healthy).
    pub(crate) fn try_lock_audit(&self) -> Vec<&'static str> {
        let mut contended = Vec::new();
        if self.state.try_read().is_err() {
            contended.push("state");
        }
        if self.outbox.try_lock().is_err() {
            contended.push("outbox");
        }
        contended
    }

    pub(crate) fn selected_name(&self) -> Option<String> {
        let state = self.read();
        self.meta
            .spaces
            .get(state.nav.space_idx())
            .map(|(_, name, _)| name.clone())
    }

    pub(crate) fn set_status(&self, status: String) {
        // Frontend-local wording only: drain outcomes never pass through here.
        self.write().display.status = StatusMessage::Local(status);
        self.view_invalidated.store(true, Ordering::Release);
    }

    pub(crate) fn take_requests(&self) -> Vec<DebugAction> {
        std::mem::take(&mut *lock_recovered(&self.outbox))
    }

    /// Queued request depth without draining (test read).
    #[cfg(test)]
    pub(crate) fn outbox_len(&self) -> usize {
        lock_recovered(&self.outbox).len()
    }

    pub(crate) fn push(&self, request: DebugAction) {
        lock_recovered(&self.outbox).push(request);
        self.view_invalidated.store(true, Ordering::Release);
    }

    /// Displayed disassembly anchor: first visible line in follow-PC
    /// mode (the stored pin is stale there), else the pin.
    pub(crate) fn anchor(&self) -> u32 {
        let state = self.read();
        if state.nav.follow_pc() {
            state
                .display
                .disasm_lines
                .first()
                .map(|line| line.addr)
                .unwrap_or(0)
        } else {
            state.nav.dis_addr().unwrap_or(0)
        }
    }
}

/// Narrow host-drain port over [`DebugBridge`]: snapshot plus batch take.
pub(crate) trait BridgeLock {
    fn drain_snapshot(&self, ppu_open: bool) -> DrainSnapshot;
    fn take_debug_batch(&self) -> (bool, Vec<DebugAction>);
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
    fn single_lock_covers_all_state() {
        let bridge = test_bridge();
        // One shared read sees nav, display, and inputs coherently.
        let state = bridge.read();
        assert!(state.bookmarks.is_empty());
        assert!(state.watch.is_none());
        drop(state);
        // One exclusive write mutates several fields atomically.
        let mut state = bridge.write();
        state.watch = Some(0x100);
        state.bm_input = "mark".to_string();
        drop(state);
        assert_eq!(bridge.read().watch, Some(0x100));
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
    fn state_recovers_value_after_poison() {
        let bridge = test_bridge();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = bridge.state.write().unwrap();
            panic!("intentional poison");
        }));
        assert!(bridge.state.is_poisoned());
        assert!(bridge.read().bookmarks.is_empty());
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
            bridge.read().display.status,
            StatusMessage::Local(_)
        ));
        assert!(bridge.view_invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn drain_snapshot_space_and_name_follow_nav_index() {
        let bridge = test_bridge();
        assert_eq!(bridge.drain_snapshot(false).space, Some(SpaceId(0)));
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
        bridge.write().nav.navigate(0x1234, 0x0300);
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
        bridge.write().nav.follow();
        assert_eq!(bridge.anchor(), 0x0800_27D2);
    }

    #[test]
    fn live_images_apply_only_when_newer() {
        let bridge = test_bridge();
        // Same generation: images untouched, no rebuild.
        assert!(!bridge.apply_live_images(0, Vec::new(), true));
        // Newer while running: images land, chrome flips to Running.
        let images = vec![("Tiles".to_string(), 2, 1, vec![0xFF; 8])];
        assert!(bridge.apply_live_images(3, images, true));
        assert_eq!(bridge.read().display.images_gen, 3);
        assert!(matches!(
            bridge.read().display.status,
            StatusMessage::Running
        ));
        // Newer while paused: images land, status untouched.
        bridge.write().display.status = StatusMessage::Local("held".to_string());
        assert!(bridge.apply_live_images(4, Vec::new(), false));
        assert!(bridge.read().display.images.is_empty());
        assert!(matches!(
            bridge.read().display.status,
            StatusMessage::Local(_)
        ));
    }

    #[test]
    fn drain_display_applies_selection_and_restores() {
        let bridge = test_bridge();
        // Reselected row lands alongside the fresh display.
        bridge.apply_drain_display(DisplayCache::default(), Some((0x120, Some(0xAB))));
        assert_eq!(bridge.read().selected, Some((0x120, Some(0xAB))));
        // No reselection: previous selection survives.
        bridge.apply_drain_display(DisplayCache::default(), None);
        assert_eq!(bridge.read().selected, Some((0x120, Some(0xAB))));
    }

    #[test]
    fn drain_snapshot_carries_flags_and_none_space() {
        // Empty spaces: no space, anchor falls back to zero.
        let bridge = test_bridge();
        let snap = bridge.drain_snapshot(true);
        assert_eq!(snap.space, Some(SpaceId(0)));
        assert!(snap.ppu_open);
        assert!(snap.prev_rows.is_empty());
        let coreless = DebugBridge::new(DisplayCache::default(), vec![], 0, AppLanguage::English);
        let snap = coreless.drain_snapshot(false);
        assert_eq!(snap.space, None);
        assert!(!snap.ppu_open);
        assert_eq!(coreless.selected_name(), None);
        assert_eq!(coreless.anchor(), 0);
    }

    #[test]
    fn audit_names_contended_locks() {
        let bridge = test_bridge();
        // Held write guard blocks the shared read.
        let _write = bridge.write();
        assert!(bridge.try_lock_audit().contains(&"state"));
    }

    #[test]
    fn audit_names_contended_outbox() {
        let bridge = test_bridge();
        // Held outbox guard surfaces while state stays free.
        let _guard = bridge.outbox.lock().unwrap();
        assert_eq!(bridge.try_lock_audit(), vec!["outbox"]);
    }

    #[test]
    fn bridge_lock_trait_swaps_ppu_flag_and_batch() {
        use super::BridgeLock;
        let bridge = test_bridge();
        bridge.open_ppu.store(true, Ordering::Release);
        bridge.push(DebugAction::Refresh);
        let lock: &dyn BridgeLock = &bridge;
        let (open, batch) = lock.take_debug_batch();
        assert!(open);
        assert_eq!(batch.len(), 1);
        // Swap consumes: second take sees a closed flag.
        assert!(!lock.take_debug_batch().0);
        assert!(lock.drain_snapshot(false).mem_addr == 0);
    }

    #[test]
    fn language_and_spaces_read_from_meta() {
        let bridge = test_bridge();
        assert_eq!(bridge.language(), AppLanguage::English);
        assert_eq!(bridge.spaces().len(), 1);
        assert_eq!(bridge.spaces()[0].0, SpaceId(0));
    }
}
