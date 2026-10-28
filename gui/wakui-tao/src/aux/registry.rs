//! Aux-window route table: one entry per secondary window.
//! Dispatch resolves native ids once; every entry pumps domain events.
//!
//! The table is generic over the view-side aux kind `K`: entries are routed
//! by [`AuxId`], kinds only arbitrate reservations and opens.

use super::focus::{AuxEvents, AuxIdentity, AuxRender, pump_aux};
use crate::domain::{AuxId, AuxNativeEvent, NativeWindowId};
use wakui::aux::AuxWindows;
#[cfg(test)]
use wakui::aux::OpenRequest;
use wakui::control::{ControlEvent, ControlScreen};
use wakui::mailbox::WindowMailbox;

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

/// Generic widget capability: the host pushes screens, native drains events.
/// Native code never sees a domain controller or session.
pub(crate) trait WidgetSessionPort {
    /// Push the latest app screen; rebuild the native view from it.
    fn push_snapshot(&mut self, screen: &ControlScreen);
    /// Take queued control events for the host drain.
    fn drain_events(&mut self) -> Vec<ControlEvent>;
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

    /// Full render through the shared pump, tagged by the window title.
    fn on_redraw(&mut self, title: &str) -> AuxOutcome {
        let error = super::focus::AuxRender::render(self, title);
        let mailbox = self.widget_mailbox();
        AuxOutcome {
            id: None,
            close: self.close_requested()
                || mailbox.as_ref().is_some_and(WindowMailbox::close_requested),
            submitted: mailbox.as_ref().is_some_and(WindowMailbox::has_pending),
            error,
        }
    }

    fn as_widget(&mut self) -> Option<&mut dyn WidgetSessionPort> {
        None
    }

    fn widget_mailbox(&self) -> Option<WindowMailbox> {
        None
    }
}

/// Native capability adapter; shared host owns all lifecycle and route state.
pub(crate) struct AuxRegistry<K: Copy + Eq + std::hash::Hash + 'static> {
    pub(crate) windows: AuxWindows<K, NativeWindowId, Box<dyn AuxController>>,
}

