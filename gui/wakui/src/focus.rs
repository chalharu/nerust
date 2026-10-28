//! Toolkit-independent auxiliary-window focus and deferred auto-pause protocol.

/// Which aux window changed focus. Interactive viewers may report a resume;
/// read-only viewers (settings, history) only cancel a pending pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuxKind {
    Debug,
    Ppu,
    History,
    Settings,
}

impl AuxKind {
    /// Windows whose arrival resumes a system-paused session.
    /// Arrival anywhere else only cancels a pending auto-pause.
    pub fn resume_on_arrival(self) -> bool {
        matches!(self, AuxKind::Debug | AuxKind::Ppu)
    }

    /// Central log tag for every aux window.
    pub fn log_tag(self) -> &'static str {
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
struct FocusState {
    /// Main window holds focus.
    main_active: bool,
    /// Session paused by the protocol, never by the user directly.
    auto_paused: bool,
    /// Blur armed a pause; committed at burst end unless cancelled.
    pending: bool,
}

impl FocusState {
    fn new() -> Self {
        Self {
            main_active: true,
            auto_paused: false,
            pending: false,
        }
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
/// Native adapters translate focus callbacks and the end of an event burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusEvent {
    Main(bool),
    Aux(AuxKind, bool),
    BurstEnd,
}

/// Focus-domain effect: what the host must do after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusEffect {
    Pause,
    Resume,
}

/// Focus coordinator: maps [`FocusEvent`] to an optional [`FocusEffect`].
/// Departures only arm; `BurstEnd` commits a still-armed pause.
pub struct FocusCoordinator {
    state: FocusState,
}

impl Default for FocusCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusCoordinator {
    pub fn new() -> Self {
        Self {
            state: FocusState::new(),
        }
    }

    pub fn main_active(&self) -> bool {
        self.state.main_active
    }

    pub fn on(&mut self, event: FocusEvent) -> Option<FocusEffect> {
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
                if on_aux_focus(&mut self.state, kind, focused) {
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
    pub fn note_protocol_pause(&mut self) {
        self.state.mark_auto_paused();
    }
}

/// Shared aux-focus step with the resume protocol chosen by window kind.
/// Arrival resumes for debug/PPU; settings/history only cancel a pending pause.
fn on_aux_focus(focus: &mut FocusState, kind: AuxKind, focused: bool) -> bool {
    if focused {
        if kind.resume_on_arrival() {
            focus.arrive_aux()
        } else {
            // Preserve the auto-pause flag for a later main-window arrival.
            focus.cancel_pending();
            false
        }
    } else {
        focus.leave_aux();
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinator_main_blur_commits_pause_at_burst_end() {
        let mut coord = FocusCoordinator::new();
        assert_eq!(coord.on(FocusEvent::Main(false)), None);
        assert!(!coord.main_active());
        assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
        assert_eq!(coord.on(FocusEvent::BurstEnd), None);
    }

    #[test]
    fn coordinator_same_app_arrival_cancels_pause() {
        let mut coord = FocusCoordinator::new();
        assert_eq!(coord.on(FocusEvent::Main(false)), None);
        assert_eq!(coord.on(FocusEvent::Aux(AuxKind::Debug, true)), None);
        assert_eq!(coord.on(FocusEvent::BurstEnd), None);
    }

    #[test]
    fn coordinator_same_burst_event_orders_preserve_last_event_protocol() {
        for kind in [
            AuxKind::Debug,
            AuxKind::Ppu,
            AuxKind::History,
            AuxKind::Settings,
        ] {
            for (departure, arrival) in [
                (FocusEvent::Main(false), FocusEvent::Aux(kind, true)),
                (FocusEvent::Aux(kind, false), FocusEvent::Main(true)),
            ] {
                for arrival_first in [false, true] {
                    let mut coord = FocusCoordinator::new();
                    let events = if arrival_first {
                        [arrival, departure]
                    } else {
                        [departure, arrival]
                    };
                    for event in events {
                        assert_eq!(coord.on(event), None);
                    }
                    // Arrivals cancel earlier departures, not later ones.
                    assert_eq!(
                        coord.on(FocusEvent::BurstEnd),
                        arrival_first.then_some(FocusEffect::Pause)
                    );
                    assert_eq!(coord.on(FocusEvent::BurstEnd), None);
                }
            }
        }
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
    fn coordinator_requires_explicit_protocol_pause_and_consumes_it_once() {
        for arrival in [
            FocusEvent::Main(true),
            FocusEvent::Aux(AuxKind::Debug, true),
            FocusEvent::Aux(AuxKind::Ppu, true),
        ] {
            let mut coord = FocusCoordinator::default();
            assert!(coord.main_active());
            assert_eq!(coord.on(FocusEvent::Main(false)), None);
            assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
            // An emitted Pause is not proof the caller actually paused.
            assert_eq!(coord.on(arrival), None);
            coord.note_protocol_pause();
            assert_eq!(coord.on(arrival), Some(FocusEffect::Resume));
            assert_eq!(coord.on(arrival), None);
            assert_eq!(coord.on(FocusEvent::Main(true)), None);
            assert_eq!(coord.on(FocusEvent::BurstEnd), None);
        }
    }

    #[test]
    fn coordinator_settings_arrival_preserves_resume() {
        let mut coord = FocusCoordinator::new();
        assert_eq!(coord.on(FocusEvent::Aux(AuxKind::Settings, false)), None);
        assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
        coord.note_protocol_pause();
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
            assert_eq!(coord.on(FocusEvent::BurstEnd), None);
            assert_eq!(coord.on(FocusEvent::Main(true)), Some(FocusEffect::Resume));
            assert_eq!(coord.on(FocusEvent::Main(true)), None);
        }
    }

    #[test]
    fn aux_kind_log_tags_are_central() {
        assert_eq!(AuxKind::Debug.log_tag(), "debugger");
        assert_eq!(AuxKind::Ppu.log_tag(), "PPU viewer");
        assert_eq!(AuxKind::History.log_tag(), "history window");
        assert_eq!(AuxKind::Settings.log_tag(), "settings");
    }

    #[test]
    fn focus_state_tracks_main_and_burst_contract() {
        let mut coord = FocusCoordinator::new();
        assert!(coord.main_active());
        assert_eq!(coord.on(FocusEvent::Aux(AuxKind::Ppu, false)), None);
        assert_eq!(coord.on(FocusEvent::Main(true)), None);
        assert!(coord.main_active());
        assert_eq!(coord.on(FocusEvent::Aux(AuxKind::Debug, false)), None);
        assert_eq!(coord.on(FocusEvent::BurstEnd), Some(FocusEffect::Pause));
        assert_eq!(coord.on(FocusEvent::BurstEnd), None);
    }
}
