//! Aux-window registry: one route table for every secondary window.
//! Dispatch looks entries up by id; each entry pumps its own events.

use std::sync::Arc;

use tao::{event::WindowEvent, window::WindowId};

use nerust_core_traits::debugger::SpaceId;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_gui_shell::{debug::DebugAction, session::osd::HistoryLine};
use nerust_gui_viewmodel::debugger::DisplayCache;
use nerust_settings_core::model::language::AppLanguage;

use super::bridge::{DebugBridge, DrainSnapshot};
use super::focus::{AuxEvents, AuxIdentity, AuxKind, AuxRender, pump_aux};

/// Effects one aux event produced: computed in the entry, applied by the host.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AuxOutcome {
    /// Close flag raised: drop the window.
    pub close: bool,
    /// History clear requested (history only).
    pub cleared: bool,
}

/// Uniform aux controller behind the registry.
pub(crate) trait AuxController: AuxIdentity + AuxRender + AuxEvents {
    /// Pump one native event: prelude, dispatch, outcome.
    fn on_event(&mut self, event: WindowEvent) -> AuxOutcome {
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

    /// Apply finished live images (debug only).
    fn apply_live(
        &mut self,
        _generation: u64,
        _images: Vec<(String, u32, u32, Vec<u8>)>,
        _running: bool,
    ) -> bool {
        false
    }

    /// Rebuild the view from the bridge (debug/PPU only).
    fn sync_view(&mut self) {}

    /// Share the debugger bridge for viewer creation (debug only).
    fn shared_bridge(&self) -> Option<Arc<DebugBridge>> {
        None
    }

    /// Take the pending settings apply on close (settings only).
    fn take_pending_settings(&mut self) -> Option<SettingsSnapshot> {
        None
    }

    /// Contended lock names, if any (debug only).
    fn debug_lock_audit(&self) -> Vec<&'static str> {
        Vec::new()
    }

    /// Take a history clear request (history only).
    fn take_history_clear(&mut self) -> bool {
        false
    }

    /// Push fresh history content (history only).
    fn sync_history(&mut self, _lines: Arc<[HistoryLine]>, _language: AppLanguage) {}
}

/// Route table for every secondary window, keyed by native id.
#[derive(Default)]
pub(crate) struct AuxRegistry {
    entries: Vec<Box<dyn AuxController>>,
}

impl AuxRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Seat a newly created aux window.
    pub(crate) fn insert(&mut self, entry: Box<dyn AuxController>) {
        self.entries.push(entry);
    }

    /// Kind of the window behind `id`, if registered.
    pub(crate) fn kind_of(&self, id: WindowId) -> Option<AuxKind> {
        self.entries
            .iter()
            .find(|entry| entry.tao_window_id() == id)
            .map(|entry| entry.kind())
    }

    /// Whether a window of this kind is currently open.
    pub(crate) fn contains(&self, kind: AuxKind) -> bool {
        self.entries.iter().any(|entry| entry.kind() == kind)
    }

    /// Pump one native event through the matching entry.
    pub(crate) fn on_event(&mut self, id: WindowId, event: WindowEvent) -> Option<AuxOutcome> {
        self.entries
            .iter_mut()
            .find(|entry| entry.tao_window_id() == id)
            .map(|entry| entry.on_event(event))
    }

    /// Render the open window of this kind, if any.
    pub(crate) fn render_kind(&mut self, kind: AuxKind) {
        if let Some(entry) = self.entry_mut(kind) {
            entry.on_redraw();
        }
    }

    /// Native id of the open window of this kind, if any.
    pub(crate) fn id_of(&self, kind: AuxKind) -> Option<WindowId> {
        self.entries
            .iter()
            .find(|entry| entry.kind() == kind)
            .map(|entry| entry.tao_window_id())
    }

    /// Drop the window of this kind, returning it for teardown.
    pub(crate) fn remove_kind(&mut self, kind: AuxKind) -> Option<Box<dyn AuxController>> {
        self.entries
            .iter()
            .position(|entry| entry.kind() == kind)
            .map(|index| self.entries.remove(index))
    }

    /// Drop every aux window (close teardown).
    pub(crate) fn close_all(&mut self) {
        self.entries.clear();
    }

    fn entry(&self, kind: AuxKind) -> Option<&dyn AuxController> {
        self.entries
            .iter()
            .find(|entry| entry.kind() == kind)
            .map(AsRef::as_ref)
    }

    fn entry_mut(&mut self, kind: AuxKind) -> Option<&mut dyn AuxController> {
        self.entries
            .iter_mut()
            .find(|entry| entry.kind() == kind)
            .map(|entry| &mut **entry as &mut dyn AuxController)
    }

    /// Debugger batch take (`None` when the debugger is closed).
    pub(crate) fn debug_take_requests(&mut self) -> Option<(bool, Vec<DebugAction>)> {
        self.entry_mut(AuxKind::Debug)
            .and_then(AuxController::take_debug_requests)
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
        if let Some(entry) = self.entry_mut(AuxKind::History) {
            entry.sync_history(lines, language);
        }
    }

    /// Take the pending settings apply on close.
    pub(crate) fn settings_take_pending(&mut self) -> Option<SettingsSnapshot> {
        self.entry_mut(AuxKind::Settings)
            .and_then(AuxController::take_pending_settings)
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
