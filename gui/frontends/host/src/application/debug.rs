//! Stateful debug transport. UI ports receive results, never individual queue transitions.

use crate::{
    debug_schedule::DebugSchedule,
    debugger::DrainSnapshot,
    drain::{DrainEvent, DrainQueue, poll_flight, prepare_pause_actions},
    loop_driver::WakeNeed,
    session::SessionEvent,
};
use nerust_frontend_kernel::pause::PausePolicy;
use nerust_gui_shell::{
    debug::{DebugAction, DebugDrainInput, DebugDrainOutput, LiveImagesFlight},
    session::{LiveImages, SessionHandle, commands::SessionCommand},
};
use std::{task::Poll, time::Instant};

pub trait DebugView {
    fn apply_drain(&mut self, output: DebugDrainOutput);
    fn apply_live(&mut self, live: LiveImages, running: bool);
}

pub struct DebugRuntime {
    schedule: DebugSchedule,
    drain: DrainQueue<DrainSnapshot>,
    live_flight: Option<LiveImagesFlight>,
}

impl Default for DebugRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugRuntime {
    pub fn new() -> Self {
        Self {
            schedule: DebugSchedule::new(),
            drain: Default::default(),
            live_flight: None,
        }
    }
    pub fn schedule(&self) -> &DebugSchedule {
        &self.schedule
    }
    pub fn schedule_mut(&mut self) -> &mut DebugSchedule {
        &mut self.schedule
    }
    pub fn pending(&self) -> bool {
        self.drain.pending()
    }
    pub fn close(&mut self) {
        self.drain.clear();
        self.live_flight = None;
        self.schedule.set_live_wanted(false);
        // Consume deferred opens so a queued event cannot reopen on teardown.
        self.schedule.take_debug_open_request();
        self.schedule.take_ppu_open_request();
    }
    pub fn enqueue(
        &mut self,
        session: &SessionHandle,
        requests: &[DebugAction],
        snapshot: Option<DrainSnapshot>,
    ) {
        if let Some(snapshot) = snapshot {
            self.drain
                .enqueue(session.debug_epoch(), requests, snapshot);
        } else {
            self.schedule
                .set_last_drain_paused(session.tick_metrics().paused);
        }
    }
    fn start_batch(
        &mut self,
        session: &mut SessionHandle,
        pause: &mut PausePolicy,
        batch: crate::drain::DrainBatch<DrainSnapshot>,
    ) {
        let flight = batch.begin(|actions, snapshot| {
            prepare_pause_actions(actions, |action| {
                let command = match action {
                    DebugAction::Pause => SessionCommand::Pause,
                    DebugAction::Resume => SessionCommand::Resume,
                    DebugAction::TogglePause => SessionCommand::TogglePause,
                    _ => unreachable!("pause actions only"),
                };
                let _ = crate::session::dispatch(session, pause, SessionEvent::Command(command));
                session.tick_metrics().loaded
            });
            session.debug_drain_begin(
                actions,
                &DebugDrainInput {
                    space: snapshot.space,
                    mem_addr: snapshot.mem_addr,
                    dis_addr: snapshot.dis_addr,
                    watch: snapshot.watch,
                    freeze: snapshot.freeze,
                    reg_target: snapshot.reg_target,
                    prev_dump_rows: &snapshot.prev_rows,
                    need_images: snapshot.ppu_open,
                },
            )
        });
        if let Some(flight) = flight {
            self.drain.store(flight);
        }
    }
    pub fn poll(
        &mut self,
        session: &mut SessionHandle,
        pause: &mut PausePolicy,
        view: &mut impl DebugView,
    ) -> bool {
        loop {
            match self.drain.poll() {
                DrainEvent::Idle => return false,
                DrainEvent::Pending => return true,
                DrainEvent::Start(batch) => self.start_batch(session, pause, batch),
                DrainEvent::Done(output, _) => {
                    view.apply_drain(*output);
                    self.schedule
                        .set_last_drain_paused(session.tick_metrics().paused);
                }
            }
        }
    }
    pub fn live_follow_active(session: &SessionHandle, debug_open: bool, ppu_open: bool) -> bool {
        let tick = session.tick_metrics();
        debug_open && ppu_open && tick.loaded && !tick.paused
    }
    fn poll_live(&mut self, session: &SessionHandle, view: &mut impl DebugView) -> bool {
        match poll_flight(&mut self.live_flight) {
            Poll::Ready(Some(live)) => view.apply_live(live, !session.tick_metrics().paused),
            Poll::Ready(None) => {}
            Poll::Pending => return true,
        }
        false
    }
    pub fn pump_live(
        &mut self,
        session: &SessionHandle,
        debug_open: bool,
        ppu_open: bool,
        view: &mut impl DebugView,
        now: Instant,
    ) {
        if self.poll_live(session, view)
            || !Self::live_follow_active(session, debug_open, ppu_open)
            || !self.schedule.live_due(now)
        {
            return;
        }
        self.live_flight = Some(session.debug_live_images_begin(true));
        self.poll_live(session, view);
    }
    pub fn wake(
        &mut self,
        session: &SessionHandle,
        debug_open: bool,
        ppu_open: bool,
        drain_pending: bool,
    ) -> WakeNeed {
        let follow = Self::live_follow_active(session, debug_open, ppu_open);
        self.schedule.set_live_wanted(follow);
        WakeNeed {
            live_tick: follow.then(|| self.schedule.next_live_tick()).flatten(),
            flight_pending: drain_pending || self.live_flight.is_some(),
            osd_deadline: None,
        }
    }
    pub fn chrome_drain_due(&self, debug_open: bool, paused: bool) -> bool {
        debug_open && self.schedule.last_drain_paused() != Some(paused) && !self.drain.pending()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ApplicationHost, tests::Fixture};
    use super::*;
    use crate::drain::DrainBatch;
    use nerust_gui_shell::session::access::PauseControl;

