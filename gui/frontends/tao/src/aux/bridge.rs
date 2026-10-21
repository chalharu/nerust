//! Shared bridge: nav plus inputs under one mutex, display under an `Arc` swap.

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

/// Poison-safe read lock over the display swap.
fn read_display(
    m: &RwLock<std::sync::Arc<DisplayCache>>,
) -> RwLockReadGuard<'_, std::sync::Arc<DisplayCache>> {
    m.read().unwrap_or_else(|e| {
        log::warn!("bridge display recovered after poisoning; state may be stale");
        e.into_inner()
    })
}

/// Poison-safe write lock over the display swap.
fn write_display(
    m: &RwLock<std::sync::Arc<DisplayCache>>,
) -> RwLockWriteGuard<'_, std::sync::Arc<DisplayCache>> {
    m.write().unwrap_or_else(|e| {
        log::warn!("bridge display recovered after poisoning; state may be stale");
        e.into_inner()
    })
}

/// Poison-safe core lock: recovers the inner value after a panic.
fn lock_core(m: &Mutex<BridgeCore>) -> MutexGuard<'_, BridgeCore> {
    lock_recovered(m)
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

/// Coherent view-model read: one core lock plus one display `Arc` clone.
/// Views render from this, never from live guards.
#[derive(Debug, Clone)]
pub(crate) struct BridgeView {
    pub nav: NavState,
    pub display: DisplayCache,
    pub bookmarks: Vec<(u32, String)>,
    pub bm_input: String,
    pub watch: Option<u32>,
    pub freeze: Option<(u32, u8)>,
    pub freeze_input: String,
    pub selected: Option<(u32, Option<u8>)>,
    pub edit_input: String,
    pub reg_target: Option<&'static str>,
    pub reg_input: String,
}

/// Nav plus small inputs under one mutex. Display images live separately
/// under the `Arc` swap; every field here is small and cheap to clone.
struct BridgeCore {
    nav: NavState,
    bookmarks: Vec<(u32, String)>,
    bm_input: String,
    watch: Option<u32>,
    freeze: Option<(u32, u8)>,
    freeze_input: String,
    selected: Option<(u32, Option<u8>)>,
    /// New-value input for the two-phase write (Phase E).
    edit_input: String,
    /// Register write target (kernel name) plus hex input. Mirrors
    /// the memory edit row: pick a register, type a value, Set.
    reg_target: Option<&'static str>,
    reg_input: String,
    ppu_hover: String,
}

/// Immutable bridge metadata: fixed at open time, lock-free.
struct BridgeMeta {
    spaces: Vec<(SpaceId, String, u32)>,
    language: AppLanguage,
}

pub(crate) struct DebugBridge {
    core: Mutex<BridgeCore>,
    display: RwLock<std::sync::Arc<DisplayCache>>,
    meta: BridgeMeta,
    outbox: Mutex<Vec<DebugAction>>,
    epoch: Mutex<Option<std::sync::Arc<AtomicBool>>>,
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
            core: Mutex::new(BridgeCore {
                nav: NavState::new(spaces.len(), mem_addr),
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
            display: RwLock::new(std::sync::Arc::new(display)),
            meta: BridgeMeta { spaces, language },
            outbox: Mutex::new(Vec::new()),
            epoch: Mutex::new(None),
            view_invalidated: AtomicBool::new(false),
            ppu_invalidated: AtomicBool::new(false),
            open_ppu: AtomicBool::new(false),
        }
    }

    /// UI language snapshot at open time.
    pub(crate) fn language(&self) -> AppLanguage {
        self.meta.language
    }

    /// Address spaces for viewer creation.
    pub(crate) fn spaces(&self) -> &[(SpaceId, String, u32)] {
        &self.meta.spaces
    }

