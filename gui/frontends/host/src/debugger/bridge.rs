//! Shared split-lock storage. Lock order: epoch -> state -> display -> outbox.
//! Immutable metadata never needs a lock; typing never copies the display.

use std::sync::{
    Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard,
    atomic::{AtomicBool, Ordering},
};

use nerust_core_traits::debugger::{HexRow, SpaceId};
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{DebugImageRgba, DisplayCache, NavState, tile_hover_text};
use nerust_settings_core::model::language::AppLanguage;

use super::{DebuggerEvent, DebuggerState, DebuggerUpdate, model::apply_live};
use nerust_frontend_viewdefs::sync::lock_recovered;

fn read_display(lock: &RwLock<Arc<DisplayCache>>) -> RwLockReadGuard<'_, Arc<DisplayCache>> {
    lock.read().unwrap_or_else(|error| {
        log::warn!("bridge display recovered after poisoning; state may be stale");
        error.into_inner()
    })
}

fn write_display(lock: &RwLock<Arc<DisplayCache>>) -> RwLockWriteGuard<'_, Arc<DisplayCache>> {
    lock.write().unwrap_or_else(|error| {
        log::warn!("bridge display recovered after poisoning; state may be stale");
        error.into_inner()
    })
}

#[derive(Debug, Clone)]
pub struct DrainSnapshot {
    pub space: Option<SpaceId>,
    pub mem_addr: u32,
    pub dis_addr: Option<u32>,
    pub watch: Option<u32>,
    pub freeze: Option<(u32, u8)>,
    pub reg_target: Option<&'static str>,
    pub prev_rows: Arc<[HexRow]>,
    pub ppu_open: bool,
}

/// Coherent owned view capture. Image vectors are deliberately excluded.
#[derive(Debug, Clone)]
pub struct BridgeView {
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

struct BridgeMeta {
    spaces: Vec<(SpaceId, String, u32)>,
    language: AppLanguage,
}

pub struct DebugBridge {
    core: Mutex<DebuggerState>,
    display: RwLock<Arc<DisplayCache>>,
    meta: BridgeMeta,
    outbox: Mutex<Vec<DebugAction>>,
    epoch: Mutex<Option<Arc<AtomicBool>>>,
    pub view_invalidated: AtomicBool,
    pub ppu_invalidated: AtomicBool,
    pub open_ppu: AtomicBool,
}

impl DebugBridge {
    pub fn new(
        display: DisplayCache,
        spaces: Vec<(SpaceId, String, u32)>,
        mem_addr: u32,
        language: AppLanguage,
    ) -> Self {
        Self {
            core: Mutex::new(DebuggerState::new(spaces.len(), mem_addr)),
            display: RwLock::new(Arc::new(display)),
            meta: BridgeMeta { spaces, language },
            outbox: Mutex::new(Vec::new()),
            epoch: Mutex::new(None),
            view_invalidated: AtomicBool::new(false),
            ppu_invalidated: AtomicBool::new(false),
            open_ppu: AtomicBool::new(false),
        }
    }

    /// Epoch is acquired first and held until the complete ordered batch lands.
    /// A retired epoch rejects all UI mutations, including input echoes/errors.
    /// `None` permits coreless windows and headless native-view harnesses.
    pub fn dispatch(&self, event: DebuggerEvent) -> DebuggerUpdate {
        let epoch = lock_recovered(&self.epoch);
        if epoch
            .as_ref()
            .is_some_and(|epoch| !epoch.load(Ordering::Acquire))
        {
            return DebuggerUpdate::default();
        }
        let mut core = lock_recovered(&self.core);
        let mut display = write_display(&self.display);
        // Runtime retirement flips the atomic independently of this mutex.
        // Recheck after waiting for state/display, before any UI mutation.
        if epoch
            .as_ref()
            .is_some_and(|epoch| !epoch.load(Ordering::Acquire))
        {
            return DebuggerUpdate::default();
        }
        let reduction = core.reduce(event, &self.meta.spaces, self.meta.language, &display);
        let update = if reduction.changes_display(&display) {
            reduction.apply(Arc::make_mut(&mut display))
        } else {
            reduction.update
        };
        // One outbox lock: consumers and epoch rebinding can never split a
        // SelectRow + EditStage transaction across drains or sessions.
        if !update.actions.is_empty() {
            lock_recovered(&self.outbox).extend_from_slice(&update.actions);
        }
        if update.open_ppu {
            self.open_ppu.store(true, Ordering::Release);
        }
        // Iced still needs to redraw typed text although scope=None tells
        // native incremental views that no lists need rebuilding.
        self.view_invalidated.store(true, Ordering::Release);
        update
    }

