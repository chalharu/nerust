//! Per-window event dispatch: one handler per window keeps the loop match a flat router.
//! Destroyed/focus-guard order is load-bearing; do not reorder.

use tao::{event::WindowEvent, event_loop::ControlFlow};

use crate::ports::WindowOpener;

use super::WindowRuntime;
use crate::aux::focus::{AuxKind, FocusEffect, FocusEvent};

/// Map a native window event to a focus-domain event: only `Focused` maps.
fn focus_event_of(kind: Option<AuxKind>, event: &WindowEvent) -> Option<FocusEvent> {
    if let WindowEvent::Focused(focused) = event {
        Some(match kind {
            None => FocusEvent::Main(*focused),
            Some(kind) => FocusEvent::Aux(kind, *focused),
        })
    } else {
        None
    }
}

impl WindowRuntime {
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
                self.host.prepare_close();
                self.surface_xid = None;
                *control_flow = ControlFlow::Exit;
            }
            WindowEvent::Focused(true) => {
                // FocusEvent only; the coordinator decides whether to resume.
                if self.host.focus_event(FocusEvent::Main(true)) == Some(FocusEffect::Resume) {
                    self.host.resume_session();
                }
                self.host.request_redraw();
                // Keep the debugger toolbar truthful without
                // re-hitting the emu thread when nothing moved.
                let ctx = self.host.frame_ctx();
                self.host.sync_debug_chrome(&ctx);
            }
            WindowEvent::Focused(false) => {
                // Deferred: the pause lands at MainEventsCleared only if focus truly left.
                let _ = self.host.focus_event(FocusEvent::Main(false));
                self.host.clear_keys();
                // Same agreement in the other direction, still
                // without redundant round-trips.
                let ctx = self.host.frame_ctx();
                self.host.sync_debug_chrome(&ctx);
            }
            WindowEvent::Resized(_) => {
                self.host.sync_fullscreen_default_from_window();
                self.host.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.host.on_keyboard_input(event);
                self.pump_debug_open_request(opener);
            }
            _ => (),
        }
    }

    /// Keyboard/F4 open wiring: flag-to-window path stays headless-testable.
    fn pump_debug_open_request(&mut self, opener: &dyn WindowOpener) {
        if self.host.take_debug_open_request() {
            self.host.open_debug_windows(opener, false);
        }
    }

    /// Route one aux event by kind. The registry pumps; the host applies the outcome.
    pub(super) fn on_aux_event(
        &mut self,
        kind: AuxKind,
        event: WindowEvent,
        opener: &dyn WindowOpener,
    ) {
        match kind {
            AuxKind::Settings => self.on_settings_event(event),
            AuxKind::Debug => self.on_debug_event(opener, event),
            AuxKind::Ppu => self.on_ppu_event(event),
            AuxKind::History => self.host.on_history_event(event),
        }
    }

    pub(super) fn on_settings_event(&mut self, event: WindowEvent) {
        // External destroy: run the normal close path (pending apply still commits).
        if matches!(event, WindowEvent::Destroyed) {
            if self.host.settings_open() && self.host.close_settings_and_maybe_rebuild() {
                self.recreate_renderer();
            }
            return;
        }
        // Same-app focus moves cancel a pending pause; leaving anywhere arms it.
        let _ = self.drive_focus(AuxKind::Settings, &event);
        let domain = crate::aux::native::adapt_event(&event);
        let Some(outcome) = self.host.aux_on_event(AuxKind::Settings, domain) else {
            return;
        };
        // Coalesced present: the pump above already requested a redraw.
        if outcome.close && self.host.close_settings_and_maybe_rebuild() {
            self.recreate_renderer();
        }
    }

    /// Focus-only drive shared by every aux path: maps `Focused` to
    /// the domain event, resumes on `Resume`, and reports arrival.
    fn drive_focus(&mut self, kind: AuxKind, event: &WindowEvent) -> bool {
        let mut resume = false;
        if let Some(focus) = focus_event_of(Some(kind), event)
            && let Some(effect) = self.host.focus_event(focus)
        {
            resume = effect == FocusEffect::Resume;
        }
        if resume {
            self.host.resume_session();
        }
        matches!(event, WindowEvent::Focused(true))
    }

    pub(super) fn on_debug_event(&mut self, opener: &dyn WindowOpener, event: WindowEvent) {
        if let WindowEvent::Moved(position) = &event {
            self.host.remember_debug_window_pos(position.x, position.y);
        }
        // Focus re-reads, never force-pauses; external destroy drops the handle.
        if matches!(event, WindowEvent::Destroyed) {
            self.host.drop_debug_window();
            self.host.request_redraw();
            return;
        }
        let focused = self.drive_focus(AuxKind::Debug, &event);
        let domain = crate::aux::native::adapt_event(&event);
        let Some(outcome) = self.host.aux_on_event(AuxKind::Debug, domain) else {
            return;
        };
        if focused {
            let ctx = self.host.frame_ctx();
            self.host.sync_debug_chrome(&ctx);
        }
        self.drain_and_maybe_open_ppu(opener);
        // Closing never resumes: resume is always explicit.
        if self.host.is_debug_open() {
            // Coalesced redraw: the event/drain paths above already
            // flagged what changed; one request covers the frame.
            self.host.request_redraw();
        }
        if outcome.close {
            self.host.drop_debug_window();
            self.host.request_redraw();
        }
    }

    pub(super) fn on_debug_redrawn(&mut self, opener: &dyn WindowOpener) {
        self.drain_and_maybe_open_ppu(opener);
        self.host.render_aux(AuxKind::Debug);
    }

    pub(super) fn on_ppu_event(&mut self, event: WindowEvent) {
        // Same external-destroy guard as the debugger: a dead
        // handle must not linger and block reopening.
        if matches!(event, WindowEvent::Destroyed) {
            self.host.drop_ppu_window();
            self.host.request_redraw();
            return;
        }
        let _ = self.drive_focus(AuxKind::Ppu, &event);
        if let WindowEvent::Moved(position) = &event {
            self.host.remember_ppu_window_pos(position.x, position.y);
        }
        let domain = crate::aux::native::adapt_event(&event);
        let Some(outcome) = self.host.aux_on_event(AuxKind::Ppu, domain) else {
            return;
        };
        if outcome.close {
            self.host.drop_ppu_window();
            self.host.request_redraw();
        }
    }

    /// Debug drain plus deferred PPU-viewer open, shared by the
    /// debugger event and redraw paths.
    fn drain_and_maybe_open_ppu(&mut self, opener: &dyn WindowOpener) {
        self.host.drain_debug_requests();
        if self.host.take_ppu_open_request() {
            self.host.open_debug_windows(opener, true);
            self.host.run_debug_drain(&[]);
        }
    }
}

