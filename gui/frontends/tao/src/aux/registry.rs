//! Aux-window route table: one entry per secondary window.
//! Dispatch resolves native ids once; every entry pumps domain events.

use std::collections::HashMap;
use std::sync::Arc;

use nerust_core_traits::debugger::SpaceId;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::{debug::DebugAction, session::osd::HistoryLine};
use nerust_gui_viewmodel::debugger::DisplayCache;
use nerust_settings_core::model::language::AppLanguage;

use super::bridge::{DebugBridge, DrainSnapshot};
use super::domain::{AuxId, AuxNativeEvent, NativeWindowId};
use super::focus::{AuxEvents, AuxIdentity, AuxKind, AuxRender, pump_aux};

/// Effects one aux event produced: computed in the entry, applied by the host.
#[derive(Debug, Clone, Default)]
pub(crate) struct AuxOutcome {
    /// Close flag raised: drop the window.
    pub close: bool,
    /// History clear requested (history only).
    pub cleared: bool,
    pub error: Option<String>,
}

/// Debug-drain port: batch take plus apply for one drain.
pub(crate) trait DebugDrainPort {
    /// Debugger batch take (debug only).
    fn take_debug_requests(&mut self) -> Option<(bool, Vec<DebugAction>)>;

    /// Apply a finished drain (debug only).
    fn apply_drain(&mut self, display: DisplayCache, reselected: Option<(u32, Option<u8>)>);
}

/// Live-images port: finished live fetches land here.
/// Only the debugger implements this; see `AuxController::as_live`.
pub(crate) trait LiveImagesPort {
    /// Apply finished live images (debug only).
    fn apply_live(
        &mut self,
        _generation: u64,
        _images: Vec<(String, u32, u32, Vec<u8>)>,
        _running: bool,
    ) -> bool;
}

/// Settings-close port: pending apply staged for close teardown.
/// Only settings implements this; see `AuxController::as_settings`.
pub(crate) trait SettingsClosePort {
    /// Take the pending settings apply on close (settings only).
    fn take_pending_settings(&mut self) -> Option<SettingsSnapshot>;
}

/// History-sync port: content push for the history viewer.
pub(crate) trait HistorySyncPort {
    /// Push fresh history content (history only).
    fn sync_history(&mut self, lines: Arc<[HistoryLine]>, language: AppLanguage);
}

/// Uniform aux controller: identity plus render plus capability queries.
pub(crate) trait AuxController: AuxIdentity + AuxRender + AuxEvents {
    /// Pump one domain event: prelude, dispatch, outcome.
    fn on_event(&mut self, event: AuxNativeEvent) -> AuxOutcome {
        pump_aux(self, event);
        AuxOutcome {
            close: self.close_requested(),
            cleared: self.take_history_clear(),
            error: None,
        }
    }

    /// Full render through the shared pump.
    fn on_redraw(&mut self) -> AuxOutcome {
        let error = super::focus::AuxRender::render(self);
        AuxOutcome {
            close: self.close_requested(),
            cleared: self.take_history_clear(),
            error,
        }
    }

    /// Rebuild the view from the bridge (debug/PPU only).
    fn sync_view(&mut self) {}

    /// Debug-drain capability (debugger only).
    fn as_debug(&mut self) -> Option<&mut dyn DebugDrainPort> {
        None
    }

    /// Read-only drain snapshot (`None` unless debugger).
    fn debug_snapshot_of(&self, _ppu_open: bool) -> Option<DrainSnapshot> {
        None
    }

    /// Live-images capability (debugger only).
    fn as_live(&mut self) -> Option<&mut dyn LiveImagesPort> {
        None
    }

    /// Settings-close capability (settings only).
    fn as_settings(&mut self) -> Option<&mut dyn SettingsClosePort> {
        None
    }

    /// History-sync capability (history only).
    fn as_history(&mut self) -> Option<&mut dyn HistorySyncPort> {
        None
    }

    /// Take a history clear request (history only; routed here so the
    /// default pump needs no port bound).
    fn take_history_clear(&mut self) -> bool {
        false
    }

    /// Share the debugger bridge for viewer creation (debug only).
    fn shared_bridge(&self) -> Option<Arc<DebugBridge>> {
        None
    }

