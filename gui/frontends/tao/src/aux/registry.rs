//! Aux-window route table: one entry per secondary window.
//! Dispatch resolves native ids once; every entry pumps domain events.

use std::sync::Arc;

use nerust_frontend_host::aux::AuxWindows;
#[cfg(test)]
use nerust_frontend_host::aux::OpenRequest;
use nerust_frontend_host::{
    application::ApplicationHost, widget_session::WindowMailbox, window::Window,
};
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{DebugImageRgba, DisplayCache};

use super::bridge::{DebugBridge, DrainSnapshot};
use super::domain::{AuxId, AuxNativeEvent, NativeWindowId};
use super::focus::{AuxEvents, AuxIdentity, AuxKind, AuxRender, pump_aux};

/// Effects one aux event produced: computed in the entry, applied by the host.
#[derive(Debug, Clone, Default)]
pub(crate) struct AuxOutcome {
    pub id: Option<AuxId>,
    /// Close flag raised: drop the window.
    pub close: bool,
    /// Opaque work is waiting; only ApplicationHost may consume it.
    pub submitted: bool,
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
        _images: Vec<DebugImageRgba>,
        _running: bool,
    ) -> bool;
}

/// Generic session capability; native code never sees a domain controller.
pub(crate) trait WidgetSessionPort {
    fn refresh_widget(&mut self, host: &mut ApplicationHost) -> Window;
}

/// Uniform aux controller: identity plus render plus capability queries.
pub(crate) trait AuxController: AuxIdentity + AuxRender + AuxEvents {
    /// Pump one domain event: prelude, dispatch, outcome.
    fn on_event(&mut self, event: AuxNativeEvent) -> AuxOutcome {
        pump_aux(self, event);
        let mailbox = self.widget_mailbox();
        AuxOutcome {
            id: None,
            close: self.close_requested()
                || mailbox.as_ref().is_some_and(WindowMailbox::close_requested),
            submitted: mailbox.as_ref().is_some_and(WindowMailbox::has_pending),
            error: None,
        }
    }

