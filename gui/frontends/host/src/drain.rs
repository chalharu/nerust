//! Non-blocking frontend drain scheduling. Native code owns pause policy and UI effects.

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::Poll,
};

use nerust_gui_shell::debug::{
    DebugAction, DrainFlight, DrainPoll, LiveImagesFlight, LiveImagesPoll,
};

/// A flight that can advance without blocking the native event loop.
pub trait Flight {
    type Output;

    fn poll(&mut self) -> Poll<Self::Output>;
}

impl Flight for DrainFlight {
    type Output = Box<nerust_gui_shell::debug::DebugDrainOutput>;

    fn poll(&mut self) -> Poll<Self::Output> {
        match DrainFlight::poll(self) {
            DrainPoll::Pending => Poll::Pending,
            DrainPoll::Done(output) => Poll::Ready(output),
        }
    }
}

impl Flight for LiveImagesFlight {
    type Output = nerust_gui_shell::session::LiveImages;

    fn poll(&mut self) -> Poll<Self::Output> {
        match LiveImagesFlight::poll(self) {
            LiveImagesPoll::Pending => Poll::Pending,
            LiveImagesPoll::Done(output) => Poll::Ready(output),
        }
    }
}

/// Poll once, keeping only pending flights. `Ready(None)` means there was no flight;
/// `Ready(Some(output))` also represents a completed imageless live fetch.
pub fn poll_flight<F: Flight>(slot: &mut Option<F>) -> Poll<Option<F::Output>> {
    let Some(mut flight) = slot.take() else {
        return Poll::Ready(None);
    };
    match flight.poll() {
        Poll::Ready(output) => Poll::Ready(Some(output)),
        Poll::Pending => {
            *slot = Some(flight);
            Poll::Pending
        }
    }
}

fn is_pause(action: &DebugAction) -> bool {
    matches!(
        action,
        DebugAction::Pause | DebugAction::Resume | DebugAction::TogglePause
    )
}

/// Apply frontend pause ownership just before starting its batch. The callback
/// returns whether a core is loaded: only then replace the shell pause with refresh.
/// Without a core the original action must survive for the shell's NoCore status.
pub fn prepare_pause_actions(
    actions: &mut [DebugAction],
    mut pause: impl FnMut(DebugAction) -> bool,
) {
    for action in actions {
        if is_pause(action) && pause(*action) {
            *action = DebugAction::Refresh;
        }
    }
}

/// Immutable gesture inputs captured against one worker epoch.
#[derive(Clone)]
pub struct DrainBatch<S> {
    pub epoch: Arc<AtomicBool>,
    pub actions: Vec<DebugAction>,
    pub snapshot: S,
}

impl<S> DrainBatch<S> {
    /// The epoch guard runs before the callback, including frontend pause changes.
    /// Dropping an old batch must never reinterpret it against the replacement core.
    pub fn begin<F>(
        mut self,
        begin: impl FnOnce(&mut [DebugAction], &S) -> F,
    ) -> Option<StartedDrain<S, F>> {
        if !self.epoch.load(Ordering::Acquire) {
            return None;
        }
        let flight = begin(&mut self.actions, &self.snapshot);
        Some(StartedDrain {
            epoch: self.epoch,
            snapshot: self.snapshot,
            flight,
        })
    }
}

/// An epoch-bound flight, constructed only after checking the queued batch.
pub struct StartedDrain<S, F> {
    epoch: Arc<AtomicBool>,
    snapshot: S,
    flight: F,
}

impl<S, F: Flight> StartedDrain<S, F> {
    /// Poll once with epoch checks on both sides. `Ready(None)` discards an
    /// invalidated flight without publishing its output; callers drop it then.
    pub fn poll(&mut self) -> Poll<Option<F::Output>> {
        if !self.epoch.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        let output = self.flight.poll();
        if !self.epoch.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        output.map(Some)
    }
}

/// One native pump step. Apply `Done` before asking for the next `Start`.
pub enum DrainEvent<S, O> {
    Idle,
    Pending,
    Start(DrainBatch<S>),
    Done(O, S),
}

/// FIFO with at most one flight. Invalid epochs are dropped before polling or
/// starting; this is invalidation, not cancellation of already accepted worker ops.
pub struct DrainQueue<S, F = DrainFlight> {
    queue: VecDeque<DrainBatch<S>>,
    flight: Option<StartedDrain<S, F>>,
}

impl<S, F> Default for DrainQueue<S, F> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            flight: None,
        }
    }
}

impl<S, F> DrainQueue<S, F> {
    pub fn clear(&mut self) {
        self.flight = None;
        self.queue.clear();
    }

    pub fn pending(&self) -> bool {
        self.flight.is_some() || !self.queue.is_empty()
    }