    /// Contended lock names, if any (debug only).
    fn debug_lock_audit(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// One routed entry: registry id plus native id plus the controller.
struct AuxEntry {
    id: AuxId,
    native: NativeWindowId,
    controller: Box<dyn AuxController>,
}

/// Route table for every secondary window: kind, id, and native maps.
#[derive(Default)]
pub(crate) struct AuxRegistry {
    entries: HashMap<AuxKind, AuxEntry>,
    by_native: HashMap<NativeWindowId, AuxKind>,
    next_id: u64,
}

impl AuxRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Seat a newly created aux window, assigning its route id.
    pub(crate) fn insert(&mut self, entry: Box<dyn AuxController>) -> AuxId {
        let id = AuxId::new(self.next_id);
        self.next_id += 1;
        let kind = entry.kind();
        let native = entry.native_id();
        self.by_native.insert(native, kind);
        self.entries.insert(
            kind,
            AuxEntry {
                id,
                native,
                controller: entry,
            },
        );
        id
    }

    /// Route id of the open window of this kind, if any.
    pub(crate) fn route_id(&self, kind: AuxKind) -> Option<AuxId> {
        self.entries.get(&kind).map(|entry| entry.id)
    }

    /// Kind of the window behind `id`, if registered.
    pub(crate) fn kind_of(&self, id: NativeWindowId) -> Option<AuxKind> {
        self.by_native.get(&id).copied()
    }

    /// Whether a window of this kind is currently open.
    pub(crate) fn contains(&self, kind: AuxKind) -> bool {
        self.entries.contains_key(&kind)
    }

    /// Pump one domain event through the entry of this kind.
    pub(crate) fn on_event(&mut self, kind: AuxKind, event: AuxNativeEvent) -> Option<AuxOutcome> {
        self.entries
            .get_mut(&kind)
            .map(|entry| entry.controller.on_event(event))
    }

    /// Render the open window of this kind, if any.
    pub(crate) fn render_kind(&mut self, kind: AuxKind) -> Option<AuxOutcome> {
        self.entries
            .get_mut(&kind)
            .map(|entry| entry.controller.on_redraw())
    }

    pub(crate) fn pump_deadlines(&mut self, now: std::time::Instant) -> Option<std::time::Instant> {
        self.entries
            .values_mut()
            .filter_map(|entry| {
                entry.controller.wake_due(now);
                entry.controller.wake_deadline()
            })
            .min()
    }

    /// Drop the window of this kind, returning it for teardown.
    pub(crate) fn remove_kind(&mut self, kind: AuxKind) -> Option<Box<dyn AuxController>> {
        let entry = self.entries.remove(&kind)?;
        self.by_native.remove(&entry.native);
        Some(entry.controller)
    }

    /// Drop every aux window (close teardown).
    pub(crate) fn close_all(&mut self) {
        self.entries.clear();
        self.by_native.clear();
    }

    fn entry(&self, kind: AuxKind) -> Option<&(dyn AuxController + '_)> {
        self.entries.get(&kind).map(|entry| &*entry.controller)
    }

    fn entry_mut(&mut self, kind: AuxKind) -> Option<&mut (dyn AuxController + '_)> {
        match self.entries.get_mut(&kind) {
            Some(entry) => Some(&mut *entry.controller),
            None => None,
        }
    }

    /// Debugger batch take (`None` when the debugger is closed).
    pub(crate) fn debug_take_requests(&mut self) -> Option<(bool, Vec<DebugAction>)> {
        self.entry_mut(AuxKind::Debug)
            .and_then(AuxController::as_debug)
            .and_then(DebugDrainPort::take_debug_requests)
    }

    /// Drain snapshot (`None` when the debugger is closed).
    pub(crate) fn debug_snapshot(&self, ppu_open: bool) -> Option<DrainSnapshot> {
        // Read-only query behind a shared borrow: the snapshot clones small fields only.
        self.entries
            .get(&AuxKind::Debug)
            .and_then(|entry| entry.controller.debug_snapshot_of(ppu_open))
    }

    /// Apply a finished drain, then rebuild the viewers.
    pub(crate) fn debug_apply(
        &mut self,
        display: DisplayCache,
        reselected: Option<(u32, Option<u8>)>,
    ) {
        if let Some(port) = self
            .entry_mut(AuxKind::Debug)
            .and_then(AuxController::as_debug)
        {
            port.apply_drain(display, reselected);
        }
        self.debug_sync();
        self.ppu_sync();
    }

    /// Apply finished live images; true when the viewer needs a rebuild.
    pub(crate) fn debug_apply_live(
        &mut self,
        generation: u64,
        images: Vec<(String, u32, u32, Vec<u8>)>,
        running: bool,
    ) -> bool {
        let rebuilt = self
            .entry_mut(AuxKind::Debug)
            .and_then(AuxController::as_live)
            .is_some_and(|entry| entry.apply_live(generation, images, running));
        if rebuilt {
            self.debug_sync();
        }
        rebuilt
    }

    /// Rebuild the debugger view from the bridge.
    pub(crate) fn debug_sync(&mut self) {
        if let Some(entry) = self.entry_mut(AuxKind::Debug) {
            entry.sync_view();
        }
    }

    /// Share the debugger bridge for viewer creation.
    pub(crate) fn debug_bridge(&self) -> Option<Arc<DebugBridge>> {
        self.entry(AuxKind::Debug)
            .and_then(AuxController::shared_bridge)
    }

    /// Rebuild the PPU viewer from the bridge.
    pub(crate) fn ppu_sync(&mut self) {
        if let Some(entry) = self.entry_mut(AuxKind::Ppu) {
            entry.sync_view();
        }
    }

    /// Push fresh history content.
    pub(crate) fn history_sync(&mut self, lines: Arc<[HistoryLine]>, language: AppLanguage) {
        if let Some(port) = self
            .entry_mut(AuxKind::History)
            .and_then(AuxController::as_history)
        {
            port.sync_history(lines, language);
        }
    }

    /// Take the pending settings apply on close.
    pub(crate) fn settings_take_pending(&mut self) -> Option<SettingsSnapshot> {
        self.entry_mut(AuxKind::Settings)
            .and_then(AuxController::as_settings)
            .and_then(SettingsClosePort::take_pending_settings)
    }

    /// Contended bridge locks, if any (debug only).
    pub(crate) fn debug_locks_free(&self) -> bool {
        self.entry(AuxKind::Debug)
            .is_some_and(|entry| entry.debug_lock_audit().is_empty())
    }

    /// Bring the window of this kind to the front, if open.
    pub(crate) fn focus_kind(&self, kind: AuxKind) {
        if let Some(entry) = self.entry(kind) {
            entry.focus_window();
        }
    }
}

/// Debug spaces for viewer creation: ids plus ranges.
pub(crate) fn debug_spaces(
    session: &nerust_gui_shell::session::SessionHandle,
) -> Vec<(SpaceId, String, u32)> {
    session
        .debug_spaces()
        .into_iter()
        .map(|info| (info.id, info.name.to_string(), *info.range.start()))
        .collect()
}

#[cfg(test)]
pub(crate) mod test_double {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Arc;

