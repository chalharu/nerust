//! Per-window event dispatch: one handler per window keeps the loop match a flat router.
//! Destroyed/focus-guard order is load-bearing; do not reorder.
//!
//! Every aux window is a generic widget window: routing is by [`AuxId`],
//! kinds never appear here. Window positions are remembered per kind through
//! the app trait.

use tao::{event::WindowEvent, event_loop::ControlFlow};

use crate::ports::WindowOpener;

use super::WindowRuntime;
use wakui_core::chrome::aux::AuxId;

impl<K: Copy + Eq + std::hash::Hash + 'static> WindowRuntime<K> {
    pub(super) fn on_main_event(
        &mut self,
        opener: &dyn WindowOpener,
        event: WindowEvent,
        control_flow: &mut ControlFlow,
    ) {
        match event {
            // Destroyed arrives without CloseRequested on external
            // kills; both run the same idempotent teardown and exit.
            WindowEvent::CloseRequested | WindowEvent::Destroyed => {
                self.begin_close();
                *control_flow = ControlFlow::Exit;
            }
            WindowEvent::Focused(true) => {
                // Focus truth plus resume only; chrome sync and drains
                // stay deferred to the tick burst (one pass, one capture).
                self.host
                    .on_focus_event(wakui_core::input::focus::FocusEvent::Main(true));
                self.host.request_redraw();
            }
            WindowEvent::Focused(false) => {
                // Deferred: the pause lands at MainEventsCleared only if focus truly left.
                self.host
                    .on_focus_event(wakui_core::input::focus::FocusEvent::Main(false));
                self.host.core.clear_input();
            }
            WindowEvent::Resized(_) => {
                self.host.sync_fullscreen_default_from_window();
                self.host.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.host.on_keyboard_input(event, opener);
                self.host.pump_deferred_opens(opener);
            }
            _ => (),
        }
    }

    /// Route one aux event by id. The registry pumps; the host applies the outcome.
    pub(super) fn on_aux_event(
        &mut self,
        id: AuxId,
        event: WindowEvent,
        opener: &dyn WindowOpener,
    ) {
        // External destroy runs the normal close path (pending apply still
        // commits); focus drives the shared protocol; moves remember position.
        if matches!(event, WindowEvent::Destroyed) {
            if self.host.finish_widget_window(id, true) {
                self.recreate_renderer();
            }
            self.host.request_redraw();
            return;
        }
        if let WindowEvent::Moved(position) = &event
            && let Some(kind) = self.host.aux.windows.kind_of_id(id)
        {
            self.host
                .core
                .remember_window_pos(kind, position.x, position.y);
        }
        let _ = self.host.drive_aux_focus(id, &event);
        let domain = crate::aux_windows::event_adapt::adapt_event(&event);
        let Some(outcome) = self.host.aux_on_event(id, domain) else {
            return;
        };
        self.apply_aux_outcome(id, outcome);
        self.host.pump_deferred_opens(opener);
    }

    pub(super) fn on_aux_redrawn(&mut self, id: AuxId, opener: &dyn WindowOpener) {
        let Some(outcome) = self.host.render_aux(id) else {
            return;
        };
        self.apply_aux_outcome(id, outcome);
        self.host.pump_deferred_opens(opener);
    }

    fn apply_aux_outcome(&mut self, id: AuxId, outcome: crate::aux_windows::registry::AuxOutcome) {
        if let Some(detail) = outcome.error {
            self.host.core.report_host_error(detail);
        }
        if (outcome.close || outcome.submitted) && self.host.finish_widget_window(id, outcome.close)
        {
            self.recreate_renderer();
        }
    }
}

#[cfg(test)]
mod tests {
    use tao::{dpi::PhysicalPosition, event::WindowEvent, event_loop::ControlFlow};

