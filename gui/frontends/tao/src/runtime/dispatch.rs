//! Per-window event dispatch: one handler per window keeps the
//! event-loop match in `super` a flat router. Handlers mirror the old
//! inline arms (debug/PPU focus agreement shares helpers); check the
//! arm history before reordering Destroyed/focus guards.

use tao::{event::WindowEvent, event_loop::ControlFlow};

use crate::ports::WindowOpener;

use super::WindowRuntime;
use crate::aux::focus::{AuxKind, WindowHandler};

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
                // A same-burst aux focus already cancelled any
                // pending pause; this is belt-and-braces.
                if self.host.note_main_arrival() {
                    self.host.resume_session();
                }
                self.host.request_redraw();
                // Keep the debugger toolbar truthful without
                // re-hitting the emu thread when nothing moved.
                self.host.sync_debug_chrome();
            }
            WindowEvent::Focused(false) => {
                // Deferred: an aux window may take focus in the same
                // burst, so the pause lands at MainEventsCleared only
                // if focus truly left us.
                self.host.note_main_departure();
                self.host.clear_keys();
                // Same agreement in the other direction, still
                // without redundant round-trips.
                self.host.sync_debug_chrome();
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

    /// Keyboard/F4 open wiring, extracted so the flag-to-window path
    /// stays headless-testable (`KeyEvent` itself is unbuildable
    /// outside tao, so the input arm above keeps no other logic).
    fn pump_debug_open_request(&mut self, opener: &dyn WindowOpener) {
        if self.host.take_debug_open_request() {
            self.host.open_debug_windows(opener, false);
        }
    }

    pub(super) fn on_settings_event(&mut self, event: WindowEvent) {
        // External destroy without CloseRequested: the window
        // is already gone, so run the normal close path (pending
        // apply still commits).
        if matches!(event, WindowEvent::Destroyed) {
            if let Some(handle) = self.host.take_settings_window()
                && self.host.close_settings_and_maybe_rebuild(handle)
            {
                self.recreate_renderer();
            }
            return;
        }
        // Same-app focus moves cancel a pending auto-pause;
        // leaving anywhere arms it (see the main arms). Runs
        // before borrowing the handle below.
        if let WindowEvent::Focused(focused) = event {
            self.host.on_aux_focus(AuxKind::Settings, focused);
        }
        let Some(handle) = self.host.settings_window_mut() else {
            return;
        };
        handle.on_event(event);
        // Coalesced present: every visual path above already
        // requested a redraw; presenting here per event would
        // serialize a full draw behind every drag/motion event
        // and unload the backlog as a hang on release.
        if handle.close_requested() {
            let Some(handle) = self.host.take_settings_window() else {
                return;
            };
            if self.host.close_settings_and_maybe_rebuild(handle) {
                self.recreate_renderer();
            }
        }
    }

    pub(super) fn on_debug_event(&mut self, opener: &dyn WindowOpener, event: WindowEvent) {
        if let WindowEvent::Moved(position) = &event {
            self.host.remember_debug_window_pos(position.x, position.y);
        }
        // Focus re-reads, never force-pauses. External destroy
        // drops the handle so a later F4 reopens; checked before
        // focus handling so a dead window arms no auto-pause.
        if matches!(event, WindowEvent::Destroyed) {
            self.host.drop_debug_window();
            self.host.request_redraw();
            return;
        }
        // Focus-only drive: Moved/Resized/key events must not arm
        // the deferred pause. Only Focused(true/false) touches it.
        let focused = matches!(event, WindowEvent::Focused(true));
        if matches!(event, WindowEvent::Focused(_))
            && self.host.on_aux_focus(AuxKind::Debug, focused)
        {
            self.host.resume_session();
        }
        {
            let Some(handle) = self.host.debug_window_mut() else {
                return;
            };
            handle.on_event(event);
        }
        if focused {
            self.host.sync_debug_chrome();
        }
        self.drain_and_maybe_open_ppu(opener);
        // Closing never resumes: resume is always explicit.
        if self.host.is_debug_open() {
            // Coalesced redraw: the event/drain paths above already
            // flagged what changed; one request covers the frame.
            self.host.request_redraw();
        }
        if self.host.debug_close_requested() {
            self.host.drop_debug_window();
            self.host.request_redraw();
        }
    }

    pub(super) fn on_debug_redrawn(&mut self, opener: &dyn WindowOpener) {
        self.drain_and_maybe_open_ppu(opener);
        if let Some(handle) = self.host.debug_window_mut() {
            handle.render();
        }
    }

    pub(super) fn on_ppu_event(&mut self, event: WindowEvent) {
        // Same external-destroy guard as the debugger: a dead
        // handle must not linger and block reopening.
        if matches!(event, WindowEvent::Destroyed) {
            self.host.drop_ppu_window();
            self.host.request_redraw();
            return;
        }
        // A system-paused session resumes like on main focus;
        // explicit pauses stay paused.
        // Focus-only drive: Moved/Resized/key events must not arm
        // the deferred pause. Only Focused(true/false) touches it.
        let focused = matches!(event, WindowEvent::Focused(true));
        if matches!(event, WindowEvent::Focused(_)) && self.host.on_aux_focus(AuxKind::Ppu, focused)
        {
            self.host.resume_session();
        }
        if let WindowEvent::Moved(position) = &event {
            self.host.remember_ppu_window_pos(position.x, position.y);
        }
        {
            let Some(handle) = self.host.ppu_window_mut() else {
                return;
            };
            handle.on_event(event);
        }
        if self.host.ppu_close_requested() {
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
        runtime.host.on_aux_focus(AuxKind::Settings, true);
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
    fn ppu_focused_without_window_returns_early() {
        let (mut runtime, menu) = test_runtime();
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
}