impl<K: Copy + Eq + std::hash::Hash + 'static> Default for AuxRegistry<K> {
    fn default() -> Self {
        Self {
            windows: AuxWindows::new(),
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash + 'static> AuxRegistry<K> {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn request_open(&mut self, kind: K, title: &str) -> OpenRequest {
        self.windows.request_open(kind, title)
    }

    pub(crate) fn window(&self, id: AuxId) -> Option<&wakui::window::Window> {
        self.windows.window(id)
    }

    #[allow(dead_code)]
    pub(crate) fn window_mut(&mut self, id: AuxId) -> Option<&mut wakui::window::Window> {
        self.windows.window_mut(id)
    }

    pub(crate) fn complete_open(
        &mut self,
        id: AuxId,
        entry: Box<dyn AuxController>,
    ) -> Result<(), Box<dyn AuxController>> {
        if self.windows.kind_of_id(id).is_none() {
            return Err(entry);
        }
        let result = self
            .windows
            .complete_open(id, Some(entry.native_id()), entry);
        if result.is_ok() {
            debug_assert!(self.windows.kind_of_id(id).is_some());
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn cancel_open(&mut self, id: AuxId) -> bool {
        self.windows.cancel_open(id)
    }

    /// Test-only seating: replace any window of this kind, then complete.
    #[cfg(test)]
    pub(crate) fn insert_for_test(&mut self, kind: K, entry: Box<dyn AuxController>) -> AuxId {
        self.windows.remove(kind);
        let OpenRequest::Create(id) = self.request_open(kind, "test") else {
            unreachable!("removed kind must reserve a new id");
        };
        if self.complete_open(id, entry).is_err() {
            self.cancel_open(id);
            panic!("duplicate native auxiliary window identity");
        }
        id
    }

    /// Route id of the open window of this kind, if any.
    #[allow(dead_code)]
    pub(crate) fn route_id(&self, kind: K) -> Option<AuxId> {
        self.windows.route_id(kind)
    }

    /// Kind of the window behind this native id, if registered.
    #[allow(dead_code)]
    pub(crate) fn kind_of(&self, id: NativeWindowId) -> Option<K> {
        self.windows.kind_of(id)
    }

    /// Id of the window behind this native id, if registered.
    pub(crate) fn id_of(&self, id: NativeWindowId) -> Option<AuxId> {
        self.windows.id_of(id)
    }

    /// Whether a window of this kind is currently open.
    #[allow(dead_code)]
    pub(crate) fn contains(&self, kind: K) -> bool {
        self.windows.contains(kind)
    }

    /// Pump one domain event through the entry behind this id.
    pub(crate) fn on_event(&mut self, id: AuxId, event: AuxNativeEvent) -> Option<AuxOutcome> {
        self.windows.get_mut_by_id(id).map(|entry| {
            let mut outcome = entry.on_event(event);
            outcome.id = Some(id);
            outcome
        })
    }

    /// Render the window behind this id, tagged by its current title.
    pub(crate) fn render_id(&mut self, id: AuxId) -> Option<AuxOutcome> {
        let title = self.windows.window(id)?.title.clone();
        self.windows.get_mut_by_id(id).map(|entry| {
            let mut outcome = entry.on_redraw(&title);
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
    #[allow(dead_code)]
    pub(crate) fn remove_kind(&mut self, kind: K) -> Option<Box<dyn AuxController>> {
        self.windows.remove(kind)
    }

    /// Drop the window behind this id, returning it for teardown.
    #[allow(dead_code)]
    pub(crate) fn remove_id(&mut self, id: AuxId) -> Option<Box<dyn AuxController>> {
        self.windows.remove_id(id)
    }

    /// Drop every aux window (close teardown).
    pub(crate) fn close_all(&mut self) {
        self.windows.clear();
    }

    fn entry(&self, id: AuxId) -> Option<&(dyn AuxController + '_)> {
        self.windows.get_by_id(id).map(|entry| &**entry)
    }

    fn entry_mut(&mut self, id: AuxId) -> Option<&mut (dyn AuxController + '_)> {
        match self.windows.get_mut_by_id(id) {
            Some(entry) => Some(&mut **entry),
            None => None,
        }
    }

    /// Push the latest widget screen into the window behind this id.
    pub(crate) fn push_widget_snapshot(&mut self, id: AuxId, screen: &ControlScreen) -> bool {
        self.entry_mut(id)
            .and_then(AuxController::as_widget)
            .map(|port| {
                port.push_snapshot(screen);
            })
            .is_some()
    }

    /// Take queued control events from the window behind this id.
    pub(crate) fn drain_widget_events(&mut self, id: AuxId) -> Vec<ControlEvent> {
        self.entry_mut(id)
            .and_then(AuxController::as_widget)
            .map(|port| port.drain_events())
            .unwrap_or_default()
    }

    /// Bring the window behind this id to the front, if open.
    pub(crate) fn focus_id(&self, id: AuxId) {
        if let Some(entry) = self.entry(id) {
            entry.focus_window();
        }
    }
}

#[cfg(test)]
pub(crate) mod test_double {
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::aux::focus::{AuxEvents, AuxIdentity, AuxRender};
    use crate::domain::{AuxNativeEvent, NativeWindowId};

    use super::{AuxController, WidgetSessionPort};
    use wakui::control::{ControlEvent, ControlScreen};
    use wakui::mailbox::WindowMailbox;

    /// Configurable stub aux: a generic widget window.
    pub(crate) struct StubAux {
        pub nid: NativeWindowId,
        pub close: bool,
        pub mailbox: Option<WindowMailbox>,
        pub pushed_screen: Rc<Cell<bool>>,
        pub widget_queue: Vec<ControlEvent>,
    }

    impl StubAux {
        pub(crate) fn new() -> Self {
            Self {
                nid: NativeWindowId::dummy(),
                close: false,
                mailbox: None,
                pushed_screen: Rc::new(Cell::new(false)),
                widget_queue: Vec::new(),
            }
        }
    }

    impl AuxIdentity for StubAux {
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
        fn render(&mut self, _title: &str) -> Option<String> {
            None
        }
    }

    impl AuxEvents for StubAux {
        fn handle(&mut self, _mapped: iced::Event) -> bool {
            false
        }
    }

    impl WidgetSessionPort for StubAux {
        fn push_snapshot(&mut self, _screen: &ControlScreen) {
            self.pushed_screen.set(true);
        }

        fn drain_events(&mut self) -> Vec<ControlEvent> {
            std::mem::take(&mut self.widget_queue)
        }
    }

    impl AuxController for StubAux {
        fn as_widget(&mut self) -> Option<&mut dyn WidgetSessionPort> {
            Some(self as &mut dyn WidgetSessionPort)
        }

        fn widget_mailbox(&self) -> Option<WindowMailbox> {
            self.mailbox.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_double::StubAux;
    use super::*;
    use crate::testkit::TestAux;

    fn registry() -> AuxRegistry<TestAux> {
        AuxRegistry::new()
    }

    #[test]
    fn reserved_window_snapshot_allows_reentry_and_stale_ids_cannot_touch_reopen() {
        use std::cell::RefCell;
        use wakui::window::{WidgetContents, WindowVisitor};

        let registry = RefCell::new(registry());
        let OpenRequest::Create(old) = registry
            .borrow_mut()
            .request_open(TestAux::Settings, TestAux::Settings.title())
        else {
            panic!("expected reservation");
        };
        let window = registry.borrow().window(old).unwrap().clone();
        assert!(matches!(
            window.contents.contents,
            WidgetContents::PendingControls
        ));
        struct Reenter<'a>(&'a RefCell<AuxRegistry<TestAux>>, AuxId, bool);
        impl WindowVisitor for Reenter<'_> {
            fn visit_pending_controls(&mut self) {
                assert_eq!(
                    self.0
                        .borrow_mut()
                        .request_open(TestAux::Settings, TestAux::Settings.title()),
                    OpenRequest::Pending(self.1)
                );
                self.2 = true;
            }
        }
        let mut visitor = Reenter(&registry, old, false);
        window.visit(&mut visitor);
        assert!(visitor.2);
        assert!(registry.borrow_mut().cancel_open(old));
        let OpenRequest::Create(current) = registry
            .borrow_mut()
            .request_open(TestAux::Settings, TestAux::Settings.title())
        else {
            panic!("expected reopen");
        };
        assert_ne!(old, current);
        assert!(registry.borrow().window(old).is_none());
        assert!(registry.borrow_mut().window_mut(old).is_none());
        assert!(
            registry
                .borrow_mut()
                .complete_open(old, Box::new(StubAux::new()))
                .is_err()
        );
        assert_eq!(registry.borrow().route_id(TestAux::Settings), Some(current));
        assert!(
            registry
                .borrow_mut()
                .complete_open(current, Box::new(StubAux::new()))
                .is_ok()
        );
        assert!(registry.borrow().window(current).is_some());
    }

    #[test]
    fn replacement_cleans_old_native_route_and_mints_new_id() {
        let mut registry = registry();
        let first = StubAux::new();
        let old_native = first.nid;
        let old_id = registry.insert_for_test(TestAux::Settings, Box::new(first));
        let second = StubAux::new();
        let new_native = second.nid;
        let new_id = registry.insert_for_test(TestAux::Settings, Box::new(second));
        assert_ne!(old_id, new_id);
        assert_eq!(registry.kind_of(old_native), None);
        assert_eq!(registry.kind_of(new_native), Some(TestAux::Settings));
        registry.remove_kind(TestAux::Settings);
        assert_eq!(registry.kind_of(new_native), None);
    }

    #[test]
    fn opening_arbitration_rejects_wrong_kind_and_duplicate_native() {
        let mut registry = registry();
        let OpenRequest::Create(settings) = registry.request_open(TestAux::Settings, "Settings")
        else {
            panic!("expected settings reservation");
        };
        assert!(registry.contains(TestAux::Settings));
        assert!(registry.render_id(settings).is_none());
        assert_eq!(
            registry.request_open(TestAux::Settings, "Settings"),
            OpenRequest::Pending(settings)
        );
        let wrong_native = NativeWindowId::dummy();
        let mut wrong = StubAux::new();
        wrong.nid = wrong_native;
        let rejected = registry.complete_open(settings, Box::new(wrong));
        // Native collision is rejected only after seating; a fresh native
        // completes the reservation.
        assert!(rejected.is_ok());
        assert_eq!(registry.kind_of(wrong_native), Some(TestAux::Settings));
        let OpenRequest::Create(other) = registry.request_open(TestAux::Debug, "Debugger") else {
            panic!("expected debug reservation");
        };
        let mut collision = StubAux::new();
        collision.nid = wrong_native;
        assert!(registry.complete_open(other, Box::new(collision)).is_err());
        assert_eq!(registry.kind_of(wrong_native), Some(TestAux::Settings));
        assert!(registry.cancel_open(other));
        assert!(!registry.contains(TestAux::Debug));
        assert!(!registry.cancel_open(settings));
    }

    #[test]
    fn registry_routes_by_map_without_scans() {
        let mut registry = registry();
        assert!(!registry.contains(TestAux::Settings));
        let id = registry.insert_for_test(TestAux::Settings, Box::new(StubAux::new()));
        assert_eq!(registry.route_id(TestAux::Settings), Some(id));
        assert!(registry.contains(TestAux::Settings));
        // Event and render route by id; close flag stays clear.
        let outcome = registry
            .on_event(id, AuxNativeEvent::Focused(true))
            .expect("settings routes");
        assert!(!outcome.close);
        registry.render_id(id);
        registry.focus_id(id);
        // Removal drops the route; re-insert assigns a fresh id.
        assert!(registry.remove_kind(TestAux::Settings).is_some());
        assert!(!registry.contains(TestAux::Settings));
        let second = registry.insert_for_test(TestAux::Settings, Box::new(StubAux::new()));
        assert_ne!(id, second);
    }

    #[test]
    fn registry_tracks_native_ids_and_clears_all() {
        let mut registry = registry();
        let absent = NativeWindowId::dummy();
        let history = StubAux::new();
        let seated = history.nid;
        assert_eq!(registry.kind_of(absent), None);
        registry.insert_for_test(TestAux::History, Box::new(history));
        // The seated stub routes back by its own minted id.
        assert_eq!(registry.kind_of(seated), Some(TestAux::History));
        assert_ne!(seated, absent);
        assert!(registry.remove_kind(TestAux::Settings).is_none());
        assert!(!registry.contains(TestAux::Settings));
        // Close-all drops every route at once.
        registry.close_all();
        assert!(!registry.contains(TestAux::History));
        assert_eq!(registry.route_id(TestAux::History), None);
    }

    #[test]
    fn on_event_reports_close_and_clear_flags() {
        let mut registry = AuxRegistry::new();
        let mut stub = StubAux::new();
        let mut host = crate::runtime::host::TestHost::new().host;
        let id = host
            .reserve_aux_window(TestAux::History)
            .and_then(|request| match request {
                OpenRequest::Create(id) => Some(id),
                _ => None,
            })
            .expect("reservation");
        assert!(
            host.core
                .prepare_widget(&mut host.aux.windows, id)
                .expect("prepared widget")
        );
        stub.mailbox = host.core.widget_mailbox(id);
        stub.close = true;
        let seat = registry.insert_for_test(TestAux::History, Box::new(stub));
        let outcome = registry
            .on_event(seat, AuxNativeEvent::Focused(true))
            .expect("history routes");
        assert!(outcome.close);
        // Default controller hooks stay quiet.
        registry.render_id(seat);
        registry.focus_id(seat);
        let mut bare = StubAux::new();
        AuxController::on_redraw(&mut bare, "History");
    }

    #[test]
    fn widget_ports_push_and_drain_through_the_open_entry() {
        use wakui::control::{ControlEvent, ControlInput};

        let mut host = crate::runtime::host::TestHost::new().host;
        let id = host
            .reserve_aux_window(TestAux::History)
            .and_then(|request| match request {
                OpenRequest::Create(id) => Some(id),
                _ => None,
            })
            .expect("reservation");
        assert!(
            host.core
                .prepare_widget(&mut host.aux.windows, id)
                .expect("prepared widget")
        );
        let screen = host.core.widget_screen(id).expect("screen");
        let control = screen
            .pages
            .iter()
            .flat_map(|page| [&page.select])
            .next()
            .expect("history has a filter control");
        let mut registry = registry();
        let mut stub = StubAux::new();
        stub.widget_queue
            .push(ControlEvent::new(control.id, ControlInput::Activate));
        let pushed = stub.pushed_screen.clone();
        let seat = registry.insert_for_test(TestAux::History, Box::new(stub));
        assert_eq!(
            registry.drain_widget_events(seat).len(),
            1,
            "queued widget event drains"
        );
        assert!(registry.drain_widget_events(seat).is_empty());
        assert!(registry.push_widget_snapshot(seat, &screen));
        assert!(pushed.get());
    }
}
