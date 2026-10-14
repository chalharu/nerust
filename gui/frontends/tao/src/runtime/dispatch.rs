//! Per-window event dispatch: one handler per window keeps the
//! event-loop match in `super` a flat router. Handlers mirror the old
//! inline arms (debug/PPU focus agreement shares helpers); check the
//! arm history before reordering Destroyed/focus guards.

use tao::{
    event::WindowEvent,
    event_loop::{ControlFlow, EventLoopWindowTarget},
};

use crate::aux::shell::AuxWindow;
use crate::event::UserEvent;

use super::WindowRuntime;

impl WindowRuntime {
    pub(super) fn on_main_event(
        &mut self,
        event_loop: &EventLoopWindowTarget<UserEvent>,
        event: WindowEvent,
        control_flow: &mut ControlFlow,
    ) {
        match event {
            WindowEvent::CloseRequested if self.host.prepare_close() => {
                *control_flow = ControlFlow::Exit;
            }
            WindowEvent::Focused(true) => {
                // A same-burst aux focus already cancelled any
                // pending pause; this is belt-and-braces.
                if self.host.focus.arrive_main() {
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
                self.host.focus.leave_main();
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
                if self.host.take_debug_open_request() {
                    self.host.open_debug_windows(event_loop, false);
                }
            }
            _ => (),
        }
    }

    pub(super) fn on_settings_event(&mut self, event: WindowEvent) {
        // External destroy without CloseRequested: the window
        // is already gone, so run the normal close path (pending
        // apply still commits).
        if matches!(event, WindowEvent::Destroyed) {
            if let Some(handle) = self.host.settings_window.take() {
                let plan = self.host.close_settings_window(handle);
                if plan.is_some_and(|p| p.renderer_needs_rebuild) {
                    self.recreate_renderer();
                }
                self.host.request_redraw();
            }
            return;
        }
        // Same-app focus moves cancel a pending auto-pause;
        // leaving anywhere arms it (see the main arms). Runs
        // before borrowing the handle below.
        if matches!(event, WindowEvent::Focused(true)) {
            self.host.focus.cancel_pending();
        }
        if matches!(event, WindowEvent::Focused(false)) {
            self.host.focus.leave_aux();
        }
        let Some(handle) = self.host.settings_window.as_mut() else {
            return;
        };
        handle.pump_window_event(event);
        // Coalesced present: every visual path above already
        // requested a redraw; presenting here per event would
        // serialize a full draw behind every drag/motion event
        // and unload the backlog as a hang on release.
        if handle
            .shell
            .should_close
            .load(std::sync::atomic::Ordering::Acquire)
        {
            let Some(handle) = self.host.settings_window.take() else {
                return;
            };
            let plan = self.host.close_settings_window(handle);
            if plan.is_some_and(|p| p.renderer_needs_rebuild) {
                self.recreate_renderer();
            }
            self.host.request_redraw();
        }
    }

    pub(super) fn on_debug_event(
        &mut self,
        event_loop: &EventLoopWindowTarget<UserEvent>,
        event: WindowEvent,
    ) {
        if let WindowEvent::Moved(position) = &event {
            self.host
                .debug
                .remember_debug_window_pos(position.x, position.y);
        }
        // Focus re-reads, never force-pauses. External destroy
        // drops the handle so a later F4 reopens; checked before
        // focus handling so a dead window arms no auto-pause.
        if matches!(event, WindowEvent::Destroyed) {
            self.host.debug.drop_debug_window();
            self.host.request_redraw();
            return;
        }
        let refocused = matches!(event, WindowEvent::Focused(true));
        self.note_aux_focus(refocused);
        {
            let Some(handle) = self.host.debug.debug_window.as_mut() else {
                return;
            };
            handle.pump_window_event(event);
        }
        if refocused {
            self.host.sync_debug_chrome();
        }
        self.drain_and_maybe_open_ppu(event_loop);
        // Closing never resumes: resume is always explicit.
        if let Some(handle) = self.host.debug.debug_window.as_mut() {
            // Coalesced present (see the settings arm): every
            // visual path above already requested a redraw.
            handle.shell.window.request_redraw();
        }
        if self.host.debug.debug_close_requested() {
            self.host.debug.drop_debug_window();
            self.host.request_redraw();
        }
    }

    pub(super) fn on_debug_redrawn(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        self.drain_and_maybe_open_ppu(event_loop);
        if let Some(handle) = self.host.debug.debug_window.as_mut() {
            handle.render();
        }
    }

    pub(super) fn on_ppu_event(&mut self, event: WindowEvent) {
        // Same external-destroy guard as the debugger: a dead
        // handle must not linger and block reopening.
        if matches!(event, WindowEvent::Destroyed) {
            self.host.debug.drop_ppu_window();
            self.host.request_redraw();
            return;
        }
        // A system-paused session resumes like on main focus;
        // explicit pauses stay paused.
        self.note_aux_focus(matches!(event, WindowEvent::Focused(true)));
        if let WindowEvent::Moved(position) = &event {
            self.host
                .debug
                .remember_ppu_window_pos(position.x, position.y);
        }
        {
            let Some(handle) = self.host.debug.ppu_window.as_mut() else {
                return;
            };
            handle.pump_window_event(event);
        }
        if self.host.debug.ppu_close_requested() {
            self.host.debug.drop_ppu_window();
            self.host.request_redraw();
        }
    }

    /// Aux-window focus agreement, shared by debugger and PPU:
    /// same-app arrival cancels a pending auto-pause and resumes a
    /// system-paused session; leaving anywhere arms it.
    fn note_aux_focus(&mut self, focused: bool) {
        if focused {
            if self.host.focus.arrive_aux() {
                self.host.resume_session();
            }
        } else {
            self.host.focus.leave_aux();
        }
    }

    /// Debug drain plus deferred PPU-viewer open, shared by the
    /// debugger event and redraw paths.
    fn drain_and_maybe_open_ppu(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        self.host.drain_debug_requests();
        if self.host.take_ppu_open_request() {
            self.host.open_debug_windows(event_loop, true);
            self.host.run_debug_drain(&[]);
        }
    }
}