    use nerust_gui_shell::debug::DebugAction;

    use crate::aux::bridge::{DebugBridge, DrainSnapshot};
    use crate::aux::domain::{AuxNativeEvent, NativeWindowId};
    use crate::aux::focus::{AuxEvents, AuxIdentity, AuxKind, AuxRender};

    use super::{
        AuxController, DebugDrainPort, HistorySyncPort, LiveImagesPort, SettingsClosePort,
    };
    /// Configurable stub aux: answers only the capability its kind owns.
    pub(crate) struct StubAux {
        pub kind: AuxKind,
        pub close: bool,
        pub clear_history: bool,
        pub live_rebuild: bool,
        pub applied_drain: Rc<Cell<bool>>,
        pub bridge: Option<Arc<DebugBridge>>,
        pub drain_open: bool,
        pub debug_batch: Vec<DebugAction>,
        pub pending_settings: Option<nerust_gui_runtime::settings::SettingsSnapshot>,
        pub reg_target: Option<&'static str>,
    }

    impl StubAux {
        pub(crate) fn new(kind: AuxKind) -> Self {
            Self {
                kind,
                close: false,
                clear_history: false,
                live_rebuild: false,
                applied_drain: Rc::new(Cell::new(false)),
                bridge: None,
                drain_open: false,
                debug_batch: Vec::new(),
                pending_settings: None,
                reg_target: None,
            }
        }
    }

    impl AuxIdentity for StubAux {
        fn kind(&self) -> AuxKind {
            self.kind
        }

        fn native_id(&self) -> NativeWindowId {
            NativeWindowId::dummy()
        }

        fn close_requested(&self) -> bool {
            self.close
        }

        fn focus_window(&self) {}
        fn request_redraw(&self) {}
    }