    pub fn language(&self) -> AppLanguage {
        self.meta.language
    }

    /// Pre-localized PPU viewer chrome. Frontends render these verbatim
    /// instead of calling into the i18n catalog themselves.
    pub fn ppu_chrome(&self) -> PpuChrome {
        PpuChrome::new(self.meta.language)
    }

    /// Pre-localized debugger chrome. Frontends render it verbatim.
    pub fn debugger_chrome(&self) -> DebuggerChrome {
        DebuggerChrome::new(self.meta.language)
    }

    /// Localized tile hover line for raw view coordinates. Geometry stays
    /// view-side; the language lives here.
    pub fn hover_line(&self, label: &str, width: u32, height: u32, x: f64, y: f64) -> String {
        tile_hover_text(self.meta.language, label, width, height, x, y)
    }

    pub fn spaces(&self) -> &[(SpaceId, String, u32)] {
        &self.meta.spaces
    }

    pub fn view_snapshot(&self) -> BridgeView {
        let core = lock_recovered(&self.core);
        let display = Arc::clone(&read_display(&self.display));
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

    /// Constant-cost image access: only the Arc is cloned under the guard.
    pub fn display_snapshot(&self) -> Arc<DisplayCache> {
        Arc::clone(&read_display(&self.display))
    }

    pub fn drain_snapshot(&self, ppu_open: bool) -> DrainSnapshot {
        let core = lock_recovered(&self.core);
        let display = Arc::clone(&read_display(&self.display));
        let snapshot = DrainSnapshot {
            space: self
                .meta
                .spaces
                .get(core.nav.space_idx())
                .map(|(id, _, _)| *id),
            mem_addr: core.nav.mem_addr(),
            dis_addr: core.nav.dis_addr(),
            watch: core.watch,
            freeze: core.freeze,
            reg_target: core.reg_target,
            prev_rows: Arc::from([]),
            ppu_open,
        };
        drop(core);
        DrainSnapshot {
            prev_rows: display.dump_rows.clone().into(),
            ..snapshot
        }
    }

    pub fn apply_live_images(
        &self,
        generation: u64,
        images: Vec<DebugImageRgba>,
        running: bool,
    ) -> bool {
        let mut display = write_display(&self.display);
        if display.images_gen == generation {
            return false;
        }
        apply_live(Arc::make_mut(&mut display), generation, images, running);
        true
    }

    pub fn apply_drain_display(
        &self,
        mut display: DisplayCache,
        reselected: Option<(u32, Option<u8>)>,
    ) {
        let mut core = lock_recovered(&self.core);
        let mut old = write_display(&self.display);
        if let Some(selected) = reselected {
            core.selected = Some(selected);
        }
        display.restore_paused_sections(&old);
        *old = Arc::new(display);
    }

    /// Cold display update port, also used by native PPU-view tests.
    pub fn update_display(&self, f: impl FnOnce(&mut DisplayCache)) {
        let mut display = write_display(&self.display);
        f(Arc::make_mut(&mut display));
    }

    pub fn try_lock_audit(&self) -> Vec<&'static str> {
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

    pub fn selected_name(&self) -> Option<String> {
        let core = lock_recovered(&self.core);
        self.meta
            .spaces
            .get(core.nav.space_idx())
            .map(|(_, name, _)| name.clone())
    }

    pub fn take_requests(&self) -> Vec<DebugAction> {
        std::mem::take(&mut *lock_recovered(&self.outbox))
    }

    /// Runtime replacement discards unclaimed actions and persistent writes.
    /// Metadata is immutable: callers reopen the window when its spaces change.
    pub fn bind_epoch(&self, next: Arc<AtomicBool>) {
        let mut epoch = lock_recovered(&self.epoch);
        if epoch.as_ref().is_some_and(|old| !Arc::ptr_eq(old, &next)) {
            let mut core = lock_recovered(&self.core);
            let mut display = write_display(&self.display);
            lock_recovered(&self.outbox).clear();
            core.watch = None;
            core.freeze = None;
            core.selected = None;
            core.reg_target = None;
            *display = Arc::new(DisplayCache::default());
            self.open_ppu.store(false, Ordering::Release);
            self.view_invalidated.store(true, Ordering::Release);
            self.ppu_invalidated.store(true, Ordering::Release);
        }
        *epoch = Some(next);
    }

    /// Narrow raw-action port for host open/pause and keyboard shortcuts.
    pub fn push(&self, request: DebugAction) {
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

    pub fn anchor(&self) -> u32 {
        let core = lock_recovered(&self.core);
        core.anchor(&read_display(&self.display))
    }

    /// PPU hover is small state; no display mutation or image copy.
    pub fn set_hover(&self, line: String) {
        let epoch = lock_recovered(&self.epoch);
        if epoch
            .as_ref()
            .is_none_or(|epoch| epoch.load(Ordering::Acquire))
        {
            lock_recovered(&self.core).ppu_hover = line;
        }
    }

    // Cross-crate native-view harness compatibility. These helpers remain
    // available when the host is built as a dependency (without cfg(test)).
    #[doc(hidden)]
    pub fn outbox_len(&self) -> usize {
        lock_recovered(&self.outbox).len()
    }
    #[doc(hidden)]
    pub fn mem_addr(&self) -> u32 {
        lock_recovered(&self.core).nav.mem_addr()
    }
    #[doc(hidden)]
    pub fn following_pc(&self) -> bool {
        lock_recovered(&self.core).nav.follow_pc()
    }
    #[doc(hidden)]
    pub fn selected(&self) -> Option<(u32, Option<u8>)> {
        lock_recovered(&self.core).selected
    }
    #[doc(hidden)]
    pub fn watch(&self) -> Option<u32> {
        lock_recovered(&self.core).watch
    }
    #[doc(hidden)]
    pub fn freeze(&self) -> Option<(u32, u8)> {
        lock_recovered(&self.core).freeze
    }
    #[doc(hidden)]
    pub fn bookmarks(&self) -> Vec<(u32, String)> {
        lock_recovered(&self.core).bookmarks.clone()
    }
    #[doc(hidden)]
    pub fn edit_input(&self) -> String {
        lock_recovered(&self.core).edit_input.clone()
    }
    #[doc(hidden)]
    pub fn reg_target(&self) -> Option<&'static str> {
        lock_recovered(&self.core).reg_target
    }
    #[doc(hidden)]
    pub fn hover(&self) -> String {
        lock_recovered(&self.core).ppu_hover.clone()
    }
    #[doc(hidden)]
    pub fn set_selected_for_test(&self, selected: Option<(u32, Option<u8>)>) {
        lock_recovered(&self.core).selected = selected;
    }
    #[doc(hidden)]
    pub fn set_edit_input(&self, text: String) {
        self.dispatch(DebuggerEvent::EditInputChanged(text));
    }
    #[doc(hidden)]
    pub fn set_freeze_input(&self, text: String) {
        self.dispatch(DebuggerEvent::FreezeInputChanged(text));
    }
    #[doc(hidden)]
    pub fn set_reg_input(&self, text: String) {
        self.dispatch(DebuggerEvent::RegInputChanged(text));
    }
    #[doc(hidden)]
    pub fn set_reg_target(&self, target: Option<&'static str>) {
        lock_recovered(&self.core).reg_target = target;
    }
    #[doc(hidden)]
    pub fn navigate(&self, anchor: u32, addr: u32) {
        lock_recovered(&self.core).nav.navigate(anchor, addr);
    }
}

/// Pre-localized PPU viewer chrome. Frontends render these verbatim;
/// only the host touches the i18n catalog for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuChrome {
    pub title: String,
    pub hover_empty: String,
    pub no_images: String,
    pub panels_title: String,
}

impl PpuChrome {
    pub fn new(language: AppLanguage) -> Self {
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        Self {
            title: ui_text(language, UiText::PpuViewer).to_string(),
            hover_empty: ui_text(language, UiText::HoverTile).to_string(),
            no_images: ui_text(language, UiText::NoImages).to_string(),
            panels_title: ui_text(language, UiText::Panels).to_string(),
        }
    }
}

/// Pre-localized debugger chrome. Frontends render these verbatim;
/// only the host touches the i18n catalog for them. Dynamic lines
/// (counts, addresses, status) arrive via the methods below so views
/// never format with a language themselves.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DebuggerChrome {
    pub debugger: String,
    pub pause: String,
    pub resume: String,
    pub step_frame: String,
    pub step_instr: String,
    pub ppu_viewer: String,
    pub refresh: String,
    pub space: String,
    pub prev: String,
    pub next: String,
    pub page: String,
    pub addr_hex: String,
    pub go: String,
    pub back: String,
    pub disassembly: String,
    pub follow_pc: String,
    pub follow_pc_tag: String,
    pub fixed_tag: String,
    pub no_disassembly: String,
    pub disassembly_hint: String,
    pub registers: String,
    pub pick_register: String,
    pub hex: String,
    pub set: String,
    pub select_dump_row: String,
    pub write: String,
    pub hex_byte: String,
    pub watch: String,
    pub freeze: String,
    pub confirm: String,
    pub cancel: String,
    pub watch_none: String,
    pub freeze_none: String,
    pub clear: String,
    pub bookmarks: String,
    pub label: String,
    pub add: String,
    pub memory: String,
    language: AppLanguage,
}