    #[derive(Default)]
    struct View {
        applied: usize,
        live: usize,
    }
    impl DebugView for View {
        fn apply_drain(&mut self, _: DebugDrainOutput) {
            self.applied += 1;
        }
        fn apply_live(&mut self, _: LiveImages, _: bool) {
            self.live += 1;
        }
    }
    fn snapshot(core: &ApplicationHost, freeze: u8) -> DrainSnapshot {
        DrainSnapshot {
            space: Some(core.session().debug_spaces()[0].id),
            mem_addr: 0,
            dis_addr: None,
            watch: None,
            freeze: Some((0, freeze)),
            reg_target: Some("a"),
            prev_rows: Default::default(),
            ppu_open: false,
        }
    }
    fn start(core: &mut ApplicationHost, actions: Vec<DebugAction>, snapshot: DrainSnapshot) {
        let epoch = core.debug_epoch();
        core.debug.start_batch(
            &mut core.session,
            &mut core.pause_policy,
            DrainBatch {
                epoch,
                actions,
                snapshot,
            },
        );
    }
    fn settle(core: &mut ApplicationHost, view: &mut View) {
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while core.poll_debug(view) {
            assert!(Instant::now() < deadline, "worker did not settle");
            std::thread::yield_now();
        }
    }

    #[test]
    fn replacement_discards_waiting_old_epoch_batches_before_pause_or_write() {
        let mut fixture = Fixture::new();
        fixture.load();
        assert!(matches!(
            fixture.core.session().debug_pause(),
            nerust_gui_viewmodel::debugger::StatusMessage::Paused
        ));
        let old = fixture.core.debug_epoch();
        let snapshot = snapshot(&fixture.core, 0xff);
        start(
            &mut fixture.core,
            vec![DebugAction::StepInstr, DebugAction::RegisterWrite(0xff)],
            snapshot.clone(),
        );
        fixture.core.enqueue_debug(
            &[DebugAction::Pause, DebugAction::RegisterWrite(0xaa)],
            Some(snapshot),
        );
        fixture.load();
        assert!(!old.load(std::sync::atomic::Ordering::Acquire));
        let mut view = View::default();
        assert!(!fixture.core.poll_debug(&mut view));
        assert!(!fixture.core.tick().paused);
        assert!(!fixture.core.debug_pending());
        assert_eq!(
            view.applied, 0,
            "old output must never reach replacement UI"
        );
        fixture.core.pause();
        let current = fixture.core.session().debug_snapshot(
            None,
            0,
            None,
            nerust_gui_shell::debug::ImageNeed::Skip,
            None,
        );
        assert!(current.regs.contains(&("a", 0)));
    }