    /// Full render through the shared pump.
    fn on_redraw(&mut self) -> AuxOutcome {
        let error = super::focus::AuxRender::render(self);
        let mailbox = self.widget_mailbox();
        AuxOutcome {
            id: None,
            close: self.close_requested()
                || mailbox.as_ref().is_some_and(WindowMailbox::close_requested),
            submitted: mailbox.as_ref().is_some_and(WindowMailbox::has_pending),
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

    fn as_widget(&mut self) -> Option<&mut dyn WidgetSessionPort> {
        None
    }

    fn widget_mailbox(&self) -> Option<WindowMailbox> {
        None
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

/// Native capability adapter; shared host owns all lifecycle and route state.
#[derive(Default)]
pub(crate) struct AuxRegistry {
    pub(crate) windows: AuxWindows<NativeWindowId, Box<dyn AuxController>>,
}

impl AuxRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn request_open(&mut self, kind: AuxKind) -> OpenRequest {
        self.windows.request_open(kind)
    }

    pub(crate) fn window(&self, id: AuxId) -> Option<&nerust_frontend_host::window::Window> {
        self.windows.window(id)
    }

    pub(crate) fn window_mut(
        &mut self,
        id: AuxId,
    ) -> Option<&mut nerust_frontend_host::window::Window> {
        self.windows.window_mut(id)
    }

    pub(crate) fn complete_open(
        &mut self,
        id: AuxId,
        entry: Box<dyn AuxController>,
    ) -> Result<(), Box<dyn AuxController>> {
        let kind = entry.kind();
        if self.windows.kind_of_id(id) != Some(kind) {
            return Err(entry);
        }
        let result = self
            .windows
            .complete_open(id, Some(entry.native_id()), entry);
        if result.is_ok() {
            debug_assert_eq!(self.route_id(kind), Some(id));
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn cancel_open(&mut self, id: AuxId) -> bool {
        self.windows.cancel_open(id)
    }

    /// Compatibility helper for stubs; replacing a kind removes its old native route.
    #[cfg(test)]
    pub(crate) fn insert(&mut self, entry: Box<dyn AuxController>) -> AuxId {
        let kind = entry.kind();
        self.windows.remove(kind);
        let OpenRequest::Create(id) = self.request_open(kind) else {
            unreachable!("removed kind must reserve a new id");
        };
        if self.complete_open(id, entry).is_err() {
            self.cancel_open(id);
            panic!("duplicate native auxiliary window identity");
        }
        id
    }

    /// Route id of the open window of this kind, if any.
    pub(crate) fn route_id(&self, kind: AuxKind) -> Option<AuxId> {
        self.windows.route_id(kind)
    }

    /// Kind of the window behind `id`, if registered.
    pub(crate) fn kind_of(&self, id: NativeWindowId) -> Option<AuxKind> {
        self.windows.kind_of(id)
    }

    /// Whether a window of this kind is currently open.
    pub(crate) fn contains(&self, kind: AuxKind) -> bool {
        self.windows.contains(kind)
    }

    /// Pump one domain event through the entry of this kind.
    pub(crate) fn on_event(&mut self, kind: AuxKind, event: AuxNativeEvent) -> Option<AuxOutcome> {
        let id = self.route_id(kind)?;
        self.windows.get_mut(kind).map(|entry| {
            let mut outcome = entry.on_event(event);
            outcome.id = Some(id);
            outcome
        })
    }

    /// Render the open window of this kind, if any.
    pub(crate) fn render_kind(&mut self, kind: AuxKind) -> Option<AuxOutcome> {
        let id = self.route_id(kind)?;
        self.windows.get_mut(kind).map(|entry| {
            let mut outcome = entry.on_redraw();
            outcome.id = Some(id);
            outcome
        })
    }

    pub(crate) fn pump_deadlines(&mut self, now: std::time::Instant) -> Option<std::time::Instant> {
        self.windows
            .values_mut()
            .filter_map(|entry| {
                entry.wake_due(now);
                entry.wake_deadline()
            })
            .min()
    }

    /// Drop the window of this kind, returning it for teardown.
    pub(crate) fn remove_kind(&mut self, kind: AuxKind) -> Option<Box<dyn AuxController>> {
        self.windows.remove(kind)
    }

    /// Drop every aux window (close teardown).
    pub(crate) fn close_all(&mut self) {
        self.windows.clear();
    }

    fn entry(&self, kind: AuxKind) -> Option<&(dyn AuxController + '_)> {
        self.windows.get(kind).map(|entry| &**entry)
    }

    fn entry_mut(&mut self, kind: AuxKind) -> Option<&mut (dyn AuxController + '_)> {
        match self.windows.get_mut(kind) {
            Some(entry) => Some(&mut **entry),
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
        self.windows
            .get(AuxKind::Debug)
            .and_then(|entry| entry.debug_snapshot_of(ppu_open))
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
        images: Vec<DebugImageRgba>,
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

    pub(crate) fn refresh_widget(&mut self, id: AuxId, host: &mut ApplicationHost) {
        let Some(kind) = self.windows.kind_of_id(id) else {
            return;
        };
        let window = self
            .entry_mut(kind)
            .and_then(AuxController::as_widget)
            .map(|port| port.refresh_widget(host));
        if let Some(window) = window
            && let Some(current) = self.window_mut(id)
        {
            *current = window;
        }
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

#[cfg(test)]
pub(crate) mod test_double {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Arc;

    use nerust_gui_shell::debug::DebugAction;

    use crate::aux::bridge::{DebugBridge, DrainSnapshot};
    use crate::aux::domain::{AuxNativeEvent, NativeWindowId};
    use crate::aux::focus::{AuxEvents, AuxIdentity, AuxKind, AuxRender};
    use nerust_gui_viewmodel::debugger::DebugImageRgba;

    use super::{AuxController, DebugDrainPort, LiveImagesPort, WindowMailbox};
    /// Configurable stub aux: answers only the capability its kind owns.
    pub(crate) struct StubAux {
        pub kind: AuxKind,
        pub nid: NativeWindowId,
        pub close: bool,
        pub mailbox: Option<WindowMailbox>,
        pub live_rebuild: bool,
        pub applied_drain: Rc<Cell<bool>>,
        pub bridge: Option<Arc<DebugBridge>>,
        pub drain_open: bool,
        pub debug_batch: Vec<DebugAction>,
        pub reg_target: Option<&'static str>,
    }

    impl StubAux {
        pub(crate) fn new(kind: AuxKind) -> Self {
            Self {
                kind,
                nid: NativeWindowId::dummy(),
                close: false,
                mailbox: None,
                live_rebuild: false,
                applied_drain: Rc::new(Cell::new(false)),
                bridge: None,
                drain_open: false,
                debug_batch: Vec::new(),
                reg_target: None,
            }
        }
    }

    impl AuxIdentity for StubAux {
        fn kind(&self) -> AuxKind {
            self.kind
        }

        fn native_id(&self) -> NativeWindowId {
            self.nid
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
            _images: Vec<DebugImageRgba>,
            _running: bool,
        ) -> bool {
            self.live_rebuild
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

        fn widget_mailbox(&self) -> Option<WindowMailbox> {
            self.mailbox.clone()
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
    use nerust_core_traits::debugger::SpaceId;

    #[test]
    fn reserved_window_snapshot_allows_reentry_and_stale_ids_cannot_touch_reopen() {
        use nerust_frontend_host::window::{DebugContents, WidgetContents, WindowVisitor};
        use std::cell::RefCell;

        let registry = RefCell::new(AuxRegistry::new());
        let OpenRequest::Create(old) = registry.borrow_mut().request_open(AuxKind::Debug) else {
            panic!("expected reservation");
        };
        let window = registry.borrow().window(old).unwrap().clone();
        assert!(matches!(window.contents.contents, WidgetContents::Debug(_)));
        struct Reenter<'a>(&'a RefCell<AuxRegistry>, AuxId, bool);
        impl WindowVisitor for Reenter<'_> {
            fn visit_debug(&mut self, _: &DebugContents) {
                assert_eq!(
                    self.0.borrow_mut().request_open(AuxKind::Debug),
                    OpenRequest::Pending(self.1)
                );
                self.2 = true;
            }
        }
        let mut visitor = Reenter(&registry, old, false);
        window.visit(&mut visitor);
        assert!(visitor.2);
        assert!(registry.borrow_mut().cancel_open(old));
        let OpenRequest::Create(current) = registry.borrow_mut().request_open(AuxKind::Debug)
        else {
            panic!("expected reopen");
        };
        assert_ne!(old, current);
        assert!(registry.borrow().window(old).is_none());
        assert!(registry.borrow_mut().window_mut(old).is_none());
        assert!(
            registry
                .borrow_mut()
                .complete_open(old, Box::new(StubAux::new(AuxKind::Debug)))
                .is_err()
        );
        assert_eq!(registry.borrow().route_id(AuxKind::Debug), Some(current));
        assert!(
            registry
                .borrow_mut()
                .complete_open(current, Box::new(StubAux::new(AuxKind::Debug)))
                .is_ok()
        );
        assert!(registry.borrow().window(current).is_some());
    }

    #[test]
    fn replacement_cleans_old_native_route_and_mints_new_id() {
        let mut registry = AuxRegistry::new();
        let first = StubAux::new(AuxKind::Debug);
        let old_native = first.nid;
        let old_id = registry.insert(Box::new(first));
        let second = StubAux::new(AuxKind::Debug);
        let new_native = second.nid;
        let new_id = registry.insert(Box::new(second));
        assert_ne!(old_id, new_id);
        assert_eq!(registry.kind_of(old_native), None);
        assert_eq!(registry.kind_of(new_native), Some(AuxKind::Debug));
        registry.remove_kind(AuxKind::Debug);
        assert_eq!(registry.kind_of(new_native), None);
    }

    #[test]
    fn opening_arbitration_rejects_wrong_kind_and_duplicate_native() {
        let mut registry = AuxRegistry::new();
        let OpenRequest::Create(debug) = registry.request_open(AuxKind::Debug) else {
            panic!("expected debugger reservation");
        };
        assert!(registry.contains(AuxKind::Debug));
        assert!(registry.debug_snapshot(false).is_none());
        assert!(registry.render_kind(AuxKind::Debug).is_none());
        assert_eq!(
            registry.request_open(AuxKind::Debug),
            OpenRequest::Pending(debug)
        );
        let wrong = Box::new(StubAux::new(AuxKind::Ppu));
        let wrong_native = wrong.nid;
        let rejected = registry.complete_open(debug, wrong).err().unwrap();
        assert_eq!(rejected.kind(), AuxKind::Ppu);
        assert_eq!(registry.kind_of(wrong_native), None);
        let handle = StubAux::new(AuxKind::Debug);
        let native = handle.nid;
        assert!(registry.complete_open(debug, Box::new(handle)).is_ok());
        assert_eq!(
            registry.request_open(AuxKind::Debug),
            OpenRequest::Present(debug)
        );
        let OpenRequest::Create(ppu) = registry.request_open(AuxKind::Ppu) else {
            panic!("expected PPU reservation");
        };
        let mut collision = StubAux::new(AuxKind::Ppu);
        collision.nid = native;
        assert!(registry.complete_open(ppu, Box::new(collision)).is_err());
        assert_eq!(registry.kind_of(native), Some(AuxKind::Debug));
        assert!(registry.cancel_open(ppu));
        assert!(!registry.contains(AuxKind::Ppu));
        assert!(!registry.cancel_open(debug));
    }

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
        let absent = NativeWindowId::dummy();
        let history = StubAux::new(AuxKind::History);
        let seated = history.nid;
        assert_eq!(registry.kind_of(absent), None);
        registry.insert(Box::new(history));
        // The seated stub routes back by its own minted id.
        assert_eq!(registry.kind_of(seated), Some(AuxKind::History));
        assert_ne!(seated, absent);
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
        let mut host = crate::runtime::host::TestHost::new().host;
        let (_, mut session) =
            crate::aux::widgets::bindings::tests::prepare(&mut host, AuxKind::History);
        crate::aux::widgets::bindings::tests::activate(&mut session, "Clear History");
        stub.mailbox = Some(session.mailbox());
        stub.close = true;
        registry.insert(Box::new(stub));
        let outcome = registry
            .on_event(AuxKind::History, AuxNativeEvent::Focused(true))
            .expect("history routes");
        assert!(outcome.close);
        assert!(outcome.submitted);
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
        let mut host = crate::runtime::host::TestHost::new().host;
        let OpenRequest::Create(absent) = host.reserve_aux_window(AuxKind::History).unwrap() else {
            panic!("reservation")
        };
        registry.refresh_widget(absent, &mut host);
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
        assert!(
            registry
                .windows
                .mailbox(registry.route_id(AuxKind::Settings).unwrap())
                .is_none()
        );
    }
}
