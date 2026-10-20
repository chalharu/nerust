//! Aux-window focus agreement as behavior-carrying types. Leafward
//! home for the deferred auto-pause protocol: aux handles implement
//! [`AuxWindow`] + [`AuxEvents`], runtime only drives them. No runtime imports.

use tao::event::WindowEvent;

/// Which aux window changed focus. The deferred auto-pause protocol
/// decision lives here (not at the dispatch call sites): interactive
/// viewers (debugger, PPU) may report a resume, while read-only
/// viewers (settings, history) only cancel a pending pause and never
/// resume anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuxKind {
    Debug,
    Ppu,
    History,
    Settings,
}

impl AuxKind {
    /// Central log tag for every aux window. Single home so renames
    /// never drift across handle `LOG_TAG`s and scaffold strings.
    pub(crate) fn log_tag(self) -> &'static str {
        match self {
            AuxKind::Debug => "debugger",
            AuxKind::Ppu => "PPU viewer",
            AuxKind::History => "history window",
            AuxKind::Settings => "settings",
        }
    }
}

/// Ad-hoc memo gate: remembers the last pushed value and reports
/// whether a new value differs. Replaces the six inline
/// `last_* != new` gates with one named type.
#[derive(Debug, Default)]
pub(crate) struct Memo<T: PartialEq> {
    last: Option<T>,
}

impl<T: PartialEq + Clone> Memo<T> {
    pub(crate) fn new() -> Self {
        Self { last: None }
    }

    /// Returns true when `value` differs from the remembered one (or
    /// nothing was remembered), updating the memory. False means skip.
    pub(crate) fn check(&mut self, value: &T) -> bool {
        if self.last.as_ref() == Some(value) {
            return false;
        }
        self.last = Some(value.clone());
        true
    }

    pub(crate) fn clear(&mut self) {
        self.last = None;
    }

    pub(crate) fn get(&self) -> Option<&T> {
        self.last.as_ref()
    }
}

/// Main-window focus truth plus the deferred auto-pause protocol.
/// Invariants: a pause lands only via the burst-end commit after
/// arming (`leave_main`/`leave_aux`) without an intervening cancel;
/// arrival reports a resume at most once per armed pause. Explicit
/// user pauses never set the flag.
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

    /// Aux-window focus change with the resume protocol chosen by
    /// kind. Public narrow entry: interactive viewers may report a
    /// resume, read-only viewers only cancel a pending pause. Returns
    /// whether a system-paused session should resume.
    pub(crate) fn on_aux(&mut self, kind: AuxKind, focused: bool) -> bool {
        on_aux_focus(self, kind, focused)
    }

    fn arrive_main(&mut self) -> bool {
        self.main_active = true;
        self.arrive()
    }

    fn leave_main(&mut self) {
        self.main_active = false;
        self.pending = true;
    }

    fn arrive_aux(&mut self) -> bool {
        self.arrive()
    }

    fn leave_aux(&mut self) {
        self.pending = true;
    }

    fn cancel_pending(&mut self) {
        self.pending = false;
    }

    fn take_armed(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    fn mark_auto_paused(&mut self) {
        self.auto_paused = true;
    }

    /// Shared arrival core: cancel pending, consume the resume flag.
    fn arrive(&mut self) -> bool {
        self.pending = false;
        std::mem::replace(&mut self.auto_paused, false)
    }
}

/// Focus-domain event: the only inputs the coordinator accepts.
/// Dispatch maps `WindowEvent::Focused` to these; nothing else may
/// arm or cancel the deferred pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusEvent {
    Main(bool),
    Aux(AuxKind, bool),
    BurstEnd,
}

/// Focus-domain effect: what the host must do after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FocusEffect {
    Pause,
    Resume,
}

/// Focus coordinator: owns a [`FocusState`] and maps [`FocusEvent`]
/// to an optional [`FocusEffect`]. `Main(false)`/`Aux(_, false)`
/// only arm; `BurstEnd` commits a still-armed pause; arrivals report
/// a resume. No bools leak out.
pub(crate) struct FocusCoordinator {
    state: FocusState,
}

impl FocusCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            state: FocusState::new(),
        }
    }

    pub(crate) fn main_active(&self) -> bool {
        self.state.main_active()
    }

    /// Narrow aux entry used by dispatch: kind-routed, no direct
    /// access to `arrive_aux`/`cancel_pending`/`take_armed`.
    pub(crate) fn on_aux(&mut self, kind: AuxKind, focused: bool) -> bool {
        self.state.on_aux(kind, focused)
    }

    pub(crate) fn on(&mut self, event: FocusEvent) -> Option<FocusEffect> {
        match event {
            FocusEvent::Main(true) => {
                if self.state.arrive_main() {
                    Some(FocusEffect::Resume)
                } else {
                    None
                }
            }
            FocusEvent::Main(false) => {
                self.state.leave_main();
                None
            }
            FocusEvent::Aux(kind, focused) => {
                if self.state.on_aux(kind, focused) {
                    Some(FocusEffect::Resume)
                } else {
                    None
                }
            }
            FocusEvent::BurstEnd => {
                if self.state.take_armed() {
                    // Armed and uncancelled: the caller commits the
                    // pause (checking `can_pause`) and marks the
                    // protocol flag via `note_protocol_pause`.
                    Some(FocusEffect::Pause)
                } else {
                    None
                }
            }
        }
    }

    /// Record a protocol-driven pause (user pauses never set this).
    pub(crate) fn note_protocol_pause(&mut self) {
        self.state.mark_auto_paused();
    }

    #[cfg(test)]
    pub(crate) fn state_mut(&mut self) -> &mut FocusState {
        &mut self.state
    }
}

