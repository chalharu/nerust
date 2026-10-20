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
use super::focus::{AuxEvents, AuxIdentity, AuxKind, AuxRender, pump_aux};
use super::native::{AuxId, AuxNativeEvent, NativeWindowId};

/// Effects one aux event produced: computed in the entry, applied by the host.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AuxOutcome {
    /// Close flag raised: drop the window.
    pub close: bool,
    /// History clear requested (history only).
    pub cleared: bool,
}

/// Debug-drain port: batch take plus snapshot/apply for one drain.
pub(crate) trait DebugDrainPort {
    /// Debugger batch take (debug only).
    fn take_debug_requests(&mut self) -> Option<(bool, Vec<DebugAction>)> {
        None
    }

    /// Drain snapshot for the debug drain (debug only).
    fn drain_snapshot(&self, _ppu_open: bool) -> Option<DrainSnapshot> {
        None
    }

    /// Apply a finished drain (debug only).
    fn apply_drain(&mut self, _display: DisplayCache, _reselected: Option<(u32, Option<u8>)>) {}
}

/// Live-images port: finished live fetches land here.
pub(crate) trait LiveImagesPort {
    /// Apply finished live images (debug only).
    fn apply_live(
        &mut self,
        _generation: u64,
        _images: Vec<(String, u32, u32, Vec<u8>)>,
        _running: bool,
    ) -> bool {
        false
    }
}

/// Settings-close port: pending apply staged for close teardown.
pub(crate) trait SettingsClosePort {
    /// Take the pending settings apply on close (settings only).
    fn take_pending_settings(&mut self) -> Option<SettingsSnapshot> {
        None
    }
}

/// History-sync port: content push plus clear requests.
pub(crate) trait HistorySyncPort {
    /// Take a history clear request (history only).
    fn take_history_clear(&mut self) -> bool {
        false
    }

    /// Push fresh history content (history only).
    fn sync_history(&mut self, _lines: Arc<[HistoryLine]>, _language: AppLanguage) {}
}

/// Bridge-share port: viewer creation plus lock audit.
pub(crate) trait BridgeSharePort {
    /// Share the debugger bridge for viewer creation (debug only).
    fn shared_bridge(&self) -> Option<Arc<DebugBridge>> {
        None
    }

    /// Contended lock names, if any (debug only).
    fn debug_lock_audit(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// Uniform aux controller: identity plus render plus one port per capability.
pub(crate) trait AuxController:
    AuxIdentity
    + AuxRender
    + AuxEvents
    + DebugDrainPort
    + LiveImagesPort
    + SettingsClosePort
    + HistorySyncPort
    + BridgeSharePort
{
    /// Pump one domain event: prelude, dispatch, outcome.
    fn on_event(&mut self, event: AuxNativeEvent) -> AuxOutcome {
        pump_aux(self, event);
        AuxOutcome {
            close: self.close_requested(),
            cleared: self.take_history_clear(),
        }
    }

    /// Full render through the shared pump.
    fn on_redraw(&mut self) {
        super::focus::AuxRender::render(self);
    }

    /// Rebuild the view from the bridge (debug/PPU only).
    fn sync_view(&mut self) {}
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
    pub(crate) fn render_kind(&mut self, kind: AuxKind) {
        if let Some(entry) = self.entries.get_mut(&kind) {
            entry.controller.on_redraw();
        }
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
            .and_then(DebugDrainPort::take_debug_requests)
    }

    /// Drain snapshot (`None` when the debugger is closed).
    pub(crate) fn debug_snapshot(&self, ppu_open: bool) -> Option<DrainSnapshot> {
        self.entry(AuxKind::Debug)
            .and_then(|entry| entry.drain_snapshot(ppu_open))
    }

    /// Apply a finished drain, then rebuild the viewers.
    pub(crate) fn debug_apply(
        &mut self,
        display: DisplayCache,
        reselected: Option<(u32, Option<u8>)>,
    ) {
        if let Some(entry) = self.entry_mut(AuxKind::Debug) {
            entry.apply_drain(display, reselected);
            entry.sync_view();
        }
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
            .and_then(BridgeSharePort::shared_bridge)
    }

    /// Rebuild the PPU viewer from the bridge.
    pub(crate) fn ppu_sync(&mut self) {
        if let Some(entry) = self.entry_mut(AuxKind::Ppu) {
            entry.sync_view();
        }
    }

    /// Push fresh history content.
    pub(crate) fn history_sync(&mut self, lines: Arc<[HistoryLine]>, language: AppLanguage) {
        if let Some(entry) = self.entry_mut(AuxKind::History) {
            entry.sync_history(lines, language);
        }
    }

    /// Take the pending settings apply on close.
    pub(crate) fn settings_take_pending(&mut self) -> Option<SettingsSnapshot> {
        self.entry_mut(AuxKind::Settings)
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
mod tests {
    use super::*;

    struct StubAux {
        kind: AuxKind,
    }

    impl AuxIdentity for StubAux {
        fn kind(&self) -> AuxKind {
            self.kind
        }

        fn native_id(&self) -> NativeWindowId {
            NativeWindowId::dummy()
        }

        fn close_requested(&self) -> bool {
            false
        }

        fn focus_window(&self) {}
        fn request_redraw(&self) {}
    }

    impl AuxRender for StubAux {
        fn handle_common(&mut self, _event: &AuxNativeEvent) {}
        fn feed_event(&mut self, _event: AuxNativeEvent) -> Option<iced::Event> {
            None
        }
        fn render(&mut self) {}
    }

    impl AuxEvents for StubAux {
        fn handle(&mut self, _mapped: iced::Event) -> bool {
            false
        }
    }

    impl DebugDrainPort for StubAux {}
    impl LiveImagesPort for StubAux {}
    impl SettingsClosePort for StubAux {}
    impl HistorySyncPort for StubAux {}
    impl BridgeSharePort for StubAux {}
    impl AuxController for StubAux {}

    #[test]
    fn registry_routes_by_map_without_scans() {
        let mut registry = AuxRegistry::new();
        assert!(!registry.contains(AuxKind::Debug));
        let id = registry.insert(Box::new(StubAux {
            kind: AuxKind::Debug,
        }));
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
        let second = registry.insert(Box::new(StubAux {
            kind: AuxKind::Debug,
        }));
        assert_ne!(id, second);
    }

    #[test]
    fn ports_default_to_closed_quiet() {
        let mut stub = StubAux {
            kind: AuxKind::History,
        };
        assert!(DebugDrainPort::take_debug_requests(&mut stub).is_none());
        assert!(DebugDrainPort::drain_snapshot(&stub, false).is_none());
        assert!(!LiveImagesPort::apply_live(&mut stub, 1, Vec::new(), false));
        assert!(SettingsClosePort::take_pending_settings(&mut stub).is_none());
        assert!(!HistorySyncPort::take_history_clear(&mut stub));
        assert!(BridgeSharePort::shared_bridge(&stub).is_none());
        assert!(BridgeSharePort::debug_lock_audit(&stub).is_empty());
    }
}