    impl AuxRender for StubAux {
        fn handle_common(&mut self, _event: &AuxNativeEvent) {}
        fn feed_event(&mut self, _event: AuxNativeEvent) -> Option<iced::Event> {
            None
        }
        fn render(&mut self) -> Option<String> {
            None
        }
    }

    impl AuxEvents for StubAux {
        fn handle(&mut self, _mapped: iced::Event) -> bool {
            false
        }
    }

    impl DebugDrainPort for StubAux {
        fn take_debug_requests(&mut self) -> Option<(bool, Vec<DebugAction>)> {
            (self.kind == AuxKind::Debug)
                .then(|| (self.drain_open, std::mem::take(&mut self.debug_batch)))
        }

        fn apply_drain(
            &mut self,
            _display: nerust_gui_viewmodel::debugger::DisplayCache,
            _reselected: Option<(u32, Option<u8>)>,
        ) {
            self.applied_drain.set(true);
        }
    }

    impl LiveImagesPort for StubAux {
        fn apply_live(
            &mut self,
            _generation: u64,
            _images: Vec<(String, u32, u32, Vec<u8>)>,
            _running: bool,
        ) -> bool {
            self.live_rebuild
        }
    }

    impl SettingsClosePort for StubAux {
        fn take_pending_settings(
            &mut self,
        ) -> Option<nerust_gui_runtime::settings::SettingsSnapshot> {
            (self.kind == AuxKind::Settings).then(|| self.pending_settings.take())?
        }
    }

    impl HistorySyncPort for StubAux {
        fn sync_history(
            &mut self,
            _lines: Arc<[nerust_gui_shell::session::osd::HistoryLine]>,
            _language: nerust_settings_core::model::language::AppLanguage,
        ) {
        }
    }

    impl AuxController for StubAux {
        fn as_debug(&mut self) -> Option<&mut dyn DebugDrainPort> {
            (self.kind == AuxKind::Debug).then_some(self as &mut dyn DebugDrainPort)
        }

        fn debug_snapshot_of(&self, ppu_open: bool) -> Option<DrainSnapshot> {
            (self.kind == AuxKind::Debug).then(|| DrainSnapshot {
                space: None,
                mem_addr: 0,
                dis_addr: None,
                watch: None,
                freeze: None,
                reg_target: self.reg_target,
                prev_rows: Arc::from([]),
                ppu_open,
            })
        }

        fn as_live(&mut self) -> Option<&mut dyn LiveImagesPort> {
            (self.kind == AuxKind::Debug).then_some(self as &mut dyn LiveImagesPort)
        }

        fn as_settings(&mut self) -> Option<&mut dyn SettingsClosePort> {
            (self.kind == AuxKind::Settings).then_some(self as &mut dyn SettingsClosePort)
        }

        fn as_history(&mut self) -> Option<&mut dyn HistorySyncPort> {
            (self.kind == AuxKind::History).then_some(self as &mut dyn HistorySyncPort)
        }

        fn take_history_clear(&mut self) -> bool {
            self.clear_history
        }

        fn shared_bridge(&self) -> Option<Arc<DebugBridge>> {
            (self.kind == AuxKind::Debug).then(|| self.bridge.clone())?
        }

        fn debug_lock_audit(&self) -> Vec<&'static str> {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_double::StubAux;
    use super::*;

    #[test]
    fn registry_routes_by_map_without_scans() {
        let mut registry = AuxRegistry::new();
        assert!(!registry.contains(AuxKind::Debug));
        let id = registry.insert(Box::new(StubAux::new(AuxKind::Debug)));
        assert_eq!(registry.route_id(AuxKind::Debug), Some(id));
        assert!(registry.contains(AuxKind::Debug));
        // Event and render route by kind; close flag stays clear.
        let outcome = registry
            .on_event(AuxKind::Debug, AuxNativeEvent::Focused(true))
            .expect("debug routes");
        assert!(!outcome.close);
        registry.render_kind(AuxKind::Debug);
        registry.focus_kind(AuxKind::Debug);
        // Removal drops the route; re-insert assigns a fresh id.
        assert!(registry.remove_kind(AuxKind::Debug).is_some());
        assert!(!registry.contains(AuxKind::Debug));
        let second = registry.insert(Box::new(StubAux::new(AuxKind::Debug)));
        assert_ne!(id, second);
    }