/// Shared aux-focus step: same-app arrival cancels a pending pause
/// (reporting resume); leaving anywhere arms it. Single source below.
fn aux_focus_step(focus: &mut FocusState, focused: bool) -> bool {
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
fn aux_focus_step_quiet(focus: &mut FocusState, focused: bool) {
    if focused {
        // Cancel only: unlike `arrive_aux`, the auto-pause flag is
        // preserved so a later main-window arrival still resumes.
        focus.cancel_pending();
    } else {
        focus.leave_aux();
    }
}

/// Shared aux-focus step with the resume protocol chosen by window
/// kind. Returns whether a system-paused session should resume.
pub(crate) fn on_aux_focus(focus: &mut FocusState, kind: AuxKind, focused: bool) -> bool {
    match kind {
        AuxKind::Debug | AuxKind::Ppu => aux_focus_step(focus, focused),
        AuxKind::Settings | AuxKind::History => {
            aux_focus_step_quiet(focus, focused);
            false
        }
    }
}

/// Uniform aux-window identity + close + render surface. Narrow:
/// no shell access leaks; event handling lives in [`AuxEvents`].
pub(crate) trait AuxWindow {
    fn kind(&self) -> AuxKind;
    /// Native window identity for event routing.
    fn tao_window_id(&self) -> tao::window::WindowId;
    /// Close was requested through the shared flag.
    fn close_requested(&self) -> bool;
    /// Bring this viewer to the front.
    fn focus_window(&self);
    /// Native redraw request without reaching into the window.
    fn request_redraw(&self);
    /// Shared event prelude: resize/scale/modifiers/close.
    fn handle_common(&mut self, event: &WindowEvent);
    /// Convert one native event for the window's own dispatcher.
    fn feed_event(&mut self, event: WindowEvent) -> Option<iced::Event>;
    /// Full render through the shared pump; only the log tag differs.
    fn render(&mut self);
}

/// Uniform aux-window event handling: one converted event in.
/// Returns whether this pass already requested a redraw, so the pump
/// can skip its trailing request.
pub(crate) trait AuxEvents {
    fn handle(&mut self, mapped: iced::Event) -> bool;
}

/// Feed one native event through the shared prelude + program
/// dispatch + redraw request. Free function so the four aux windows
/// share one pump instead of copying it per window.
pub(crate) fn pump_aux<W: AuxWindow + AuxEvents>(window: &mut W, event: WindowEvent) {
    window.handle_common(&event);
    match window.feed_event(event) {
        Some(mapped) => {
            if !window.handle(mapped) {
                window.request_redraw();
            }
        }
        None => window.request_redraw(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_main_blur_commits_pause_at_burst_end() {
        let mut coord = FocusCoordinator::new();
        assert_eq!(coord.on(FocusEvent::Main(false)), None);
        assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
        // Consumed: a second burst end without re-arm is quiet.
        assert_eq!(coord.on(FocusEvent::BurstEnd), None);
    }

    #[test]
    fn coordinator_same_app_arrival_cancels_pause() {
        let mut coord = FocusCoordinator::new();
        assert_eq!(coord.on(FocusEvent::Main(false)), None);
        // Same-burst aux arrival cancels the armed pause.
        assert_eq!(coord.on(FocusEvent::Aux(AuxKind::Debug, true)), None);
        assert_eq!(coord.on(FocusEvent::BurstEnd), None);
    }

    #[test]
    fn coordinator_reports_resume_exactly_once() {
        let mut coord = FocusCoordinator::new();
        assert_eq!(coord.on(FocusEvent::Aux(AuxKind::Debug, false)), None);
        assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
        coord.note_protocol_pause();
        assert_eq!(coord.on(FocusEvent::Main(true)), Some(FocusEffect::Resume));
        assert_eq!(coord.on(FocusEvent::Main(true)), None);
    }

    #[test]
    fn coordinator_settings_arrival_preserves_resume() {
        let mut coord = FocusCoordinator::new();
        assert_eq!(coord.on(FocusEvent::Aux(AuxKind::Settings, false)), None);
        assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
        coord.note_protocol_pause();
        // Settings arrival cancels without resuming.
        assert_eq!(coord.on(FocusEvent::Aux(AuxKind::Settings, true)), None);
        assert_eq!(coord.on(FocusEvent::Main(true)), Some(FocusEffect::Resume));
    }

    #[test]
    fn aux_kind_routes_resume_protocol() {
        for kind in [AuxKind::Debug, AuxKind::Ppu] {
            let mut coord = FocusCoordinator::new();
            assert_eq!(coord.on(FocusEvent::Main(false)), None);
            assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
            coord.note_protocol_pause();
            assert!(coord.on_aux(kind, true));
        }
        for kind in [AuxKind::Settings, AuxKind::History] {
            let mut coord = FocusCoordinator::new();
            assert_eq!(coord.on(FocusEvent::Main(false)), None);
            assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
            coord.note_protocol_pause();
            assert!(!coord.on_aux(kind, true));
            assert!(coord.state_mut().arrive_main());
        }
    }

    #[test]
    fn memo_check_gates_repeats() {
        let mut memo: Memo<(u64, u8)> = Memo::new();
        assert!(memo.check(&(1, 2)));
        assert!(!memo.check(&(1, 2)));
        assert!(memo.check(&(1, 3)));
        memo.clear();
        assert!(memo.check(&(1, 3)));
    }

    #[test]
    fn aux_kind_log_tags_are_central() {
        assert_eq!(AuxKind::Debug.log_tag(), "debugger");
        assert_eq!(AuxKind::Ppu.log_tag(), "PPU viewer");
        assert_eq!(AuxKind::History.log_tag(), "history window");
        assert_eq!(AuxKind::Settings.log_tag(), "settings");
    }
}
