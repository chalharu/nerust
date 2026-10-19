//! Single-owner error-dialog state machine (host-testable).
//!
//! The shell `ModalBox` stays the only queue gate; this controller owns
//! the frontend side (`showing`/`current`/`auto_paused`) so every JNI
//! call site is a thin adapter around it.
use nerust_gui_shell::session::osd::{ModalSink, ModalSource, OsdManager};

/// JNI handoff for a dialog that should be presented now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ShowTransition {
    pub auto_pause: bool,
}

/// Follow-up after a dialog dismiss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DismissTransition {
    pub resume: bool,
    pub reshow: bool,
}

/// Follow-up after the settings dialog closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsClosedTransition {
    Show { auto_pause: bool },
    Resume,
    Idle,
}

/// Ambient screen + session facts a dialog decision needs.
/// Bundled so call sites cannot transpose bare bools.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScreenState {
    pub resumed: bool,
    pub settings_open: bool,
    pub can_pause: bool,
}

impl ScreenState {
    pub(crate) fn new(resumed: bool, settings_open: bool, can_pause: bool) -> Self {
        Self {
            resumed,
            settings_open,
            can_pause,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ErrorDialogController {
    showing: bool,
    current: Option<String>,
    auto_paused: bool,
}

impl ErrorDialogController {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Detail currently presented (or requeued); the JNI call borrows it.
    pub(crate) fn current_detail(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Whether the session is paused on behalf of the error dialog.
    pub(crate) fn auto_paused(&self) -> bool {
        self.auto_paused
    }

    /// Take the next queued batch for display, unless suspended, already
    /// showing, or settings owns the screen. Moves the batch into the
    /// controller; the JNI adapter reads it via `current_detail`.
    pub(crate) fn poll_show(
        &mut self,
        queue: &mut impl ModalSource,
        screen: ScreenState,
    ) -> Option<ShowTransition> {
        if !screen.resumed || self.showing || screen.settings_open {
            return None;
        }
        let detail = queue.take_modal_batch()?;
        self.current = Some(detail);
        self.showing = true;
        if screen.can_pause {
            self.auto_paused = true;
        }
        Some(ShowTransition {
            auto_pause: screen.can_pause,
        })
    }

    /// Dismiss hook from the dialog. Resumes only our own auto-pause,
    /// then asks for the next batch. A stale duplicate (suspend already
    /// requeued, or a doubled failure synthesis) is a no-op: the shell
    /// gate release is idempotent and state is untouched.
    pub(crate) fn on_dismissed(
        &mut self,
        queue: &mut (impl ModalSink + ModalSource),
        screen: ScreenState,
    ) -> DismissTransition {
        const IDLE: DismissTransition = DismissTransition {
            resume: false,
            reshow: false,
        };
        if !self.showing && self.current.is_none() {
            queue.modal_closed();
            return IDLE;
        }
        self.showing = false;
        queue.modal_closed();
        if screen.settings_open {
            // Preempted, never acked: requeue the head so the ack
            // guarantee survives the replacement in display order.
            if let Some(detail) = self.current.take() {
                queue.requeue_front(detail);
            }
            return IDLE;
        }
        self.current = None;
        // A dismiss racing suspend resumes nothing: the resume path
        // replays from the preserved pause flag and queue instead, and
        // suspend owns the requeue (see `on_suspended`).
        if !screen.resumed {
            return IDLE;
        }
        if self.auto_paused {
            self.auto_paused = false;
            return DismissTransition {
                resume: true,
                reshow: true,
            };
        }
        DismissTransition {
            resume: false,
            reshow: true,
        }
    }

    /// Activity suspend kills transient UI: release the gate and requeue
    /// the unacked head; the pause flag survives for the resume path.
    pub(crate) fn on_suspended(&mut self, queue: &mut (impl ModalSink + ModalSource)) {
        queue.modal_closed();
        if let Some(detail) = self.current.take() {
            queue.requeue_front(detail);
        }
        self.showing = false;
    }

    /// Settings close flushes the queue; an orphaned auto-pause resumes
    /// only when no dialog came up to own it.
    pub(crate) fn on_settings_closed(
        &mut self,
        queue: &mut (impl ModalSink + ModalSource),
        screen: ScreenState,
    ) -> SettingsClosedTransition {
        let unshielded = ScreenState::new(screen.resumed, false, screen.can_pause);
        if let Some(transition) = self.poll_show(queue, unshielded) {
            return SettingsClosedTransition::Show {
                auto_pause: transition.auto_pause,
            };
        }
        if !self.showing && queue.modal_pending() == 0 && self.auto_paused {
            self.auto_paused = false;
            return SettingsClosedTransition::Resume;
        }
        SettingsClosedTransition::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(detail: &str) -> OsdManager {
        let mut manager = OsdManager::new();
        manager.queue_modal(detail.to_string());
        manager
    }

    #[test]
    fn poll_show_gates_on_resume_showing_settings_and_queue() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("boom");
        assert_eq!(
            controller.poll_show(&mut manager, ScreenState::new(false, false, true)),
            None
        );
        assert_eq!(
            controller.poll_show(&mut manager, ScreenState::new(true, true, true)),
            None
        );
        assert_eq!(
            controller.poll_show(&mut manager, ScreenState::new(true, false, true)),
            Some(ShowTransition { auto_pause: true })
        );
        assert_eq!(controller.current_detail(), Some("boom"));
        // Gate is open: a second poll (or a dismiss race) shows nothing.
        assert_eq!(
            controller.poll_show(&mut manager, ScreenState::new(true, false, true)),
            None
        );

        let mut empty = OsdManager::new();
        let mut fresh = ErrorDialogController::new();
        assert_eq!(
            fresh.poll_show(&mut empty, ScreenState::new(true, false, true)),
            None
        );
        assert_eq!(fresh.current_detail(), None);
    }

    #[test]
    fn poll_show_moves_batch_and_records_auto_pause() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("moved");
        let transition = controller
            .poll_show(&mut manager, ScreenState::new(true, false, false))
            .expect("batch queued");
        assert!(!transition.auto_pause);
        assert!(!controller.auto_paused());
        // Moved, not cloned: the queue no longer holds the batch.
        assert_eq!(manager.modal_pending(), 0);
        assert_eq!(manager.take_modal_batch(), None);
    }

    #[test]
    fn dismiss_resumes_only_own_auto_pause_then_reshows() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("first");
        manager.queue_modal("second".to_string());
        controller
            .poll_show(&mut manager, ScreenState::new(true, false, true))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(&mut manager, ScreenState::new(true, false, false));
        assert_eq!(
            transition,
            DismissTransition {
                resume: true,
                reshow: true,
            }
        );
        assert!(!controller.auto_paused());
        // Next batch is still queued behind the released gate.
        let next = controller
            .poll_show(&mut manager, ScreenState::new(true, false, true))
            .expect("second batch queued");
        assert!(next.auto_pause);
        assert_eq!(controller.current_detail(), Some("second"));
    }

    #[test]
    fn dismiss_without_auto_pause_reshows_without_resume() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("only");
        controller
            .poll_show(&mut manager, ScreenState::new(true, false, false))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(&mut manager, ScreenState::new(true, false, false));
        assert_eq!(
            transition,
            DismissTransition {
                resume: false,
                reshow: true,
            }
        );
    }

    #[test]
    fn dismiss_while_settings_open_requeues_head_and_keeps_pause() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("preempted");
        controller
            .poll_show(&mut manager, ScreenState::new(true, false, true))
            .expect("batch queued");
        let transition = controller.on_dismissed(&mut manager, ScreenState::new(true, true, false));
        assert_eq!(
            transition,
            DismissTransition {
                resume: false,
                reshow: false,
            }
        );
        assert!(controller.auto_paused());
        assert_eq!(manager.modal_pending(), 1);
        // Settings close replays the same head in display order.
        let replayed =
            controller.on_settings_closed(&mut manager, ScreenState::new(true, false, true));
        assert_eq!(
            replayed,
            SettingsClosedTransition::Show { auto_pause: true }
        );
        assert_eq!(controller.current_detail(), Some("preempted"));
    }

