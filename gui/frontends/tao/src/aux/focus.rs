//! Aux-window focus agreement as behavior-carrying types. Leafward
//! home for the deferred auto-pause protocol: aux handles implement
//! [`WindowHandler`], runtime only drives it. No runtime imports.

use iced_tiny_skia::Renderer;
use iced_winit::program::Program;
use tao::event::WindowEvent;

use super::shell::{AuxShell, AuxWindow};

/// Main-window focus truth plus the deferred auto-pause protocol.
/// Invariants: a pause lands only via the burst-end commit after
/// arming (`leave_main`/`leave_aux`) without an intervening cancel
/// (`cancel_pending`/`arrive_*`); arrival reports a resume at most once
/// per armed pause (the flag is consumed). Explicit user pauses never
/// set the flag.
pub(crate) struct FocusState {
    /// Main window holds focus.
    main_active: bool,
    /// Session paused by the protocol, never by the user directly.
    auto_paused: bool,
    /// Blur armed a pause; committed at burst end unless cancelled.
    pending: bool,
}

impl FocusState {
    pub(crate) fn new() -> Self {
        Self {
            main_active: true,
            auto_paused: false,
            pending: false,
        }
    }

    pub(crate) fn main_active(&self) -> bool {
        self.main_active
    }

    /// Main-window arrival: activate, cancel any pending pause, and
    /// report whether a system-paused session must resume.
    pub(crate) fn arrive_main(&mut self) -> bool {
        self.main_active = true;
        self.arrive()
    }

    /// Main-window loss: deactivate and arm the deferred pause.
    pub(crate) fn leave_main(&mut self) {
        self.main_active = false;
        self.pending = true;
    }

    /// Aux-window arrival (same-app move, not away): cancel any
    /// pending pause and report whether to resume.
    pub(crate) fn arrive_aux(&mut self) -> bool {
        self.arrive()
    }

    /// Aux-window loss: arm the deferred pause.
    pub(crate) fn leave_aux(&mut self) {
        self.pending = true;
    }

    /// Cancel a pending pause without further effect (e.g. settings
    /// arrival, which never resumes).
    pub(crate) fn cancel_pending(&mut self) {
        self.pending = false;
    }

    /// Burst-end commit point: take the armed flag. False when
    /// cancelled meanwhile or never armed.
    pub(crate) fn take_armed(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// Record a protocol-driven pause (user pauses never set this).
    pub(crate) fn mark_auto_paused(&mut self) {
        self.auto_paused = true;
    }

    /// Shared arrival core: cancel pending, consume the resume flag.
    fn arrive(&mut self) -> bool {
        self.pending = false;
        std::mem::replace(&mut self.auto_paused, false)
    }
}

/// Shared aux-focus step: same-app arrival cancels a pending pause
/// (reporting resume); leaving anywhere arms it. Single source below.
pub(crate) fn aux_focus_step(focus: &mut FocusState, focused: bool) -> bool {
    if focused {
        focus.arrive_aux()
    } else {
        focus.leave_aux();
        false
    }
}

/// Quiet aux-focus step for windows that must never resume: arrival
/// only cancels a pending pause (the auto-pause flag stays intact),
/// leaving arms the deferred pause. Settings uses this; debug/PPU/
/// history use [`aux_focus_step`], which may report a resume.
pub(crate) fn aux_focus_step_quiet(focus: &mut FocusState, focused: bool) {
    if focused {
        // Cancel only: unlike `arrive_aux`, the auto-pause flag is
        // preserved so a later main-window arrival still resumes.
        focus.cancel_pending();
    } else {
        focus.leave_aux();
    }
}

/// Uniform aux-window handler: event feed plus shell access.
/// Handles keep only their program type, log tag, shell access, and
/// program-specific [`WindowHandler::handle_event`]; identity, close,
/// focus, render, and the pump stay default so the four twins cannot
/// drift apart. Focus agreement stays [`aux_focus_step`]/quiet outside
/// the trait: no handle state participates.
pub(crate) trait WindowHandler: AuxWindow {
    type Program: Program<Theme = iced::Theme, Renderer = Renderer>;
    /// Log tag for the shared render path (per-window identity).
    const LOG_TAG: &'static str;
    fn shell(&self) -> &AuxShell<Self::Program>;
    fn shell_mut(&mut self) -> &mut AuxShell<Self::Program>;
    /// Handle one converted event. Returns whether this pass already
    /// requested a redraw, so the pump can skip its trailing request.
    fn handle_event(&mut self, mapped: iced::Event) -> bool;

