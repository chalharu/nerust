//! Aux-window focus agreement as behavior-carrying types.
//! Aux handles implement [`AuxWindow`] + [`AuxEvents`]; runtime only drives them.

use crate::aux::native::{AuxNativeEvent, NativeWindowId};
use crate::aux::shell::AuxShellOps;

/// Which aux window changed focus. Interactive viewers may report a resume;
/// read-only viewers (settings, history) only cancel a pending pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// Main-window focus truth plus the deferred auto-pause protocol.
/// Pauses land only via burst-end commit; arrivals report a resume at most once.
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

    /// Aux-window focus change with the resume protocol chosen by kind.
    /// Returns whether a system-paused session should resume.
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
/// Only `WindowEvent::Focused` maps to these.
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

/// Focus coordinator: maps [`FocusEvent`] to an optional [`FocusEffect`].
/// Departures only arm; `BurstEnd` commits a still-armed pause.
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
                    // Armed and uncancelled: the caller commits the pause.
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

/// Quiet aux-focus step for windows that must never resume: arrival only cancels.
/// Settings uses this; debug/PPU/history use [`aux_focus_step`].
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

/// Identity half: routing, liveness, and redraw requests.
pub(crate) trait AuxIdentity {
    fn kind(&self) -> AuxKind;
    /// Native window identity for event routing.
    fn native_id(&self) -> NativeWindowId;
    /// Close was requested through the shared flag.
    fn close_requested(&self) -> bool;
    /// Bring this viewer to the front.
    fn focus_window(&self);
    /// Native redraw request without reaching into the window.
    fn request_redraw(&self);
}

/// Render half: event prelude, conversion, and full render.
pub(crate) trait AuxRender {
    /// Shared event prelude: resize/scale/modifiers/close.
    fn handle_common(&mut self, event: &AuxNativeEvent);
    /// Convert one domain event for the window's own dispatcher.
    fn feed_event(&mut self, event: AuxNativeEvent) -> Option<iced::Event>;
    /// Full render through the shared pump; only the log tag differs.
    fn render(&mut self);
}

/// Combined pump surface: every aux window speaks both halves.
pub(crate) trait AuxWindow: AuxIdentity + AuxRender {}
impl<T: AuxIdentity + AuxRender + ?Sized> AuxWindow for T {}

/// Single shell accessor behind every aux handle.
/// Blanket [`AuxIdentity`]/[`AuxRender`] impls delegate here.
pub(crate) trait AuxShellRef {
    fn kind(&self) -> AuxKind;
    fn shell(&self) -> &dyn AuxShellOps;
    fn shell_mut(&mut self) -> &mut dyn AuxShellOps;
}

impl<T: AuxShellRef + ?Sized> AuxIdentity for T {
    fn kind(&self) -> AuxKind {
        AuxShellRef::kind(self)
    }

    fn native_id(&self) -> NativeWindowId {
        self.shell().native_id()
    }

    fn close_requested(&self) -> bool {
        self.shell().close_requested()
    }

    fn focus_window(&self) {
        self.shell().focus_window();
    }

    fn request_redraw(&self) {
        self.shell().request_redraw();
    }
}

impl<T: AuxShellRef + ?Sized> AuxRender for T {
    fn handle_common(&mut self, event: &AuxNativeEvent) {
        self.shell_mut().handle_common(event);
    }

    fn feed_event(&mut self, event: AuxNativeEvent) -> Option<iced::Event> {
        self.shell_mut().feed_event(event)
    }

    fn render(&mut self) {
        let tag = self.kind().log_tag();
        self.shell_mut().render_tagged(tag);
    }
}

/// Uniform aux-window event handling: one converted event in.
/// Returns whether this pass already requested a redraw.
pub(crate) trait AuxEvents {
    fn handle(&mut self, mapped: iced::Event) -> bool;
}

/// Feed one domain event through the shared prelude + program dispatch + redraw.
pub(crate) fn pump_aux<W: AuxWindow + AuxEvents + ?Sized>(window: &mut W, event: AuxNativeEvent) {
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
            assert_eq!(
                coord.on(FocusEvent::Aux(kind, true)),
                Some(FocusEffect::Resume)
            );
        }
        for kind in [AuxKind::Settings, AuxKind::History] {
            let mut coord = FocusCoordinator::new();
            assert_eq!(coord.on(FocusEvent::Main(false)), None);
            assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
            coord.note_protocol_pause();
            assert_eq!(coord.on(FocusEvent::Aux(kind, true)), None);
            assert!(coord.state_mut().arrive_main());
        }
    }

    #[test]
    fn aux_kind_log_tags_are_central() {
        assert_eq!(AuxKind::Debug.log_tag(), "debugger");
        assert_eq!(AuxKind::Ppu.log_tag(), "PPU viewer");
        assert_eq!(AuxKind::History.log_tag(), "history window");
        assert_eq!(AuxKind::Settings.log_tag(), "settings");
    }
}
