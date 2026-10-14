//! Per-window event dispatch: one handler per window keeps the
//! event-loop match in `super` a flat router. Handlers mirror the old
//! inline arms (debug/PPU focus agreement shares helpers); check the
//! arm history before reordering Destroyed/focus guards.

use tao::{
    event::WindowEvent,
    event_loop::{ControlFlow, EventLoopWindowTarget},
};

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
                self.host.active = true;
                // A same-burst aux focus already cancelled any
                // pending pause; this is belt-and-braces.
                self.host.cancel_pending_auto_pause();
                if self.host.auto_paused() {
                    self.host.resume_session();
                    self.host.clear_auto_paused();
                }
                self.host.request_redraw();
                // Keep the debugger toolbar truthful without
                // re-hitting the emu thread when nothing moved.
                self.host.sync_debug_chrome();
            }
            WindowEvent::Focused(false) => {
                self.host.active = false;
                // Deferred, not immediate: an aux window may take
                // focus in the same burst (same-app switch keeps
                // running); the pause lands at MainEventsCleared
                // only if focus truly left us (`can_pause` is
                // re-checked there).
                self.host.arm_pending_auto_pause();
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
        // is already gone, so skip event/render and run the
        // normal close path (pending apply still commits).
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
            self.host.cancel_pending_auto_pause();
        }
        if matches!(event, WindowEvent::Focused(false)) {
            self.host.arm_pending_auto_pause();
        }
        let Some(handle) = self.host.settings_window.as_mut() else {
            return;
        };
        match &event {
            WindowEvent::Resized(size) => handle.resize(size.width, size.height),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                handle.set_scale_factor(*scale_factor as f32);
            }
            WindowEvent::ModifiersChanged(state) => handle.set_modifiers(*state),
            WindowEvent::CloseRequested => {
                handle
                    .shell
                    .should_close
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            _ => {}
        }
        handle.update_modifiers_from_tao_event(&event);
        handle.handle_tao_event(event);
        // Coalesced present: every visual path above already
        // requested a redraw; presenting here per event would
        // serialize a full draw behind every drag/motion event
        // and unload the backlog as a hang on release.
        handle.shell.window.request_redraw();
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
        // Focus re-reads, never force-pauses; the empty batch
        // still refreshes the toolbar label.
        // External destroy without CloseRequested: drop the
        // stale handle so a later F4 reopens.
        // External destroy without CloseRequested: drop the
        // stale handle so a later F4 reopens. Checked before focus
        // handling: a dead window arms no auto-pause (same order as
        // the PPU arm).
        if matches!(event, WindowEvent::Destroyed) {
            self.host.debug.debug_window.take();
            self.host.request_redraw();
            return;
        }
        let refocused = matches!(event, WindowEvent::Focused(true));
        self.note_aux_focus(refocused);
        {
            let Some(handle) = self.host.debug.debug_window.as_mut() else {
                return;
            };
            handle.shell.handle_common_event(&event);
            handle.handle_tao_event(event);
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
            if handle
                .shell
                .should_close
                .load(std::sync::atomic::Ordering::Acquire)
            {
                self.host.debug.debug_window.take();
                self.host.request_redraw();
            }
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
            self.host.debug.ppu_window.take();
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
        let Some(handle) = self.host.debug.ppu_window.as_mut() else {
            return;
        };
        handle.shell.handle_common_event(&event);
        handle.handle_tao_event(event);
        handle.shell.window.request_redraw();
        if handle
            .shell
            .should_close
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.host.debug.ppu_window.take();
            self.host.request_redraw();
        }
    }

    /// Aux-window focus agreement, shared by the debugger and PPU
    /// arms: same-app arrival cancels a pending auto-pause (the user
    /// moved between our windows, not away) and resumes a
    /// system-paused session; leaving anywhere arms the deferred
    /// pause. Explicit pauses carry no flag and stay paused.
    fn note_aux_focus(&mut self, focused: bool) {
        if focused {
            self.host.cancel_pending_auto_pause();
            if self.host.auto_paused() {
                self.host.resume_session();
                self.host.clear_auto_paused();
            }
        } else {
            self.host.arm_pending_auto_pause();
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