    pub fn flight_pending(&self) -> bool {
        self.flight.is_some()
    }

    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }

    /// Append an already separated batch (also useful for deterministic probes).
    pub fn push_back(&mut self, batch: DrainBatch<S>) {
        self.queue.push_back(batch);
    }

    /// Install the flight returned by `DrainBatch::begin`, never replacing a
    /// pending flight. The native pump must settle the previous one first.
    pub fn store(&mut self, flight: StartedDrain<S, F>) {
        assert!(
            self.flight.is_none(),
            "cannot replace a pending debugger drain"
        );
        self.flight = Some(flight);
    }
}

impl<S: Clone, F> DrainQueue<S, F> {
    /// Split immediately before each pause transition, including consecutive
    /// pauses. Empty gestures still enqueue a refresh/read round-trip.
    pub fn enqueue(&mut self, epoch: Arc<AtomicBool>, actions: &[DebugAction], snapshot: S) {
        let mut start = 0;
        for (index, action) in actions.iter().enumerate() {
            if index > start && is_pause(action) {
                self.push_back(DrainBatch {
                    epoch: epoch.clone(),
                    actions: actions[start..index].to_vec(),
                    snapshot: snapshot.clone(),
                });
                start = index;
            }
        }
        self.push_back(DrainBatch {
            epoch,
            actions: actions[start..].to_vec(),
            snapshot,
        });
    }
}