    #[test]
    fn suspend_requeues_head_and_late_dismiss_is_noop() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("head");
        manager.queue_modal("tail".to_string());
        controller
            .poll_show(&mut manager, ScreenState::new(true, false, true))
            .expect("batch queued");
        controller.on_suspended(&mut manager);
        assert!(controller.auto_paused());
        assert_eq!(manager.modal_pending(), 2);
        // Late dismiss after the suspend requeue: no double requeue.
        let transition =
            controller.on_dismissed(&mut manager, ScreenState::new(false, false, false));
        assert_eq!(
            transition,
            DismissTransition {
                resume: false,
                reshow: false,
            }
        );
        assert_eq!(manager.modal_pending(), 2);
        let replayed = controller
            .poll_show(&mut manager, ScreenState::new(true, false, false))
            .expect("head requeued");
        assert!(!replayed.auto_pause);
        assert_eq!(controller.current_detail(), Some("head"));
    }

    #[test]
    fn dismiss_racing_suspend_resumes_nothing_and_keeps_queue() {
        // Suspend has not run yet but the activity is already gone:
        // dropping here is safe because suspend owns the requeue, and a
        // suspend that already ran leaves no current to drop.
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("racing");
        controller
            .poll_show(&mut manager, ScreenState::new(true, false, true))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(&mut manager, ScreenState::new(false, false, false));
        assert_eq!(
            transition,
            DismissTransition {
                resume: false,
                reshow: false,
            }
        );
        assert!(controller.auto_paused());
        // Dismiss consumed the unacked head; suspend finds nothing to
        // requeue but the pause flag still drives the resume path.
        controller.on_suspended(&mut manager);
        assert_eq!(manager.modal_pending(), 0);
        assert!(controller.auto_paused());
    }

    #[test]
    fn duplicate_dismiss_is_idempotent() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("once");
        controller
            .poll_show(&mut manager, ScreenState::new(true, false, false))
            .expect("batch queued");
        controller.on_dismissed(&mut manager, ScreenState::new(true, false, false));
        let again = controller.on_dismissed(&mut manager, ScreenState::new(true, false, false));
        assert_eq!(
            again,
            DismissTransition {
                resume: false,
                reshow: false,
            }
        );
        assert_eq!(manager.modal_pending(), 0);
    }

    #[test]
    fn settings_close_resumes_orphaned_auto_pause_only_when_idle() {
        // Orphan: paused for a preempted dialog, queue drained elsewhere.
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("preempted");
        controller
            .poll_show(&mut manager, ScreenState::new(true, false, true))
            .expect("batch queued");
        controller.on_dismissed(&mut manager, ScreenState::new(true, true, false));
        assert_eq!(manager.take_modal_batch(), Some("preempted".to_string()));
        manager.modal_closed();
        assert_eq!(
            controller.on_settings_closed(&mut manager, ScreenState::new(true, false, true)),
            SettingsClosedTransition::Resume
        );
        assert!(!controller.auto_paused());

        // A dialog already up owns the pause: stay idle.
        let mut showing = ErrorDialogController::new();
        let mut busy = queued("up");
        showing
            .poll_show(&mut busy, ScreenState::new(true, false, true))
            .expect("batch queued");
        assert_eq!(
            showing.on_settings_closed(&mut busy, ScreenState::new(true, false, true)),
            SettingsClosedTransition::Idle
        );
        assert!(showing.auto_paused());
    }
}