#[cfg(test)]
mod tests {
    use tao::{dpi::PhysicalPosition, event::WindowEvent, event_loop::ControlFlow};

    use super::WindowRuntime;
    use crate::aux::focus::AuxKind;
    use crate::runtime::host::TestHost;
    use crate::test_opener::{FakeOpener, OpenerCall};

    fn test_runtime() -> (WindowRuntime, crate::test_menu::MenuRecorderHandle) {
        let support = TestHost::new();
        let menu = support.menu.clone();
        let runtime = WindowRuntime {
            event_loop: None,
            host: support.host,
            renderer: None,
            surface_xid: None,
        };
        (runtime, menu)
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
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let mut flow = ControlFlow::Wait;
        runtime.surface_xid = Some(42);
        runtime.on_main_event(&opener, WindowEvent::Destroyed, &mut flow);
        assert_eq!(flow, ControlFlow::Exit);
        assert!(runtime.host.close_exited());
        assert!(runtime.surface_xid.is_none());
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn double_delivery_runs_close_once_and_exits() {
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let mut flow = ControlFlow::Wait;
        runtime.on_main_event(&opener, WindowEvent::CloseRequested, &mut flow);
        assert_eq!(flow, ControlFlow::Exit);
        assert!(runtime.host.close_exited());
        // Destroyed after CloseRequested still exits; the guarded
        // teardown ran exactly once (see the host-level idempotence test).
        runtime.on_main_event(&opener, WindowEvent::Destroyed, &mut flow);
        assert_eq!(flow, ControlFlow::Exit);
        assert!(runtime.host.close_exited());
    }

    #[test]
    fn main_focus_arms_syncs_chrome() {
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        let mut flow = ControlFlow::Wait;
        runtime.on_main_event(&opener, WindowEvent::Focused(false), &mut flow);
        assert!(!runtime.host.main_active());
        runtime.on_main_event(&opener, WindowEvent::Focused(true), &mut flow);
        assert!(runtime.host.main_active());
        // dirty gate: departure pushes once, arrival with identical
        // chrome skips the duplicate IPC.
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
    fn debug_open_request_reaches_opener_once() {
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.host.request_debug_open();
        runtime.pump_debug_open_request(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
        // Consumed: a second pump without re-arm stays quiet.
        runtime.pump_debug_open_request(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
    }

    #[test]
    fn settings_focus_never_moves_main_activity() {
        // Settings arrivals cancel without resuming and never touch
        // the main-active flag; departures only arm the deferred pause.
        let (mut runtime, menu) = test_runtime();
        runtime.on_settings_event(WindowEvent::Focused(true));
        runtime.on_settings_event(WindowEvent::Focused(false));
        assert!(runtime.host.main_active());
        assert!(menu.updates().is_empty());
        let _ = runtime
            .host
            .focus_event(crate::aux::focus::FocusEvent::Aux(AuxKind::Settings, true));
    }

    #[test]
    fn settings_destroyed_without_window_is_noop() {
        let (mut runtime, menu) = test_runtime();
        runtime.on_settings_event(WindowEvent::Destroyed);
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn debug_moved_remembers_position() {
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.on_debug_event(&opener, WindowEvent::Moved(PhysicalPosition::new(60, 20)));
        assert_eq!(runtime.host.debug_window_pos(), Some((60, 20)));
    }

    #[test]
    fn debug_destroyed_without_window_is_noop() {
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.on_debug_event(&opener, WindowEvent::Destroyed);
        assert!(menu.updates().is_empty());
        assert!(opener.calls().is_empty());
    }

    #[test]
    fn debug_focused_without_window_returns_early() {
        // No handle open: focus agreement runs, then the handler
        // returns before chrome sync or drain.
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.host.note_main_departure();
        runtime.on_debug_event(&opener, WindowEvent::Focused(true));
        assert!(menu.updates().is_empty());
        assert!(opener.calls().is_empty());
    }

    #[test]
    fn ppu_moved_remembers_position() {
        let (mut runtime, _) = test_runtime();
        runtime.on_ppu_event(WindowEvent::Moved(PhysicalPosition::new(1250, 100)));
        assert_eq!(runtime.host.ppu_window_pos(), Some((1250, 100)));
    }

    #[test]
    fn ppu_arrival_after_departure_drives_focus() {
        // Departure arms the pause; aux arrival cancels it through
        // the shared drive, even with no handle open.
        let (mut runtime, menu) = test_runtime();
        runtime.host.note_main_departure();
        runtime.on_ppu_event(WindowEvent::Focused(true));
        runtime.on_ppu_event(WindowEvent::Focused(false));
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn ppu_request_opens_debugger_once() {
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.host.request_ppu_open();
        runtime.on_debug_redrawn(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
        runtime.on_debug_redrawn(&opener);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
    }

    #[test]
    fn ppu_destroyed_without_window_is_noop() {
        let (mut runtime, menu) = test_runtime();
        runtime.on_ppu_event(WindowEvent::Destroyed);
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn debug_redrawn_drains_quietly() {
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.on_debug_redrawn(&opener);
        assert!(menu.updates().is_empty());
        assert!(opener.calls().is_empty());
    }

    #[test]
    fn debug_open_with_focus_ppu_skips_viewer_without_debugger() {
        // Debugger creation fails headless, so no viewer follows:
        // the PPU arm needs a live debugger handle.
        let (mut runtime, _) = test_runtime();
        let opener = FakeOpener::failing("no display");
        runtime.host.open_debug_windows(&opener, true);
        assert_eq!(opener.calls(), [OpenerCall::Debug]);
        assert!(!runtime.host.is_debug_open());
        assert!(!runtime.host.is_ppu_open());
    }

    #[test]
    fn focus_event_of_maps_only_focused() {
        use super::focus_event_of;
        use crate::aux::focus::FocusEvent;
        assert_eq!(
            focus_event_of(None, &WindowEvent::Focused(true)),
            Some(FocusEvent::Main(true))
        );
        assert_eq!(
            focus_event_of(Some(AuxKind::Ppu), &WindowEvent::Focused(false)),
            Some(FocusEvent::Aux(AuxKind::Ppu, false))
        );
        // Moved/Resized never arm or cancel the deferred pause.
        assert_eq!(
            focus_event_of(None, &WindowEvent::Moved(PhysicalPosition::new(1, 2))),
            None
        );
    }

    #[test]
    fn drive_focus_reports_arrival_only() {
        let (mut runtime, menu) = test_runtime();
        // Arrival without a protocol pause reports true, no resume.
        assert!(runtime.drive_focus(AuxKind::Debug, &WindowEvent::Focused(true)));
        // Departure and non-focus events report false.
        assert!(!runtime.drive_focus(AuxKind::Debug, &WindowEvent::Focused(false)));
        assert!(!runtime.drive_focus(
            AuxKind::Debug,
            &WindowEvent::Moved(PhysicalPosition::new(1, 2))
        ));
        assert!(!runtime.drive_focus(
            AuxKind::Settings,
            &WindowEvent::Resized(tao::dpi::PhysicalSize::new(800, 600))
        ));
        assert!(menu.updates().is_empty());
    }

    #[test]
    fn aux_routing_without_handles_stays_total() {
        let (mut runtime, menu) = test_runtime();
        let opener = FakeOpener::failing("no display");
        // Every aux arm resolves to no handle: pump, outcome, and
        // redraw all no-op without touching the menu.
        runtime.on_aux_event(AuxKind::Settings, WindowEvent::Focused(true), &opener);
        runtime.on_aux_event(AuxKind::Debug, WindowEvent::Focused(false), &opener);
        runtime.on_aux_event(AuxKind::Ppu, WindowEvent::Focused(false), &opener);
        runtime.on_aux_event(AuxKind::History, WindowEvent::Focused(true), &opener);
        runtime.on_settings_event(WindowEvent::Resized(tao::dpi::PhysicalSize::new(1, 1)));
        runtime.on_ppu_event(WindowEvent::Resized(tao::dpi::PhysicalSize::new(1, 1)));
        assert!(menu.updates().is_empty());
        assert!(opener.calls().is_empty());
    }

    #[test]
    fn settings_moved_never_arms_pause() {
        let (mut runtime, menu) = test_runtime();
        runtime.host.note_main_departure();
        runtime.on_settings_event(WindowEvent::Moved(PhysicalPosition::new(9, 9)));
        // Still armed: a non-focus event cancels nothing, so burst
        // end still commits the departure pause (which then no-ops).
        let mut flow = ControlFlow::Wait;
        runtime.host.update_control_flow(&mut flow);
        assert_eq!(flow, ControlFlow::Wait);
        assert!(menu.updates().is_empty());
    }
}