impl<S, F: Flight> DrainQueue<S, F> {
    pub fn poll(&mut self) -> DrainEvent<S, F::Output> {
        if let Some(mut stored) = self.flight.take() {
            match stored.poll() {
                Poll::Ready(Some(output)) => return DrainEvent::Done(output, stored.snapshot),
                Poll::Ready(None) => {}
                Poll::Pending => {
                    self.flight = Some(stored);
                    return DrainEvent::Pending;
                }
            }
        }
        while let Some(batch) = self.queue.pop_front() {
            if batch.epoch.load(Ordering::Acquire) {
                return DrainEvent::Start(batch);
            }
        }
        DrainEvent::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    struct Probe {
        polls: Rc<Cell<usize>>,
        pending: bool,
        invalidate: Option<Arc<AtomicBool>>,
    }

    impl Flight for Probe {
        type Output = usize;

        fn poll(&mut self) -> Poll<usize> {
            self.polls.set(self.polls.get() + 1);
            if let Some(epoch) = self.invalidate.take() {
                epoch.store(false, Ordering::Release);
            }
            if std::mem::take(&mut self.pending) {
                Poll::Pending
            } else {
                Poll::Ready(7)
            }
        }
    }

    fn epoch() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(true))
    }

    fn probe(pending: bool) -> Probe {
        Probe {
            polls: Rc::new(Cell::new(0)),
            pending,
            invalidate: None,
        }
    }

    #[test]
    fn fifo_pause_boundary_waits_for_prior_flight_and_keeps_snapshot() {
        let mut queue = DrainQueue::<_, Probe>::default();
        queue.enqueue(
            epoch(),
            &[
                DebugAction::StepInstr,
                DebugAction::Resume,
                DebugAction::Pause,
            ],
            42,
        );
        assert_eq!(queue.queued_len(), 3);
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("first batch")
        };
        assert!(matches!(batch.actions.as_slice(), [DebugAction::StepInstr]));
        queue.store(batch.begin(|_, _| probe(true)).unwrap());
        assert!(matches!(queue.poll(), DrainEvent::Pending));
        assert_eq!(queue.queued_len(), 2, "pause must not run ahead");
        assert!(matches!(queue.poll(), DrainEvent::Done(7, 42)));
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("resume batch")
        };
        assert!(matches!(batch.actions.as_slice(), [DebugAction::Resume]));
        queue.store(batch.begin(|_, _| probe(false)).unwrap());
        assert!(matches!(queue.poll(), DrainEvent::Done(7, 42)));
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("pause batch")
        };
        assert!(matches!(batch.actions.as_slice(), [DebugAction::Pause]));
        queue.store(batch.begin(|_, _| probe(false)).unwrap());
        assert!(matches!(queue.poll(), DrainEvent::Done(7, 42)));
        assert!(matches!(queue.poll(), DrainEvent::Idle));
        assert!(!queue.pending());
    }

    #[test]
    fn replacement_drops_flight_without_polling_and_skips_queued_mutations() {
        let old = epoch();
        let mut queue = DrainQueue::<_, Probe>::default();
        queue.enqueue(
            old.clone(),
            &[
                DebugAction::StepInstr,
                DebugAction::Pause,
                DebugAction::RegisterWrite(255),
            ],
            1,
        );
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("first batch")
        };
        let flight = probe(true);
        let polls = flight.polls.clone();
        queue.store(batch.begin(|_, _| flight).unwrap());
        old.store(false, Ordering::Release);
        queue.enqueue(epoch(), &[DebugAction::Refresh], 2);
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("replacement batch")
        };
        assert_eq!(batch.snapshot, 2);
        assert_eq!(polls.get(), 0);
        assert_eq!(queue.queued_len(), 0);
    }

    #[test]
    fn invalidation_between_dequeue_and_begin_never_calls_pause_callback() {
        let old = epoch();
        let mut queue = DrainQueue::<_, Probe>::default();
        queue.enqueue(old.clone(), &[DebugAction::Resume], ());
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("batch")
        };
        old.store(false, Ordering::Release);
        assert!(
            batch
                .begin::<Probe>(|_, _| panic!("old pause must not run"))
                .is_none()
        );
    }

    #[test]
    fn invalidation_during_poll_discards_output() {
        let old = epoch();
        let mut queue = DrainQueue::<_, Probe>::default();
        queue.enqueue(old.clone(), &[], ());
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("empty refresh batch")
        };
        let mut flight = probe(false);
        flight.invalidate = Some(old);
        queue.store(batch.begin(|_, _| flight).unwrap());
        assert!(matches!(queue.poll(), DrainEvent::Idle));
        assert!(!queue.pending());
    }

    #[test]
    fn pause_callback_runs_in_order_and_preserves_coreless_status_actions() {
        for loaded in [false, true] {
            let mut actions = [
                DebugAction::Pause,
                DebugAction::RegisterWrite(3),
                DebugAction::TogglePause,
                DebugAction::Resume,
            ];
            let mut pauses = Vec::new();
            prepare_pause_actions(&mut actions, |action| {
                pauses.push(action);
                loaded
            });
            assert!(matches!(
                pauses.as_slice(),
                [
                    DebugAction::Pause,
                    DebugAction::TogglePause,
                    DebugAction::Resume
                ]
            ));
            assert!(matches!(actions[1], DebugAction::RegisterWrite(3)));
            assert_eq!(matches!(actions[0], DebugAction::Refresh), loaded);
            assert_eq!(matches!(actions[2], DebugAction::Refresh), loaded);
            assert_eq!(matches!(actions[3], DebugAction::Refresh), loaded);
        }
    }

    #[test]
    fn clear_drops_pending_and_queued_work_and_live_poll_retains_only_pending() {
        let mut queue = DrainQueue::<_, Probe>::default();
        queue.enqueue(epoch(), &[DebugAction::Pause, DebugAction::Resume], ());
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("batch")
        };
        queue.store(batch.begin(|_, _| probe(true)).unwrap());
        queue.clear();
        assert!(!queue.pending());
        assert!(matches!(queue.poll(), DrainEvent::Idle));

        let mut slot = Some(probe(true));
        assert!(poll_flight(&mut slot).is_pending());
        assert!(slot.is_some());
        assert_eq!(poll_flight(&mut slot), Poll::Ready(Some(7)));
        assert!(slot.is_none());
        assert_eq!(poll_flight(&mut slot), Poll::Ready(None));
    }

    #[test]
    fn pause_prefix_stays_with_following_writes_until_the_next_pause() {
        let mut queue = DrainQueue::<_, Probe>::default();
        queue.enqueue(
            epoch(),
            &[
                DebugAction::Pause,
                DebugAction::StepInstr,
                DebugAction::RegisterWrite(9),
                DebugAction::Resume,
            ],
            (),
        );
        assert_eq!(queue.queued_len(), 2);
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("pause and write batch")
        };
        assert!(matches!(
            batch.actions.as_slice(),
            [
                DebugAction::Pause,
                DebugAction::StepInstr,
                DebugAction::RegisterWrite(9)
            ]
        ));
        queue.store(batch.begin(|_, _| probe(false)).unwrap());
        assert!(matches!(queue.poll(), DrainEvent::Done(7, ())));
        let DrainEvent::Start(batch) = queue.poll() else {
            panic!("resume batch")
        };
        assert!(matches!(batch.actions.as_slice(), [DebugAction::Resume]));
    }

    #[test]
    fn imageless_completion_is_distinct_from_an_empty_flight_slot() {
        struct Imageless;
        impl Flight for Imageless {
            type Output = Option<usize>;

            fn poll(&mut self) -> Poll<Self::Output> {
                Poll::Ready(None)
            }
        }
        let mut slot = Some(Imageless);
        assert_eq!(poll_flight(&mut slot), Poll::Ready(Some(None)));
        assert_eq!(poll_flight(&mut slot), Poll::Ready(None));
    }

    #[test]
    #[should_panic(expected = "cannot replace a pending debugger drain")]
    fn a_pending_flight_cannot_be_silently_replaced() {
        let mut queue = DrainQueue::<_, Probe>::default();
        for _ in 0..2 {
            let batch = DrainBatch {
                epoch: epoch(),
                actions: vec![],
                snapshot: (),
            };
            queue.store(batch.begin(|_, _| probe(true)).unwrap());
        }
    }
}
