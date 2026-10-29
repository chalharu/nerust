//! Toolkit-independent end-of-burst ordering. Native event loops retain exit,
//! event dispatch, drawing, and the translation of [`TurnOutcome`] to a wakeup.

use std::time::{Duration, Instant};

/// Per-pump wake contribution. All deadlines are folded before choosing a wake.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WakeNeed {
    pub live_tick: Option<Instant>,
    pub flight_pending: bool,
    pub overlay_deadline: Option<Instant>,
}

impl WakeNeed {
    pub fn fold(self, other: Self) -> Self {
        Self {
            live_tick: self.live_tick.into_iter().chain(other.live_tick).min(),
            flight_pending: self.flight_pending || other.flight_pending,
            overlay_deadline: self
                .overlay_deadline
                .into_iter()
                .chain(other.overlay_deadline)
                .min(),
        }
    }
}

/// Bounds flight reply latency even when no native event wakes the host.
pub fn flight_backstop(now: Instant) -> Instant {
    now + Duration::from_millis(30)
}

/// Earliest deadline wins; no deadline allows the native loop to sleep.
pub fn earliest_wake(input: WakeNeed, now: Instant) -> Option<Instant> {
    [
        input.live_tick,
        input.flight_pending.then(|| flight_backstop(now)),
        input.overlay_deadline,
    ]
    .into_iter()
    .flatten()
    .min()
}

/// Pure wake/repaint decision, preserved for native host policy tests.
pub fn update_flow(
    now: Instant,
    present: bool,
    suspended: bool,
    host_active: bool,
    live_follow: bool,
    input: WakeNeed,
) -> (Option<Instant>, bool) {
    (
        earliest_wake(input, now),
        host_active || live_follow || (present && !suspended),
    )
}

/// Fresh post-pump state, not the tick stored in the turn's frame capture.
#[derive(Debug, Clone, Copy, Default)]
pub struct TurnLiveness {
    pub present: bool,
    pub suspended: bool,
    pub main_active: bool,
    pub live_follow: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnOutcome {
    pub deadline: Option<Instant>,
    pub wants_redraw: bool,
}

/// Narrow native ports; the driver, not each adapter, owns their ordering.
pub trait LoopHost {
    /// Production adapters use `FrameCtx`; an owned capture avoids borrowing
    /// the session across pumps that can change suspension state.
    type Frame;

    /// Commit deferred end-of-burst focus effects before capturing the frame.
    fn before_turn(&mut self);
    fn frame_ctx(&mut self) -> Self::Frame;
    fn chrome(&mut self, ctx: &Self::Frame);
    fn pump_widgets(&mut self) -> WakeNeed;
    fn pump_dialog(&mut self, ctx: &Self::Frame) -> WakeNeed;
    /// Pump native auxiliary windows and collect render-retry deadlines.
    fn pump_aux(&mut self) -> WakeNeed;
    /// Must read the current session tick: drains/modal ACKs can resume it.
    fn late_liveness(&mut self) -> TurnLiveness;
    fn request_redraw(&mut self);