    /// Coherent view-model read: one core lock plus one display `Arc`.
    /// Small inputs clone under the core guard; images never clone here.
    pub(crate) fn view_snapshot(&self) -> BridgeView {
        let core = lock_core(&self.core);
        let display = std::sync::Arc::clone(&read_display(&self.display));
        BridgeView {
            nav: core.nav.clone(),
            display: display.clone_without_images(),
            bookmarks: core.bookmarks.clone(),
            bm_input: core.bm_input.clone(),
            watch: core.watch,
            freeze: core.freeze,
            freeze_input: core.freeze_input.clone(),
            selected: core.selected,
            edit_input: core.edit_input.clone(),
            reg_target: core.reg_target,
            reg_input: core.reg_input.clone(),
        }
    }

    /// Display `Arc` snapshot: `Arc::clone` under a read guard, never a vector clone.
    pub(crate) fn display_snapshot(&self) -> std::sync::Arc<DisplayCache> {
        std::sync::Arc::clone(&read_display(&self.display))
    }

    /// Narrow drain snapshot: nav plus inputs under one short core lock,
    /// previous rows off the display `Arc` (no vector clone under guard).
    pub(crate) fn drain_snapshot(&self, ppu_open: bool) -> DrainSnapshot {
        let core = lock_core(&self.core);
        let space = self
            .meta
            .spaces
            .get(core.nav.space_idx())
            .map(|(id, _, _)| *id);
        let snapshot = DrainSnapshot {
            space,
            mem_addr: core.nav.mem_addr(),
            dis_addr: core.nav.dis_addr(),
            watch: core.watch,
            freeze: core.freeze,
            reg_target: core.reg_target,
            prev_rows: std::sync::Arc::from([]),
            ppu_open,
        };
        drop(core);
        let display = self.display_snapshot();
        DrainSnapshot {
            prev_rows: display.dump_rows.clone().into(),
            ..snapshot
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
        let mut display = write_display(&self.display);
        if display.images_gen == generation {
            return false;
        }
        let display = std::sync::Arc::make_mut(&mut display);
        display.images = images;
        display.images_gen = generation;
        if running {
            display.paused = false;
            display.status = nerust_gui_viewmodel::debugger::StatusMessage::Running;
        }
        true
    }

    /// Apply a finished drain display: selection write plus paused-section restore.
    pub(crate) fn apply_drain_display(
        &self,
        mut display: DisplayCache,
        reselected: Option<(u32, Option<u8>)>,
    ) {
        if let Some(selected) = reselected {
            lock_core(&self.core).selected = Some(selected);
        }
        let old = self.display_snapshot();
        display.restore_paused_sections(&old);
        *write_display(&self.display) = std::sync::Arc::new(display);
    }

    /// Run one display mutation through the swap (clone-on-write; rare paths only).
    pub(crate) fn update_display(&self, f: impl FnOnce(&mut DisplayCache)) {
        let mut display = write_display(&self.display);
        f(std::sync::Arc::make_mut(&mut display));
    }

    /// Non-blocking contention audit: names of contended locks (empty when healthy).
    pub(crate) fn try_lock_audit(&self) -> Vec<&'static str> {
        let mut contended = Vec::new();
        if self.core.try_lock().is_err() {
            contended.push("core");
        }
        if self.display.try_read().is_err() {
            contended.push("display");
        }
        if self.outbox.try_lock().is_err() {
            contended.push("outbox");
        }
        contended
    }

    pub(crate) fn selected_name(&self) -> Option<String> {
        let core = lock_core(&self.core);
        self.meta
            .spaces
            .get(core.nav.space_idx())
            .map(|(_, name, _)| name.clone())
    }

    pub(crate) fn set_status(&self, status: String) {
        // Frontend-local wording only: drain outcomes never pass through here.
        self.update_display(|display| {
            display.status = StatusMessage::Local(status);
        });
        self.view_invalidated.store(true, Ordering::Release);
    }

    pub(crate) fn take_requests(&self) -> Vec<DebugAction> {
        std::mem::take(&mut *lock_recovered(&self.outbox))
    }