    /// Feed one native event through the shared prelude.
    fn handle_tao_event(&mut self, event: WindowEvent) -> bool {
        if let Some(mapped) = self.shell_mut().feed_tao_event(event) {
            self.handle_event(mapped)
        } else {
            false
        }
    }

    /// Feed one native event to the aux shell.
    fn on_event(&mut self, event: WindowEvent) {
        AuxWindow::pump_window_event(self, event);
    }

    /// Native window identity for event routing.
    fn tao_window_id(&self) -> tao::window::WindowId {
        self.shell().tao_window_id()
    }

    /// Close was requested through the shared flag.
    fn close_requested(&self) -> bool {
        self.shell().close_requested()
    }

    /// Bring this viewer to the front.
    fn focus_window(&self) {
        self.shell().focus_window();
    }

    /// Full render through the shared pump; only the log tag differs.
    fn render(&mut self) {
        let tag = Self::LOG_TAG;
        self.shell_mut().render(tag);
    }
}

impl<T: WindowHandler> AuxWindow for T {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell_mut().handle_common_event(&event);
        // Resize/scale/modifier paths never rebuild, so the trailing
        // request stays their only repaint; rebuild/repaint passes
        // already requested above and skip the duplicate.
        if !self.handle_tao_event(event) {
            self.shell().request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unarmed_commit_reports_false() {
        let mut focus = FocusState::new();
        assert!(!focus.take_armed());
        assert!(!focus.arrive_main());
        assert!(!focus.arrive_aux());
    }

    #[test]
    fn arm_commit_cycle_arms_once() {
        let mut focus = FocusState::new();
        focus.leave_aux();
        assert!(focus.take_armed());
        // Consumed: a second commit without re-arm is quiet.
        assert!(!focus.take_armed());
    }

    #[test]
    fn cancel_disarms_before_commit() {
        let mut focus = FocusState::new();
        focus.leave_aux();
        focus.cancel_pending();
        assert!(!focus.take_armed());
    }

    #[test]
    fn arrive_reports_resume_exactly_once() {
        let mut focus = FocusState::new();
        focus.leave_aux();
        assert!(focus.take_armed());
        focus.mark_auto_paused();
        assert!(focus.arrive_main());
        // Flag consumed: no double resume.
        assert!(!focus.arrive_main());
        assert!(!focus.arrive_aux());
    }

    #[test]
    fn main_leave_deactivates_and_arms() {
        let mut focus = FocusState::new();
        assert!(focus.main_active());
        focus.leave_main();
        assert!(!focus.main_active());
        assert!(focus.take_armed());
    }

    #[test]
    fn aux_arrival_cancels_pending_without_resume_flag() {
        let mut focus = FocusState::new();
        focus.leave_main();
        focus.arrive_aux();
        assert!(!focus.take_armed());
    }

    #[test]
    fn settings_arrival_preserves_resume_for_main_return() {
        // Settings arrival cancels without resuming; a prior
        // protocol pause still resumes on the next main arrival.
        let mut focus = FocusState::new();
        focus.leave_aux();
        assert!(focus.take_armed());
        focus.mark_auto_paused();
        focus.cancel_pending();
        assert!(focus.arrive_main());
    }
}
