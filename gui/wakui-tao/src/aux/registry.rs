//! Aux-window route table: one entry per secondary window.
//! Dispatch resolves native ids once; every entry pumps domain events.

use super::debug::{DebugMessage, views::ViewSnapshot};
use super::focus::{AuxEvents, AuxIdentity, AuxKind, AuxRender, pump_aux};
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

    /// Push the latest debugger snapshot (debug only).
    fn push_debug_snapshot(&mut self, _snapshot: &ViewSnapshot) -> bool {
        false
    }

    /// Take queued debugger gestures for the host drain (debug only).
    fn drain_debug_queue(&mut self) -> Vec<DebugMessage> {
        Vec::new()
    }

    fn as_widget(&mut self) -> Option<&mut dyn WidgetSessionPort> {
        None
    }

    fn as_ppu(&mut self) -> Option<&mut super::ppu::PpuWindowHandle> {
        None
    }

    fn widget_mailbox(&self) -> Option<WindowMailbox> {
        None
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

    pub(crate) fn window(&self, id: AuxId) -> Option<&wakui::window::Window> {
        self.windows.window(id)
    }

    pub(crate) fn window_mut(&mut self, id: AuxId) -> Option<&mut wakui::window::Window> {
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

    /// Push the latest debugger snapshot into the open debug window.
    pub(crate) fn push_debug_snapshot(&mut self, snapshot: &ViewSnapshot) -> bool {
        self.entry_mut(AuxKind::Debug)
            .map(|entry| entry.push_debug_snapshot(snapshot))
            .unwrap_or(false)
    }

    /// Take queued debugger gestures from the open debug window.
    pub(crate) fn drain_debug_queue(&mut self) -> Vec<DebugMessage> {
        self.entry_mut(AuxKind::Debug)
            .map(|entry| entry.drain_debug_queue())
            .unwrap_or_default()
    }

    /// Push staged pictures into the open PPU viewer.
    pub(crate) fn push_ppu_pictures(&mut self, pictures: super::ppu::PpuPictures) {
        if let Some(entry) = self
            .entry_mut(AuxKind::Ppu)
            .and_then(|entry| entry.as_ppu())
        {
            entry.push_pictures(pictures);
        }
    }

    /// Take queued hover coordinates from the open PPU viewer.
    pub(crate) fn drain_ppu_hover(&mut self) -> Vec<super::ppu::HoverAt> {
        self.entry_mut(AuxKind::Ppu)
            .and_then(|entry| entry.as_ppu())
            .map(|viewer| viewer.drain_hover())
            .unwrap_or_default()
    }

    /// Show app-formatted hover text on the open PPU viewer.
    pub(crate) fn push_ppu_hover(&mut self, line: String) {
        if let Some(entry) = self
            .entry_mut(AuxKind::Ppu)
            .and_then(|entry| entry.as_ppu())
        {
            entry.push_hover(line);
        }
    }

    /// Push the latest widget screen into the open widget window.
    pub(crate) fn push_widget_snapshot(&mut self, id: AuxId, screen: &ControlScreen) -> bool {
        let Some(kind) = self.windows.kind_of_id(id) else {
            return false;
        };
        self.entry_mut(kind)
            .and_then(AuxController::as_widget)
            .map(|port| {
                port.push_snapshot(screen);
            })
            .is_some()
    }

    /// Take queued control events from the open widget window.
    pub(crate) fn drain_widget_events(&mut self, id: AuxId) -> Vec<ControlEvent> {
        let Some(kind) = self.windows.kind_of_id(id) else {
            return Vec::new();
        };
        self.entry_mut(kind)
            .and_then(AuxController::as_widget)
            .map(|port| port.drain_events())
            .unwrap_or_default()
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

    use crate::aux::focus::{AuxEvents, AuxIdentity, AuxKind, AuxRender};
    use crate::domain::{AuxNativeEvent, NativeWindowId};

    use super::{AuxController, WidgetSessionPort};
    use crate::aux::debug::views::ViewSnapshot;
    use wakui::control::{ControlEvent, ControlScreen};
    use wakui::mailbox::WindowMailbox;

    /// Configurable stub aux: answers only the capability its kind owns.
    pub(crate) struct StubAux {
        pub kind: AuxKind,
        pub nid: NativeWindowId,
        pub close: bool,
        pub mailbox: Option<WindowMailbox>,
        pub pushed_snapshot: Rc<Cell<bool>>,
        pub pushed_screen: Rc<Cell<bool>>,
        pub debug_queue: Vec<super::DebugMessage>,
        pub widget_queue: Vec<ControlEvent>,
    }

    impl StubAux {
        pub(crate) fn new(kind: AuxKind) -> Self {
            Self {
                kind,
                nid: NativeWindowId::dummy(),
                close: false,
                mailbox: None,
                pushed_snapshot: Rc::new(Cell::new(false)),
                pushed_screen: Rc::new(Cell::new(false)),
                debug_queue: Vec::new(),
                widget_queue: Vec::new(),
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

    impl WidgetSessionPort for StubAux {
        fn push_snapshot(&mut self, _screen: &ControlScreen) {
            self.pushed_screen.set(true);
        }

        fn drain_events(&mut self) -> Vec<ControlEvent> {
            std::mem::take(&mut self.widget_queue)
        }
    }

    impl AuxController for StubAux {
        fn push_debug_snapshot(&mut self, _snapshot: &ViewSnapshot) -> bool {
            if self.kind != AuxKind::Debug {
                return false;
            }
            self.pushed_snapshot.set(true);
            true
        }

        fn drain_debug_queue(&mut self) -> Vec<super::DebugMessage> {
            if self.kind != AuxKind::Debug {
                return Vec::new();
            }
            std::mem::take(&mut self.debug_queue)
        }

        fn as_widget(&mut self) -> Option<&mut dyn WidgetSessionPort> {
            (self.kind == AuxKind::Settings || self.kind == AuxKind::History)
                .then_some(self as &mut dyn WidgetSessionPort)
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

    #[test]
    fn reserved_window_snapshot_allows_reentry_and_stale_ids_cannot_touch_reopen() {
        use std::cell::RefCell;
        use wakui::window::{DebugContents, WidgetContents, WindowVisitor};

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
        assert!(registry.render_kind(AuxKind::Debug).is_none());
        assert_eq!(
            registry.request_open(AuxKind::Debug),
            OpenRequest::Pending(debug)
        );
        assert!(registry.drain_debug_queue().is_empty());
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
        let id = host
            .reserve_aux_window(AuxKind::History)
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
        registry.insert(Box::new(stub));
        let outcome = registry
            .on_event(AuxKind::History, AuxNativeEvent::Focused(true))
            .expect("history routes");
        assert!(outcome.close);
        // Default controller hooks stay quiet.
        registry.render_kind(AuxKind::History);
        registry.focus_kind(AuxKind::History);
        let mut bare = StubAux::new(AuxKind::History);
        AuxController::on_redraw(&mut bare);
    }

    #[test]
    fn debug_ports_delegate_to_the_open_entry() {
        use crate::aux::debug::{DebugMessage, views::ViewSnapshot};

        let mut registry = AuxRegistry::new();
        // Closed debugger: every port stays quiet.
        assert!(registry.drain_debug_queue().is_empty());
        assert!(!registry.push_debug_snapshot(&ViewSnapshot::default()));
        // Open debugger: queue delegates; snapshots fan out.
        let mut stub = StubAux::new(AuxKind::Debug);
        stub.debug_queue.push(DebugMessage::RefreshPressed);
        let pushed = stub.pushed_snapshot.clone();
        registry.insert(Box::new(stub));
        assert!(matches!(
            registry.drain_debug_queue().as_slice(),
            [DebugMessage::RefreshPressed]
        ));
        assert!(registry.drain_debug_queue().is_empty());
        assert!(registry.push_debug_snapshot(&ViewSnapshot::default()));
        assert!(pushed.get());
        // Kind gating: a settings stub never answers debug ports.
        let mut settings = AuxRegistry::new();
        settings.insert(Box::new(StubAux::new(AuxKind::Settings)));
        assert!(settings.drain_debug_queue().is_empty());
        assert!(!settings.push_debug_snapshot(&ViewSnapshot::default()));
    }

    #[test]
    fn widget_ports_push_and_drain_through_the_open_entry() {
        use wakui::control::{ControlEvent, ControlInput};

        let mut host = crate::runtime::host::TestHost::new().host;
        let id = host
            .reserve_aux_window(AuxKind::History)
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
        let mut registry = AuxRegistry::new();
        let mut stub = StubAux::new(AuxKind::History);
        stub.widget_queue
            .push(ControlEvent::new(control.id, ControlInput::Activate));
        let pushed = stub.pushed_screen.clone();
        let seat = registry.insert(Box::new(stub));
        assert_eq!(
            registry.drain_widget_events(seat).len(),
            1,
            "queued widget event drains"
        );
        assert!(registry.drain_widget_events(seat).is_empty());
        assert!(registry.push_widget_snapshot(seat, &screen));
        assert!(pushed.get());
        // Kind gating (not just routing) keeps a seated settings
        // window from answering debugger-only ports.
        let mut other = AuxRegistry::new();
        other.insert(Box::new(StubAux::new(AuxKind::Settings)));
        assert!(other.drain_debug_queue().is_empty());
        assert!(
            other
                .windows
                .mailbox(other.route_id(AuxKind::Settings).unwrap())
                .is_none()
        );
    }

    #[test]
    fn wrong_kind_never_answers_debug_ports() {
        // Kind gating (not just routing) keeps a seated settings
        // window from answering debugger-only ports.
        let mut registry = AuxRegistry::new();
        registry.insert(Box::new(StubAux::new(AuxKind::Settings)));
        assert!(registry.drain_debug_queue().is_empty());
        assert!(
            registry
                .windows
                .mailbox(registry.route_id(AuxKind::Settings).unwrap())
                .is_none()
        );
    }
}