    #[test]
    fn registry_tracks_native_ids_and_clears_all() {
        let mut registry = AuxRegistry::new();
        assert_eq!(registry.kind_of(NativeWindowId::dummy()), None);
        registry.insert(Box::new(StubAux::new(AuxKind::History)));
        // Dummy ids route back to the seated kind.
        assert_eq!(
            registry.kind_of(NativeWindowId::dummy()),
            Some(AuxKind::History)
        );
        assert!(registry.remove_kind(AuxKind::Debug).is_none());
        registry.render_kind(AuxKind::Debug);
        registry.focus_kind(AuxKind::Debug);
        assert!(
            registry
                .on_event(AuxKind::Debug, AuxNativeEvent::Focused(true))
                .is_none()
        );
        // Close-all drops every route at once.
        registry.close_all();
        assert!(!registry.contains(AuxKind::History));
        assert_eq!(registry.route_id(AuxKind::History), None);
    }

    #[test]
    fn on_event_reports_close_and_clear_flags() {
        let mut registry = AuxRegistry::new();
        let mut stub = StubAux::new(AuxKind::History);
        stub.clear_history = true;
        stub.close = true;
        registry.insert(Box::new(stub));
        let outcome = registry
            .on_event(AuxKind::History, AuxNativeEvent::Focused(true))
            .expect("history routes");
        assert!(outcome.close);
        assert!(outcome.cleared);
        // Default controller hooks stay quiet.
        registry.render_kind(AuxKind::History);
        registry.focus_kind(AuxKind::History);
        let mut bare = StubAux::new(AuxKind::History);
        bare.sync_view();
        AuxController::on_redraw(&mut bare);
    }

    #[test]
    fn debug_ports_delegate_to_the_open_entry() {
        use nerust_settings_core::model::language::AppLanguage;
        let mut registry = AuxRegistry::new();
        // Closed debugger: every port stays quiet.
        assert!(registry.debug_take_requests().is_none());
        assert!(registry.debug_snapshot(false).is_none());
        assert!(registry.debug_bridge().is_none());
        assert!(!registry.debug_locks_free());
        assert!(!registry.debug_apply_live(1, Vec::new(), false));
        registry.debug_apply(DisplayCache::default(), None);
        registry.debug_sync();
        registry.ppu_sync();
        assert!(registry.settings_take_pending().is_none());
        registry.history_sync(Arc::from([]), AppLanguage::English);
        // Open debugger: batch, snapshot, and bridge delegate.
        let mut stub = StubAux::new(AuxKind::Debug);
        stub.drain_open = true;
        stub.debug_batch = vec![DebugAction::Refresh];
        stub.bridge = Some(Arc::new(DebugBridge::new(
            DisplayCache::default(),
            vec![(SpaceId(0), "Memory".to_string(), 0)],
            0,
            AppLanguage::English,
        )));
        let applied = std::rc::Rc::clone(&stub.applied_drain);
        registry.insert(Box::new(stub));
        let (open, batch) = registry.debug_take_requests().expect("debug routes");
        assert!(open);
        assert_eq!(batch.len(), 1);
        assert!(
            registry
                .debug_snapshot(true)
                .expect("debug routes")
                .ppu_open
        );
        assert!(registry.debug_bridge().is_some());
        assert!(registry.debug_locks_free());
        // Apply lands on the entry, then fans out to the PPU sync.
        registry.debug_apply(DisplayCache::default(), Some((0x10, None)));
        assert!(applied.get());
        // Live apply rebuilds only when the entry says so.
        assert!(!registry.debug_apply_live(2, Vec::new(), false));
    }

    #[test]
    fn debug_apply_live_rebuilds_through_sync() {
        let mut registry = AuxRegistry::new();
        let mut stub = StubAux::new(AuxKind::Debug);
        stub.live_rebuild = true;
        registry.insert(Box::new(stub));
        assert!(registry.debug_apply_live(7, Vec::new(), true));
    }

    #[test]
    fn wrong_kind_never_answers_debug_batch() {
        // Kind gating (not just routing) keeps a seated settings
        // window from answering debugger-only ports.
        let mut registry = AuxRegistry::new();
        registry.insert(Box::new(StubAux::new(AuxKind::Settings)));
        assert!(registry.debug_take_requests().is_none());
        assert!(registry.debug_snapshot(false).is_none());
        assert!(registry.settings_take_pending().is_none());
    }
}