impl DebuggerChrome {
    pub fn new(language: AppLanguage) -> Self {
        use nerust_settings_core::i18n::{UiText, text as ui_text};
        let text = |key| ui_text(language, key).to_string();
        Self {
            debugger: text(UiText::Debugger),
            pause: text(UiText::Pause),
            resume: text(UiText::Resume),
            step_frame: text(UiText::StepFrame),
            step_instr: text(UiText::StepInstr),
            ppu_viewer: text(UiText::PpuViewer),
            refresh: text(UiText::Refresh),
            space: text(UiText::Space),
            prev: text(UiText::Prev),
            next: text(UiText::Next),
            page: text(UiText::Page),
            addr_hex: text(UiText::AddrHex),
            go: text(UiText::Go),
            back: text(UiText::Back),
            disassembly: text(UiText::Disassembly),
            follow_pc: text(UiText::FollowPc),
            follow_pc_tag: text(UiText::FollowPcTag),
            fixed_tag: text(UiText::FixedTag),
            no_disassembly: text(UiText::NoDisassembly),
            disassembly_hint: text(UiText::DisassemblyHint),
            registers: text(UiText::Registers),
            pick_register: text(UiText::PickRegister),
            hex: text(UiText::Hex),
            set: text(UiText::Set),
            select_dump_row: text(UiText::SelectDumpRow),
            write: text(UiText::Write),
            hex_byte: text(UiText::HexByte),
            watch: text(UiText::Watch),
            freeze: text(UiText::Freeze),
            confirm: text(UiText::Confirm),
            cancel: text(UiText::Cancel),
            watch_none: text(UiText::WatchNone),
            freeze_none: text(UiText::FreezeNone),
            clear: text(UiText::Clear),
            bookmarks: text(UiText::Bookmarks),
            label: text(UiText::Label),
            add: text(UiText::Add),
            memory: text(UiText::Memory),
            language,
        }
    }