    /// Bind UI requests to the runtime whose data the editor displays. Drop
    /// unclaimed requests and persistent writes before accepting a new epoch.
    pub(crate) fn bind_epoch(&self, next: std::sync::Arc<AtomicBool>) {
        let mut epoch = lock_recovered(&self.epoch);
        if epoch
            .as_ref()
            .is_some_and(|old| !std::sync::Arc::ptr_eq(old, &next))
        {
            lock_recovered(&self.outbox).clear();
            let mut core = lock_core(&self.core);
            core.watch = None;
            core.freeze = None;
            core.selected = None;
            core.reg_target = None;
            drop(core);
            *write_display(&self.display) = std::sync::Arc::new(DisplayCache::default());
        }
        *epoch = Some(next);
    }

    /// Queued request depth without draining (test read).
    #[cfg(test)]
    pub(crate) fn outbox_len(&self) -> usize {
        lock_recovered(&self.outbox).len()
    }

    pub(crate) fn push(&self, request: DebugAction) {
        let epoch = lock_recovered(&self.epoch);
        if epoch
            .as_ref()
            .is_some_and(|epoch| !epoch.load(Ordering::Acquire))
        {
            return;
        }
        lock_recovered(&self.outbox).push(request);
        self.view_invalidated.store(true, Ordering::Release);
    }

    /// Displayed disassembly anchor: first visible line in follow-PC
    /// mode (the stored pin is stale there), else the pin.
    pub(crate) fn anchor(&self) -> u32 {
        let core = lock_core(&self.core);
        let follow = core.nav.follow_pc();
        let pin = core.nav.dis_addr().unwrap_or(0);
        drop(core);
        if follow {
            self.display_snapshot()
                .disasm_lines
                .first()
                .map(|line| line.addr)
                .unwrap_or(0)
        } else {
            pin
        }
    }

    /// First PC-marked disassembly row, for bookmark anchoring.
    pub(crate) fn pc_row_addr(&self) -> Option<u32> {
        self.display_snapshot()
            .disasm_lines
            .iter()
            .find(|line| line.is_pc)
            .map(|line| line.addr)
    }

    // --- Nav ports: one core lock per message. ---

    /// Harness-only read; production paths use `view_snapshot`.
    #[cfg(test)]
    /// Memory address of the dump view.
    pub(crate) fn mem_addr(&self) -> u32 {
        lock_core(&self.core).nav.mem_addr()
    }

    /// Harness-only read; production paths use `view_snapshot`.
    #[cfg(test)]
    /// Whether the disassembly follows the PC.
    pub(crate) fn following_pc(&self) -> bool {
        lock_core(&self.core).nav.follow_pc()
    }

    /// Harness staging for a selected row (production selects via drain reselect).
    #[cfg(test)]
    pub(crate) fn set_selected_for_test(&self, selected: Option<(u32, Option<u8>)>) {
        lock_core(&self.core).selected = selected;
    }

    /// Harness staging for a pending write (production stages via drain).
    #[cfg(test)]
    pub(crate) fn set_pending_for_test(
        &self,
        pending: Option<nerust_gui_viewmodel::debugger::PendingWrite>,
    ) {
        self.update_display(|display| display.pending = pending);
    }

    /// Memory address input text.
    pub(crate) fn mem_input(&self) -> String {
        lock_core(&self.core).nav.mem_input().to_string()
    }

    /// Stage memory address input text.
    pub(crate) fn set_mem_input(&self, text: String) {
        lock_core(&self.core).nav.set_mem_input(text);
    }

    /// Disassembly address input text.
    pub(crate) fn dis_input(&self) -> String {
        lock_core(&self.core).nav.dis_input().to_string()
    }

    /// Stage disassembly address input text.
    pub(crate) fn set_dis_input(&self, text: String) {
        lock_core(&self.core).nav.set_dis_input(text);
    }

    /// Re-enable follow-PC mode.
    pub(crate) fn follow_pc(&self) {
        lock_core(&self.core).nav.follow();
    }

    /// Jump memory view to an address.
    pub(crate) fn go_mem(&self, addr: u32) {
        lock_core(&self.core).nav.go_mem(addr);
    }

    /// Page the memory view.
    pub(crate) fn page_mem(&self, delta: i32) {
        lock_core(&self.core).nav.page_mem(delta);
    }