    #[test]
    fn queued_pause_waits_for_prior_real_worker_flight_and_chrome_cannot_replace_it() {
        let mut fixture = Fixture::new();
        fixture.load();
        fixture.core.pause();
        let snapshot = snapshot(&fixture.core, 0x5a);
        let space = snapshot.space.unwrap();
        start(
            &mut fixture.core,
            vec![DebugAction::StepInstr, DebugAction::RegisterWrite(0x45)],
            snapshot.clone(),
        );
        fixture
            .core
            .enqueue_debug(&[DebugAction::Resume], Some(snapshot));
        assert!(fixture.core.tick().paused, "resume has not started");
        assert!(!fixture.core.chrome_drain_due(true));
        assert_eq!(
            fixture.core.debug.drain.queued_len(),
            1,
            "chrome may not replace pending drain"
        );
        let mut view = View::default();
        settle(&mut fixture.core, &mut view);
        assert!(!fixture.core.tick().paused);
        assert!(!fixture.core.debug_pending());
        assert_eq!(view.applied, 2);
        fixture.core.pause();
        assert_eq!(fixture.core.session().debug_read_byte(space, 0), Some(0x5a));
        let current = fixture.core.session().debug_snapshot(
            None,
            0,
            None,
            nerust_gui_shell::debug::ImageNeed::Skip,
            None,
        );
        assert!(current.regs.contains(&("a", 0x45)));
    }

    #[test]
    fn focus_chrome_drains_only_on_pause_transitions() {
        let mut runtime = DebugRuntime::new();
        for baseline in [None, Some(true), Some(false)] {
            if let Some(paused) = baseline {
                runtime.schedule.set_last_drain_paused(paused);
            }
            for paused in [false, true] {
                assert!(!runtime.chrome_drain_due(false, paused));
                assert_eq!(
                    runtime.chrome_drain_due(true, paused),
                    baseline != Some(paused)
                );
            }
        }
    }

    #[test]
    fn debug_orchestrator_flights_start_empty_and_close_clean() {
        let mut fixture = Fixture::new();
        assert!(!fixture.core.debug.drain.flight_pending());
        assert!(fixture.core.debug.live_flight.is_none());
        fixture.load();
        fixture.core.pause();
        let snapshot = snapshot(&fixture.core, 0x77);
        start(
            &mut fixture.core,
            vec![DebugAction::Refresh],
            snapshot.clone(),
        );
        fixture
            .core
            .enqueue_debug(&[DebugAction::Resume], Some(snapshot));
        fixture.core.set_live_wanted(true);
        fixture.core.close_debug_flights();
        assert!(!fixture.core.debug_pending());
        assert!(fixture.core.debug.live_flight.is_none());
        assert!(
            !fixture
                .core
                .live_armed()
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert!(!fixture.core.poll_debug(&mut View::default()));
        assert!(fixture.core.tick().paused, "cancelled resume never starts");
    }

    #[test]
    fn live_cadence_and_late_pause_truth_are_common_owned() {
        let mut fixture = Fixture::new();
        fixture.load();
        let mut view = View::default();
        fixture.core.pump_live_debug(true, true, &mut view);
        let due = fixture.core.next_live_tick().unwrap();
        assert!(
            !fixture
                .core
                .live_due(due - std::time::Duration::from_nanos(1))
        );
        assert!(
            fixture
                .core
                .debug_wake(true, true, false)
                .live_tick
                .is_some()
        );
        assert!(
            fixture
                .core
                .live_armed()
                .load(std::sync::atomic::Ordering::Acquire)
        );
        fixture.core.pause();
        assert!(
            fixture
                .core
                .debug_wake(true, true, false)
                .live_tick
                .is_none()
        );
        assert!(
            !fixture
                .core
                .live_armed()
                .load(std::sync::atomic::Ordering::Acquire)
        );
    }

    #[test]
    fn common_debug_turn_arms_follow_from_post_drain_resume() {
        let mut fixture = Fixture::new();
        fixture.load();
        fixture.core.pause();
        let snapshot = snapshot(&fixture.core, 0);
        fixture
            .core
            .enqueue_debug(&[DebugAction::Resume], Some(snapshot));
        let captured = fixture.core.frame_ctx();
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        let mut view = View::default();
        loop {
            let wake = fixture.core.pump_debug(true, true, &mut view);
            assert!(Instant::now() < deadline, "worker did not settle");
            if !fixture.core.debug_pending() {
                assert!(!fixture.core.tick().paused);
                assert!(captured.tick().paused);
                assert!(
                    fixture
                        .core
                        .live_armed()
                        .load(std::sync::atomic::Ordering::Acquire)
                );
                assert_eq!(view.applied, 1);
                assert_eq!(wake.live_tick, fixture.core.next_live_tick());
                break;
            }
            assert!(wake.flight_pending);
            std::thread::yield_now();
        }
    }
}