    /// Toolbar pause toggle: resume when paused, pause otherwise.
    pub fn pause_label(&self, paused: bool) -> &str {
        if paused { &self.resume } else { &self.pause }
    }

    /// Back button with history depth, e.g. `Back (3)`.
    pub fn back_label(&self, back_len: usize) -> String {
        format!("{} ({back_len})", self.back)
    }

    /// Disassembly title: follow tag when tracking PC, anchored address otherwise.
    pub fn disassembly_title(&self, follow_pc: bool, anchor: u32) -> String {
        if follow_pc {
            format!("{} {}", self.disassembly, self.follow_pc_tag)
        } else {
            format!("{} @0x{anchor:08X} {}", self.disassembly, self.fixed_tag)
        }
    }

    /// Register target line: `Set <name>` once picked.
    pub fn set_target_label(&self, name: Option<&str>) -> String {
        match name {
            Some(name) => format!("{} {name}", self.set),
            None => self.pick_register.clone(),
        }
    }

    /// Dump selection line, including the old byte when readable.
    pub fn selected_row_text(&self, selected: Option<(u32, Option<u8>)>) -> String {
        use nerust_settings_core::i18n::selected_row;
        match selected {
            Some((addr, old)) => selected_row(self.language, addr, old),
            None => self.select_dump_row.clone(),
        }
    }