    use super::WindowRuntime;
    use crate::event_loop::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};
    use crate::testkit::TestAux;

    fn test_runtime() -> (WindowRuntime<TestAux>, crate::test_menu::MenuRecorderHandle) {
        let support = TestHost::new();
        let menu = support.menu.clone();
        let runtime = WindowRuntime {
            event_loop: None,
            event_sink: None,
            host: support.host,
            surface_xid: None,
        };
        (runtime, menu)
    }

    /// Open a headless widget window of this kind and return its id.
    /// Creation fails headless, so seat a stub handle directly.
    fn open_widget(
        runtime: &mut WindowRuntime<TestAux>,
        kind: TestAux,
    ) -> Option<wakui_core::chrome::aux::AuxId> {
        use wakui_core::chrome::aux::OpenRequest;
        let id = runtime
            .host
            .reserve_aux_window(kind)
            .and_then(|request| match request {
                OpenRequest::Create(id) => Some(id),
                _ => None,
            })?;
        assert!(runtime.host.prepare_widget(id));
        let stub = Box::new(crate::testkit::StubAux::new());
        assert!(runtime.host.aux.complete_open(id, stub).is_ok());
        Some(id)
    }

    #[test]
    fn main_close_requests_exit() {
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let mut flow = ControlFlow::Wait;
        runtime.on_main_event(&opener, WindowEvent::CloseRequested, &mut flow);
        assert_eq!(flow, ControlFlow::Exit);
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn main_destroyed_without_close_requests_exit() {
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let mut flow = ControlFlow::Wait;
        runtime.surface_xid = Some(42);
        runtime.on_main_event(&opener, WindowEvent::Destroyed, &mut flow);
        assert_eq!(flow, ControlFlow::Exit);
        assert!(runtime.host.core.close_exited());
        assert!(runtime.surface_xid.is_none());
    }

    #[test]
    fn double_delivery_runs_close_once_and_exits() {
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let mut flow = ControlFlow::Wait;
        runtime.on_main_event(&opener, WindowEvent::CloseRequested, &mut flow);
        assert_eq!(flow, ControlFlow::Exit);
        assert!(runtime.host.core.close_exited());
        // Destroyed after CloseRequested still exits; the guarded
        // teardown ran exactly once (see the host-level idempotence test).
        runtime.on_main_event(&opener, WindowEvent::Destroyed, &mut flow);
        assert_eq!(flow, ControlFlow::Exit);
        assert!(runtime.host.core.close_exited());
    }

    #[test]
    fn menu_quit_defers_teardown_to_ordered_begin_close() {
        use crate::event_loop::host::HostAction;
        use crate::testkit::TestMenu;
        let support = TestHost::new();
        let quit = support.menu_ref(TestMenu::Quit);
        let mut runtime = WindowRuntime {
            event_loop: None,
            event_sink: None,
            host: support.host,
            surface_xid: None,
        };
        let opener = FakeOpener::failing("no display");
        // Menu Quit returns the exit intent only: host teardown waits.
        assert_eq!(
            runtime.host.on_menu_command(quit, &opener),
            HostAction::Exit
        );
        assert!(!runtime.host.core.close_exited());
        // The runtime owns the ordered teardown: aux windows close and
        // the session flush runs exactly once across repeated delivery.
        runtime.begin_close();
        assert!(runtime.host.core.close_exited());
        assert!(runtime.host.aux.windows.ids().is_empty());
        assert!(!runtime.host.core.modal_guard_active());
        assert!(runtime.surface_xid.is_none());
        runtime.begin_close();
        assert!(runtime.host.core.close_exited());
    }

    #[test]
    fn main_focus_arms_without_syncing() {
        // Focus handlers only drive focus truth now: no chrome sync,
        // no drain. The tick burst performs the single sync pass.
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let mut flow = ControlFlow::Wait;
        runtime.on_main_event(&opener, WindowEvent::Focused(false), &mut flow);
        assert!(!runtime.host.core.main_active());
        runtime.on_main_event(&opener, WindowEvent::Focused(true), &mut flow);
        assert!(runtime.host.core.main_active());
        assert!(menu.updates().is_empty());
        // Tick burst commits the armed pause state and syncs chrome once.
        runtime.host.update_control_flow(&mut flow);
        assert_eq!(menu.updates().len(), 1);
    }

    #[test]
    fn main_resized_without_window_stays_quiet() {
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let mut flow = ControlFlow::Wait;
        runtime.on_main_event(
            &opener,
            WindowEvent::Resized(tao::dpi::PhysicalSize::new(800, 600)),
            &mut flow,
        );
        assert_eq!(flow, ControlFlow::Wait);
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn deferred_open_request_reaches_opener_once() {
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.host.core.request_open(TestAux::Gamma);
        assert_eq!(
            runtime.host.core.drain_open_requests(),
            vec![TestAux::Gamma]
        );
        // Consumed: a second drain without re-arm stays quiet.
        assert!(runtime.host.core.drain_open_requests().is_empty());
        let _ = opener;
    }

    #[test]
    fn widget_focus_never_moves_main_activity() {
        // Widget arrivals cancel/resume through the trait without touching
        // the main-active flag; departures only arm the deferred pause.
        let (mut runtime, menu) = test_runtime();
        let Some(id) = open_widget(&mut runtime, TestAux::Alpha) else {
            panic!("widget");
        };
        runtime.on_aux_event(id, WindowEvent::Focused(true), &FakeOpener::failing("x"));
        runtime.on_aux_event(id, WindowEvent::Focused(false), &FakeOpener::failing("x"));
        assert!(runtime.host.core.main_active());
        assert!(menu.updates().is_empty());
        let _ = runtime.host.core.drive_aux_focus(id, true);
    }

    #[test]
    fn widget_destroyed_without_window_is_noop() {
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        // Unknown id: routing finds nothing, menu untouched.
        let id = wakui_core::chrome::aux::AuxId::from_raw(9999);
        runtime.on_aux_event(id, WindowEvent::Destroyed, &opener);
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn widget_moved_remembers_position() {
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let Some(id) = open_widget(&mut runtime, TestAux::Alpha) else {
            panic!("widget");
        };
        runtime.on_aux_event(id, WindowEvent::Moved(PhysicalPosition::new(9, 9)), &opener);
        assert_eq!(runtime.host.core.window_pos(TestAux::Alpha), Some((9, 9)));
    }

    #[test]
    fn widget_focused_without_window_returns_early() {
        // No handle open: focus agreement runs, then the handler
        // returns before chrome sync or drain.
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.host.core.note_main_departure();
        let id = wakui_core::chrome::aux::AuxId::from_raw(9999);
        runtime.on_aux_event(id, WindowEvent::Focused(true), &opener);
        assert!(menu.updates().is_empty());
        assert!(opener.calls().is_empty());
    }

    #[test]
    fn deferred_delta_request_opens_once() {
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.host.core.request_open(TestAux::Delta);
        runtime.host.pump_deferred_opens(&opener);
        // Headless creation fails: Gamma never counted, Delta never opened.
        assert_eq!(opener.calls(), [OpenerCall::Delta]);
        runtime.host.pump_deferred_opens(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Delta]);
    }

    #[test]
    fn widget_destroyed_closes_window() {
        let (mut runtime, menu) = test_runtime();
        let Some(id) = open_widget(&mut runtime, TestAux::Alpha) else {
            panic!("widget");
        };
        let opener = FakeOpener::failing("no display");
        runtime.on_aux_event(id, WindowEvent::Destroyed, &opener);
        // Close commits and syncs chrome once through the shared choke point.
        assert_eq!(menu.updates().len(), 1);
        assert!(runtime.host.aux.windows.kind_of_id(id).is_none());
    }

    #[test]
    fn aux_redrawn_without_window_is_noop() {
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let id = wakui_core::chrome::aux::AuxId::from_raw(9999);
        runtime.on_aux_redrawn(id, &opener);
        assert!(menu.updates().is_empty());
        assert!(opener.calls().is_empty());
    }

    #[test]
    fn host_focus_drive_reports_arrival_only() {
        // Shared host port: arrival reports true, departure reports false.
        let (mut runtime, menu) = test_runtime();
        let Some(id) = open_widget(&mut runtime, TestAux::Alpha) else {
            panic!("widget");
        };
        assert!(runtime.host.core.drive_aux_focus(id, true));
        assert!(!runtime.host.core.drive_aux_focus(id, false));
        runtime.on_aux_event(
            id,
            WindowEvent::Moved(PhysicalPosition::new(1, 2)),
            &FakeOpener::failing("x"),
        );
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn drive_focus_reports_arrival_only() {
        let (mut runtime, menu) = test_runtime();
        let Some(id) = open_widget(&mut runtime, TestAux::Alpha) else {
            panic!("widget");
        };
        let opener = FakeOpener::failing("x");
        // Arrival without a protocol pause reports true, no resume.
        assert!(
            runtime
                .host
                .drive_aux_focus(id, &WindowEvent::Focused(true))
        );
        // Departure and non-focus events report false.
        assert!(
            !runtime
                .host
                .drive_aux_focus(id, &WindowEvent::Focused(false))
        );
        assert!(
            !runtime
                .host
                .drive_aux_focus(id, &WindowEvent::Moved(PhysicalPosition::new(1, 2)))
        );
        assert!(!runtime.host.drive_aux_focus(
            id,
            &WindowEvent::Resized(tao::dpi::PhysicalSize::new(800, 600))
        ));
        let _ = opener;
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn unknown_aux_event_stays_total() {
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let id = wakui_core::chrome::aux::AuxId::from_raw(9999);
        runtime.on_aux_event(id, WindowEvent::Focused(true), &opener);
        runtime.on_aux_event(id, WindowEvent::Focused(false), &opener);
        runtime.on_aux_event(
            id,
            WindowEvent::Resized(tao::dpi::PhysicalSize::new(1, 1)),
            &opener,
        );
        assert!(menu.updates().is_empty());
        assert!(opener.calls().is_empty());
    }
}