    /// Read after all pumps, so a slow poll cannot age the flight backstop.
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Run one end-of-burst turn. Fatal/exit checks belong to the native caller.
pub fn drive_turn(host: &mut impl LoopHost) -> TurnOutcome {
    host.before_turn();
    let ctx = host.frame_ctx();
    host.chrome(&ctx);
    let widgets = host.pump_widgets();
    let dialog = host.pump_dialog(&ctx);
    let aux = host.pump_aux();
    let wake = widgets.fold(dialog).fold(aux);
    // Never park on the captured pre-ACK pause state, nor a pre-poll clock.
    let live = host.late_liveness();
    let (deadline, wants_redraw) = update_flow(
        host.now(),
        live.present,
        live.suspended,
        live.main_active,
        live.live_follow,
        wake,
    );
    if wants_redraw {
        host.request_redraw();
    }
    TurnOutcome {
        deadline,
        wants_redraw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Host {
        calls: Vec<&'static str>,
        live: TurnLiveness,
        captured_paused: bool,
        acknowledge: bool,
        focus_pause: Option<bool>,
        debug_pause: Option<bool>,
        aux_pause: Option<bool>,
        debug: WakeNeed,
        osd: WakeNeed,
        aux: WakeNeed,
        clock: Cell<Instant>,
    }

    impl Host {
        fn new(now: Instant) -> Self {
            Self {
                calls: Vec::new(),
                live: TurnLiveness::default(),
                captured_paused: false,
                acknowledge: false,
                focus_pause: None,
                debug_pause: None,
                aux_pause: None,
                debug: WakeNeed::default(),
                osd: WakeNeed::default(),
                aux: WakeNeed::default(),
                clock: Cell::new(now),
            }
        }
    }

    impl LoopHost for Host {
        type Frame = bool;

        fn before_turn(&mut self) {
            self.calls.push("focus");
            if let Some(paused) = self.focus_pause {
                self.live.suspended = paused;
            }
        }

        fn frame_ctx(&mut self) -> bool {
            self.calls.push("capture");
            self.captured_paused = self.live.suspended;
            self.captured_paused
        }

        fn chrome(&mut self, ctx: &bool) {
            self.calls.push("chrome");
            assert_eq!(*ctx, self.captured_paused);
        }

        fn pump_widgets(&mut self) -> WakeNeed {
            self.calls.push("debug");
            if let Some(paused) = self.debug_pause {
                self.live.suspended = paused;
            }
            self.debug
        }

        fn pump_dialog(&mut self, ctx: &bool) -> WakeNeed {
            self.calls.push("osd");
            assert_eq!(*ctx, self.captured_paused);
            if self.acknowledge {
                self.live.suspended = false;
            }
            self.osd
        }

        fn pump_aux(&mut self) -> WakeNeed {
            self.calls.push("aux/retry");
            if let Some(paused) = self.aux_pause {
                self.live.suspended = paused;
            }
            self.clock
                .set(self.clock.get() + Duration::from_millis(100));
            self.aux
        }

        fn late_liveness(&mut self) -> TurnLiveness {
            self.calls.push("fresh tick");
            self.live
        }

        fn request_redraw(&mut self) {
            self.calls.push("redraw");
        }

        fn now(&self) -> Instant {
            self.clock.get()
        }
    }

    #[test]
    fn ordered_turn_uses_post_modal_ack_tick_instead_of_parking() {
        let mut host = Host::new(Instant::now());
        host.live.present = true;
        host.live.suspended = true;
        host.acknowledge = true;
        let outcome = drive_turn(&mut host);
        assert!(host.captured_paused);
        assert!(outcome.wants_redraw, "modal ACK resumed the unfocused core");
        assert_eq!(
            host.calls,
            [
                "focus",
                "capture",
                "chrome",
                "debug",
                "osd",
                "aux/retry",
                "fresh tick",
                "redraw",
            ]
        );
    }

    #[test]
    fn deferred_focus_settles_before_the_only_capture() {
        let mut host = Host::new(Instant::now());
        host.live.present = true;
        host.focus_pause = Some(true);
        let outcome = drive_turn(&mut host);
        assert!(host.captured_paused);
        assert!(!outcome.wants_redraw);
        assert_eq!(
            host.calls.iter().filter(|call| **call == "capture").count(),
            1
        );
    }

    #[test]
    fn debug_resume_and_aux_pause_are_both_observed_after_all_pumps() {
        let mut host = Host::new(Instant::now());
        host.live.present = true;
        host.live.suspended = true;
        host.debug_pause = Some(false);
        assert!(drive_turn(&mut host).wants_redraw);
        // A later native auxiliary effect wins over a completed drain.
        host.aux_pause = Some(true);
        assert!(!drive_turn(&mut host).wants_redraw);
    }

    #[test]
    fn flight_backstop_uses_clock_after_slow_pumps() {
        let now = Instant::now();
        let mut host = Host::new(now);
        host.debug.flight_pending = true;
        let outcome = drive_turn(&mut host);
        assert_eq!(outcome.deadline, Some(now + Duration::from_millis(130)));
        assert!(!outcome.wants_redraw);
    }

    #[test]
    fn deadlines_fold_across_pumps_and_compete_with_backstop() {
        let now = Instant::now();
        let mut host = Host::new(now);
        host.debug = WakeNeed {
            live_tick: Some(now + Duration::from_millis(400)),
            flight_pending: true,
            overlay_deadline: None,
        };
        host.osd.overlay_deadline = Some(now + Duration::from_millis(300));
        host.aux.overlay_deadline = Some(now + Duration::from_millis(120));
        assert_eq!(drive_turn(&mut host).deadline, host.aux.overlay_deadline);
        host.clock.set(now);
        host.aux.overlay_deadline = None;
        assert_eq!(
            drive_turn(&mut host).deadline,
            Some(flight_backstop(host.clock.get()))
        );
        host.clock.set(now);
        host.osd.live_tick = Some(now + Duration::from_millis(110));
        assert_eq!(drive_turn(&mut host).deadline, host.osd.live_tick);
    }

    #[test]
    fn idle_and_paused_sleep_but_focus_follow_and_running_redraw() {
        for (present, suspended, main_active, live_follow, redraw) in [
            (false, false, false, false, false),
            (true, true, false, false, false),
            (true, false, false, false, true),
            (false, false, true, false, true),
            (true, true, false, true, true),
        ] {
            let mut host = Host::new(Instant::now());
            host.live = TurnLiveness {
                present,
                suspended,
                main_active,
                live_follow,
            };
            let outcome = drive_turn(&mut host);
            assert_eq!(outcome.deadline, None);
            assert_eq!(outcome.wants_redraw, redraw);
            assert_eq!(host.calls.contains(&"redraw"), redraw);
        }
    }

    #[test]
    fn fold_keeps_earliest_of_each_source_and_pending_from_either_side() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        let a = WakeNeed {
            live_tick: Some(later),
            flight_pending: true,
            overlay_deadline: Some(now),
        };
        let b = WakeNeed {
            live_tick: Some(now),
            flight_pending: false,
            overlay_deadline: Some(later),
        };
        let expected = WakeNeed {
            live_tick: Some(now),
            flight_pending: true,
            overlay_deadline: Some(now),
        };
        assert_eq!(a.fold(b), expected);
        assert_eq!(b.fold(a), expected);
        assert_eq!(earliest_wake(WakeNeed::default(), now), None);
    }
}