    /// Write button carries its target address once a row is selected.
    pub fn write_target_label(&self, selected: Option<(u32, Option<u8>)>) -> String {
        match selected {
            Some((addr, _)) => format!("{} 0x{addr:08X}", self.write),
            None => self.write.clone(),
        }
    }

    /// Staged-write confirm line from the drain-owned text.
    pub fn pending_text(
        &self,
        pending: Option<nerust_gui_runtime::debugger::PendingWrite>,
    ) -> Option<String> {
        pending.map(|pending| pending.render(self.language))
    }

    /// Watch line: live value when watched, placeholder otherwise.
    pub fn watch_text(&self, watch: Option<u32>, value: Option<u8>) -> String {
        use nerust_settings_core::i18n::watch_value;
        match watch {
            Some(addr) => watch_value(self.language, addr, value),
            None => self.watch_none.clone(),
        }
    }

    /// Freeze line: frozen byte when set, placeholder otherwise.
    pub fn freeze_text(&self, freeze: Option<(u32, u8)>) -> String {
        use nerust_settings_core::i18n::frozen;
        match freeze {
            Some((addr, value)) => frozen(self.language, addr, value),
            None => self.freeze_none.clone(),
        }
    }

    /// Status line rendered in the view language.
    pub fn status_text(&self, status: &nerust_gui_runtime::debugger::StatusMessage) -> String {
        status.render(self.language)
    }
}

/// Narrow host drain port: snapshot plus atomic batch take.
pub trait BridgeLock {
    fn drain_snapshot(&self, ppu_open: bool) -> DrainSnapshot;
    fn take_debug_batch(&self) -> (bool, Vec<DebugAction>);
}

impl BridgeLock for DebugBridge {
    fn drain_snapshot(&self, ppu_open: bool) -> DrainSnapshot {
        DebugBridge::drain_snapshot(self, ppu_open)
    }