    /// Navigate the disassembly view from an anchor to an address.
    pub(crate) fn navigate(&self, anchor: u32, addr: u32) {
        lock_core(&self.core).nav.navigate(anchor, addr);
    }

    /// Step back in disassembly history.
    pub(crate) fn go_back(&self) -> Option<u32> {
        lock_core(&self.core).nav.go_back()
    }

    /// Cycle the address space: index into [`Self::spaces`], if it moved.
    pub(crate) fn cycle_space_idx(&self, delta: i32) -> Option<usize> {
        lock_core(&self.core).nav.cycle_space(delta)
    }

    /// Select an address space by index plus its start address.
    pub(crate) fn set_space(&self, idx: usize, start: u32) {
        let mut core = lock_core(&self.core);
        core.nav.set_space(idx, start);
        core.selected = None;
    }

    // --- Selection ports. ---

    /// Selected row address, if any.
    pub(crate) fn selected_addr(&self) -> Option<u32> {
        lock_core(&self.core).selected.map(|(addr, _)| addr)
    }

    /// Selected row plus byte, if any.
    pub(crate) fn selected(&self) -> Option<(u32, Option<u8>)> {
        lock_core(&self.core).selected
    }

    /// Harness-only read; production paths use `view_snapshot`.
    #[cfg(test)]
    /// Clear the selected row.
    pub(crate) fn clear_selection(&self) {
        lock_core(&self.core).selected = None;
    }

    // --- Watch/freeze ports: single write per message. ---

    /// Harness-only read; production paths use `view_snapshot`.
    #[cfg(test)]
    /// Watch address, if any.
    pub(crate) fn watch(&self) -> Option<u32> {
        lock_core(&self.core).watch
    }

    /// Arm a watch on an address.
    pub(crate) fn set_watch(&self, addr: u32) {
        lock_core(&self.core).watch = Some(addr);
    }

    /// Clear the watch plus its stale display value in one pass.
    /// Lock order is core-then-display everywhere a message takes both.
    pub(crate) fn clear_watch(&self) {
        lock_core(&self.core).watch = None;
        self.update_display(|display| display.watch_value = None);
    }

    /// Harness-only read; production paths use `view_snapshot`.
    #[cfg(test)]
    /// Freeze target, if any.
    pub(crate) fn freeze(&self) -> Option<(u32, u8)> {
        lock_core(&self.core).freeze
    }

    /// Freeze input text.
    pub(crate) fn freeze_input(&self) -> String {
        lock_core(&self.core).freeze_input.clone()
    }

    /// Stage freeze input text.
    pub(crate) fn set_freeze_input(&self, text: String) {
        lock_core(&self.core).freeze_input = text;
    }

    /// Freeze a byte at an address.
    pub(crate) fn set_freeze(&self, addr: u32, value: u8) {
        lock_core(&self.core).freeze = Some((addr, value));
    }

    /// Clear the freeze target.
    pub(crate) fn clear_freeze(&self) {
        lock_core(&self.core).freeze = None;
    }

    /// Edit input text.
    pub(crate) fn edit_input(&self) -> String {
        lock_core(&self.core).edit_input.clone()
    }

    /// Stage edit input text.
    pub(crate) fn set_edit_input(&self, text: String) {
        lock_core(&self.core).edit_input = text;
    }

    /// Clear the edit input (confirm path consumes the transaction).
    pub(crate) fn clear_edit_input(&self) {
        lock_core(&self.core).edit_input = String::new();
    }

    /// Clear a staged write (cancel path drops the pending row).
    pub(crate) fn clear_pending_write(&self) {
        self.update_display(|display| display.pending = None);
    }

    /// Register write target, if any.
    pub(crate) fn reg_target(&self) -> Option<&'static str> {
        lock_core(&self.core).reg_target
    }

    /// Pick the register write target.
    pub(crate) fn set_reg_target(&self, target: Option<&'static str>) {
        lock_core(&self.core).reg_target = target;
    }

    /// Register input text.
    pub(crate) fn reg_input(&self) -> String {
        lock_core(&self.core).reg_input.clone()
    }

    /// Stage register input text.
    pub(crate) fn set_reg_input(&self, text: String) {
        lock_core(&self.core).reg_input = text;
    }

    // --- Bookmark ports. ---

    /// Staged bookmark label (trimmed).
    pub(crate) fn bookmark_label(&self) -> String {
        lock_core(&self.core).bm_input.trim().to_string()
    }

    /// Stage bookmark label text.
    pub(crate) fn set_bookmark_input(&self, text: String) {
        lock_core(&self.core).bm_input = text;
    }

    /// Bookmarks snapshot for the view.
    pub(crate) fn bookmarks(&self) -> Vec<(u32, String)> {
        lock_core(&self.core).bookmarks.clone()
    }

    /// Push a bookmark; true when the table is full (nothing pushed).
    pub(crate) fn push_bookmark(&self, addr: u32, label: String, cap: usize) -> bool {
        let mut core = lock_core(&self.core);
        if core.bookmarks.len() >= cap {
            return true;
        }
        core.bookmarks.push((addr, label));
        core.bm_input.clear();
        false
    }

    /// Delete the bookmark at an address.
    pub(crate) fn remove_bookmark(&self, addr: u32) {
        lock_core(&self.core).bookmarks.retain(|(a, _)| *a != addr);
    }

    // --- Hover port. ---

    /// Harness-only read; production paths use `view_snapshot`.
    #[cfg(test)]
    /// Last hovered PPU line.
    pub(crate) fn hover(&self) -> String {
        lock_core(&self.core).ppu_hover.clone()
    }

    /// Record the hovered PPU line.
    pub(crate) fn set_hover(&self, line: String) {
        lock_core(&self.core).ppu_hover = line;
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
    fn split_locks_cover_all_state() {
        let bridge = test_bridge();
        // One coherent view read sees nav, inputs, and display together.
        let view = bridge.view_snapshot();
        assert!(view.bookmarks.is_empty());
        assert!(view.watch.is_none());
        bridge.set_watch(0x100);
        bridge.set_bookmark_input("mark".to_string());
        assert_eq!(bridge.watch(), Some(0x100));
        let view = bridge.view_snapshot();
        assert_eq!(view.watch, Some(0x100));
        assert_eq!(view.bm_input, "mark");
    }

    #[test]
    fn display_snapshot_shares_without_cloning() {
        let bridge = test_bridge();
        // `Arc::clone` under a read guard: two snapshots alias.
        let first = bridge.display_snapshot();
        let second = bridge.display_snapshot();
        assert!(std::sync::Arc::ptr_eq(&first, &second));
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
            let _guard = bridge.core.lock().unwrap();
            panic!("intentional poison");
        }));
        assert!(bridge.core.is_poisoned());
        assert!(bridge.view_snapshot().bookmarks.is_empty());
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
            bridge.display_snapshot().status,
            StatusMessage::Local(_)
        ));
        assert!(bridge.view_invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn replacement_discards_unclaimed_ui_actions_and_freeze() {
        let bridge = test_bridge();
        let epoch = std::sync::Arc::new(AtomicBool::new(true));
        bridge.bind_epoch(epoch.clone());
        bridge.set_freeze(0x10, 0xff);
        bridge.push(DebugAction::RegisterWrite(7));
        epoch.store(false, Ordering::Release);
        bridge.push(DebugAction::Resume);
        assert_eq!(
            bridge.outbox_len(),
            1,
            "invalid epoch rejects new submissions"
        );
        bridge.bind_epoch(std::sync::Arc::new(AtomicBool::new(true)));
        assert!(bridge.take_requests().is_empty());
        assert!(bridge.freeze().is_none());
        bridge.push(DebugAction::Refresh);
        assert_eq!(bridge.take_requests().len(), 1);
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
        bridge.navigate(0x1234, 0x0300);
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
        // Fresh nav follows by default: the first visible line wins over the pin.
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
        assert_eq!(bridge.display_snapshot().images_gen, 3);
        assert!(matches!(
            bridge.display_snapshot().status,
            StatusMessage::Running
        ));
        // Newer while paused: images land, status untouched.
        bridge.update_display(|display| {
            display.status = StatusMessage::Local("held".to_string());
        });
        assert!(bridge.apply_live_images(4, Vec::new(), false));
        assert!(bridge.display_snapshot().images.is_empty());
        assert!(matches!(
            bridge.display_snapshot().status,
            StatusMessage::Local(_)
        ));
    }

    #[test]
    fn drain_display_applies_selection_and_restores() {
        let bridge = test_bridge();
        // Reselected row lands alongside the fresh display.
        bridge.apply_drain_display(DisplayCache::default(), Some((0x120, Some(0xAB))));
        assert_eq!(bridge.selected(), Some((0x120, Some(0xAB))));
        // No reselection: previous selection survives.
        bridge.apply_drain_display(DisplayCache::default(), None);
        assert_eq!(bridge.selected(), Some((0x120, Some(0xAB))));
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
        // Held core guard blocks coherent reads.
        let _core = lock_core(&bridge.core);
        assert!(bridge.try_lock_audit().contains(&"core"));
    }

    #[test]
    fn audit_names_contended_display() {
        let bridge = test_bridge();
        // Held display guard surfaces while core stays free.
        let _display = write_display(&bridge.display);
        assert_eq!(bridge.try_lock_audit(), vec!["display"]);
    }

    #[test]
    fn audit_names_contended_outbox() {
        let bridge = test_bridge();
        // Held outbox guard surfaces while core stays free.
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

    #[test]
    fn single_write_ports_cover_inputs_and_nav() {
        let bridge = test_bridge();
        bridge.go_mem(0x100);
        assert_eq!(bridge.mem_input(), "0x00000100");
        bridge.page_mem(1);
        assert_eq!(bridge.mem_input(), "0x00000101");
        bridge.set_space(0, 0);
        // Single-space table: cycling wraps back to the same index.
        assert_eq!(bridge.cycle_space_idx(1), Some(0));
        bridge.navigate(0x100, 0x120);
        // Navigate pushed the anchor: Back pops it exactly once.
        assert_eq!(bridge.go_back(), Some(0x100));
        assert!(bridge.go_back().is_none());
        bridge.set_watch(0x10);
        assert_eq!(bridge.watch(), Some(0x10));
        bridge.clear_watch();
        assert!(bridge.watch().is_none());
        bridge.set_freeze(0x10, 0xFF);
        assert_eq!(bridge.freeze(), Some((0x10, 0xFF)));
        bridge.clear_freeze();
        assert!(bridge.freeze().is_none());
        bridge.set_freeze_input("FF".to_string());
        assert_eq!(bridge.freeze_input(), "FF");
        bridge.set_edit_input("42".to_string());
        assert_eq!(bridge.edit_input(), "42");
        bridge.clear_edit_input();
        assert!(bridge.edit_input().is_empty());
        bridge.clear_pending_write();
        bridge.set_reg_target(Some("A"));
        assert_eq!(bridge.reg_target(), Some("A"));
        bridge.set_reg_target(None);
        assert_eq!(bridge.reg_target(), None);
        bridge.set_reg_input("7".to_string());
        assert_eq!(bridge.reg_input(), "7");
        bridge.set_hover("tile".to_string());
        assert_eq!(bridge.hover(), "tile");
        bridge.set_bookmark_input("x".to_string());
        assert_eq!(bridge.bookmark_label(), "x");
        assert!(!bridge.push_bookmark(0x10, "x".to_string(), 8));
        assert_eq!(bridge.bookmarks().len(), 1);
        bridge.remove_bookmark(0x10);
        assert!(bridge.bookmarks().is_empty());
        assert!(bridge.push_bookmark(0x10, "x".to_string(), 0));
        // Back restored the popped pin into the disassembly input.
        assert_eq!(bridge.dis_input(), "0x00000100");
        bridge.clear_selection();
        assert!(bridge.selected().is_none());
        assert!(bridge.selected_addr().is_none());
        assert!(bridge.pc_row_addr().is_none());
    }
}