    fn take_debug_batch(&self) -> (bool, Vec<DebugAction>) {
        // Serialize toolbar flags and action batches with dispatch/rebind.
        let _epoch = lock_recovered(&self.epoch);
        (
            self.open_ppu.swap(false, Ordering::SeqCst),
            self.take_requests(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_viewmodel::debugger::StatusMessage;

    fn test_bridge() -> DebugBridge {
        DebugBridge::new(
            DisplayCache::default(),
            vec![(SpaceId(0), "Memory".into(), 0)],
            0,
            AppLanguage::English,
        )
    }

    #[test]
    fn split_locks_cover_all_state() {
        let bridge = test_bridge();
        assert!(bridge.view_snapshot().bookmarks.is_empty());
        bridge.set_selected_for_test(Some((0x100, None)));
        bridge.dispatch(DebuggerEvent::WatchSet);
        bridge.dispatch(DebuggerEvent::BookmarkInputChanged("mark".into()));
        let view = bridge.view_snapshot();
        assert_eq!(view.watch, Some(0x100));
        assert_eq!(view.bm_input, "mark");
    }

    #[test]
    fn display_snapshot_shares_without_cloning() {
        let bridge = test_bridge();
        assert!(Arc::ptr_eq(
            &bridge.display_snapshot(),
            &bridge.display_snapshot()
        ));
    }

    #[test]
    fn try_lock_audit_is_empty_when_uncontended() {
        assert!(test_bridge().try_lock_audit().is_empty());
    }

    #[test]
    fn lock_recovered_returns_guard_on_clean_mutex() {
        assert_eq!(*lock_recovered(&Mutex::new(42)), 42);
    }

    #[test]
    fn lock_recovered_recovers_value_after_poison() {
        let mutex = Mutex::new(7);
        let _ = std::panic::catch_unwind(|| {
            let _guard = mutex.lock().unwrap();
            panic!("intentional poison");
        });
        assert!(mutex.is_poisoned());
        assert_eq!(*lock_recovered(&mutex), 7);
    }

    #[test]
    fn state_recovers_value_after_poison() {
        let bridge = test_bridge();
        let _ = std::panic::catch_unwind(|| {
            let _guard = bridge.core.lock().unwrap();
            panic!("intentional poison");
        });
        assert!(bridge.core.is_poisoned());
        assert!(bridge.view_snapshot().bookmarks.is_empty());
    }

    #[test]
    fn outbox_push_and_take_round_trip() {
        let bridge = test_bridge();
        assert!(bridge.take_requests().is_empty());
        bridge.push(DebugAction::Refresh);
        bridge.push(DebugAction::TogglePause);
        assert_eq!(bridge.take_requests().len(), 2);
        assert!(bridge.take_requests().is_empty());
    }

    #[test]
    fn set_status_stores_local_message_and_invalidates() {
        let bridge = test_bridge();
        bridge.dispatch(DebuggerEvent::MemInputChanged("zz".into()));
        bridge.dispatch(DebuggerEvent::MemGo);
        assert!(matches!(
            bridge.display_snapshot().status,
            StatusMessage::Local(_)
        ));
        assert!(bridge.view_invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn replacement_discards_unclaimed_ui_actions_and_freeze() {
        let bridge = test_bridge();
        let epoch = Arc::new(AtomicBool::new(true));
        bridge.bind_epoch(epoch.clone());
        bridge.set_selected_for_test(Some((0x10, None)));
        bridge.set_freeze_input("ff".into());
        bridge.dispatch(DebuggerEvent::FreezeSet);
        bridge.take_requests();
        bridge.push(DebugAction::RegisterWrite(7));
        epoch.store(false, Ordering::Release);
        bridge.push(DebugAction::Resume);
        assert_eq!(bridge.outbox_len(), 1);
        bridge.bind_epoch(Arc::new(AtomicBool::new(true)));
        assert!(bridge.take_requests().is_empty());
        assert!(bridge.freeze().is_none());
        bridge.push(DebugAction::Refresh);
        assert_eq!(bridge.take_requests().len(), 1);
    }

    #[test]
    fn drain_snapshot_space_and_name_follow_nav_index() {
        let bridge = test_bridge();
        assert_eq!(bridge.drain_snapshot(false).space, Some(SpaceId(0)));
        assert_eq!(bridge.selected_name().as_deref(), Some("Memory"));
    }

    fn dis_bridge() -> DebugBridge {
        use nerust_core_traits::debugger::DisasmLine;
        DebugBridge::new(
            DisplayCache {
                disasm_lines: vec![DisasmLine {
                    addr: 0x1234,
                    bytes: [0; 4],
                    len: 2,
                    text: "NOP".into(),
                    is_pc: true,
                    target: None,
                }],
                ..DisplayCache::default()
            },
            vec![],
            0,
            AppLanguage::English,
        )
    }

    #[test]
    fn anchor_uses_pin_without_follow() {
        let bridge = dis_bridge();
        bridge.dispatch(DebuggerEvent::DisLineClicked(0x300));
        assert_eq!(bridge.anchor(), 0x300);
    }

    #[test]
    fn anchor_uses_first_line_in_follow_mode() {
        assert_eq!(dis_bridge().anchor(), 0x1234);
    }

    #[test]
    fn live_images_apply_only_when_newer() {
        let bridge = test_bridge();
        assert!(!bridge.apply_live_images(0, vec![], true));
        assert!(bridge.apply_live_images(
            3,
            vec![DebugImageRgba {
                label: "Tiles".into(),
                width: 2,
                height: 1,
                rgba: vec![0xff; 8]
            }],
            true
        ));
        assert_eq!(bridge.display_snapshot().images_gen, 3);
        assert_eq!(bridge.display_snapshot().status, StatusMessage::Running);
        bridge.update_display(|display| display.status = StatusMessage::Local("held".into()));
        assert!(bridge.apply_live_images(4, vec![], false));
        assert!(bridge.display_snapshot().images.is_empty());
        assert!(matches!(
            bridge.display_snapshot().status,
            StatusMessage::Local(_)
        ));
    }

    #[test]
    fn drain_display_applies_selection_and_restores() {
        let bridge = test_bridge();
        bridge.apply_drain_display(DisplayCache::default(), Some((0x120, Some(0xab))));
        assert_eq!(bridge.selected(), Some((0x120, Some(0xab))));
        bridge.apply_drain_display(DisplayCache::default(), None);
        assert_eq!(bridge.selected(), Some((0x120, Some(0xab))));
    }

    #[test]
    fn drain_snapshot_carries_flags_and_none_space() {
        let bridge = test_bridge();
        assert!(bridge.drain_snapshot(true).ppu_open);
        assert!(bridge.drain_snapshot(false).prev_rows.is_empty());
        let empty = DebugBridge::new(DisplayCache::default(), vec![], 0, AppLanguage::English);
        assert_eq!(empty.drain_snapshot(false).space, None);
        assert_eq!(empty.selected_name(), None);
        assert_eq!(empty.anchor(), 0);
    }

    #[test]
    fn audit_names_contended_locks() {
        let bridge = test_bridge();
        let _guard = lock_recovered(&bridge.core);
        assert!(bridge.try_lock_audit().contains(&"core"));
    }

    #[test]
    fn audit_names_contended_display() {
        let bridge = test_bridge();
        let _guard = write_display(&bridge.display);
        assert_eq!(bridge.try_lock_audit(), vec!["display"]);
    }

    #[test]
    fn audit_names_contended_outbox() {
        let bridge = test_bridge();
        let _guard = lock_recovered(&bridge.outbox);
        assert_eq!(bridge.try_lock_audit(), vec!["outbox"]);
    }

    #[test]
    fn bridge_lock_trait_swaps_ppu_flag_and_batch() {
        let bridge = test_bridge();
        bridge.dispatch(DebuggerEvent::PpuOpenPressed);
        let port: &dyn BridgeLock = &bridge;
        let (open, batch) = port.take_debug_batch();
        assert!(open);
        assert_eq!(batch.len(), 1);
        assert!(!port.take_debug_batch().0);
        assert_eq!(port.drain_snapshot(false).mem_addr, 0);
    }

    #[test]
    fn language_and_spaces_read_from_meta() {
        let bridge = test_bridge();
        assert_eq!(bridge.language(), AppLanguage::English);
        assert_eq!(bridge.spaces()[0].0, SpaceId(0));
    }

    #[test]
    fn single_write_ports_cover_inputs_and_nav() {
        let bridge = test_bridge();
        bridge.dispatch(DebuggerEvent::MemInputChanged("100".into()));
        bridge.dispatch(DebuggerEvent::MemGo);
        assert_eq!(bridge.mem_addr(), 0x100);
        bridge.dispatch(DebuggerEvent::DisLineClicked(0x120));
        bridge.dispatch(DebuggerEvent::DisBack);
        assert_eq!(bridge.anchor(), 0);
        bridge.dispatch(DebuggerEvent::SpaceNext);
        assert_eq!(bridge.mem_addr(), 0);
        bridge.set_hover("tile".into());
        assert_eq!(bridge.hover(), "tile");
    }

    #[test]
    fn display_and_outbox_poison_recovery_survive_dispatch() {
        let bridge = test_bridge();
        let _ = std::panic::catch_unwind(|| {
            let _guard = bridge.display.write().unwrap();
            panic!("display poison");
        });
        let _ = std::panic::catch_unwind(|| {
            let _guard = bridge.outbox.lock().unwrap();
            panic!("outbox poison");
        });
        bridge.dispatch(DebuggerEvent::RefreshPressed);
        assert_eq!(bridge.take_requests().len(), 1);
        assert!(bridge.display_snapshot().images.is_empty());
    }

    fn wait_for_epoch_guard(bridge: &DebugBridge) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if bridge.epoch.try_lock().is_err() {
                return true;
            }
            std::thread::yield_now();
        }
        false
    }

    #[test]
    fn dispatch_locks_epoch_before_state_and_rechecks_retirement_after_waiting() {
        let bridge = test_bridge();
        let epoch = Arc::new(AtomicBool::new(true));
        bridge.bind_epoch(epoch.clone());
        std::thread::scope(|scope| {
            let core = lock_recovered(&bridge.core);
            let worker =
                scope.spawn(|| bridge.dispatch(DebuggerEvent::MemInputChanged("stale".into())));
            let acquired_epoch_first = wait_for_epoch_guard(&bridge);
            epoch.store(false, Ordering::Release);
            drop(core);
            let update = worker.join().unwrap();
            assert!(
                acquired_epoch_first,
                "dispatch must acquire epoch before state"
            );
            assert!(update.actions.is_empty());
            assert_eq!(update.scope, None);
        });
        assert_eq!(bridge.view_snapshot().nav.mem_input(), "0x00000000");
        assert!(!bridge.view_invalidated.load(Ordering::Acquire));
    }

    #[test]
    fn epoch_rebind_cannot_overtake_or_leave_half_of_an_ordered_batch() {
        let bridge = test_bridge();
        bridge.bind_epoch(Arc::new(AtomicBool::new(true)));
        bridge.set_selected_for_test(Some((0x100, None)));
        bridge.dispatch(DebuggerEvent::EditInputChanged("42".into()));
        std::thread::scope(|scope| {
            let outbox = lock_recovered(&bridge.outbox);
            let dispatch = scope.spawn(|| bridge.dispatch(DebuggerEvent::EditWrite));
            let acquired_epoch = wait_for_epoch_guard(&bridge);
            let rebind = scope.spawn(|| bridge.bind_epoch(Arc::new(AtomicBool::new(true))));
            drop(outbox);
            let update = dispatch.join().unwrap();
            rebind.join().unwrap();
            assert!(acquired_epoch);
            assert!(matches!(
                update.actions.as_slice(),
                [DebugAction::SelectRow(0x100), DebugAction::EditStage(0x42)]
            ));
        });
        assert!(
            bridge.take_requests().is_empty(),
            "rebind discards the complete old batch"
        );
        assert!(bridge.selected().is_none());
    }
}
