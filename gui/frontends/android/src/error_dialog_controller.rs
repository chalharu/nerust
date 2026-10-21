//! Single-owner error-dialog state machine (host-testable).
//! The shell `ModalBox` stays the only queue gate.
use nerust_gui_shell::session::osd::{ModalSink, ModalSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(transparent)]
pub(crate) struct DialogRequestId(pub(crate) u64);

/// JNI handoff for a dialog that should be presented now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShowTransition {
    Present { auto_pause: bool },
}

/// Follow-up after a dialog dismiss. New arms extend the enum;
/// bool structs cannot grow arms without transposing call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DismissTransition {
    Idle,
    Reshow,
    ResumeAndReshow,
}

/// Follow-up after the settings dialog closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsClosedTransition {
    Show { auto_pause: bool },
    Resume,
    Idle,
}

/// Screen ownership for a dialog decision; invalid combos are unrepresentable.
/// Only the foreground carries pausability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    Suspended,
    SettingsOpen,
    Foreground { can_pause: bool },
}

impl Screen {
    /// Single mapping from ambient flags. Settings owns the screen
    /// over a resumed activity; suspension beats everything.
    pub(crate) fn show_gate(resumed: bool, settings_open: bool, can_pause: bool) -> Self {
        if !resumed {
            Screen::Suspended
        } else if settings_open {
            Screen::SettingsOpen
        } else {
            Screen::Foreground { can_pause }
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ErrorDialogController {
    current: Option<String>,
    auto_paused: bool,
    next_id: u64,
    current_id: Option<DialogRequestId>,
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

    pub(crate) fn current_id(&self) -> Option<DialogRequestId> {
        self.current_id
    }

    pub(crate) fn on_dismissed_for(
        &mut self,
        id: DialogRequestId,
        queue: &mut (impl ModalSink + ModalSource),
        screen: Screen,
    ) -> DismissTransition {
        if self.current_id != Some(id) {
            return DismissTransition::Idle;
        }
        self.on_dismissed(queue, screen)
    }

    /// Whether an error dialog is currently presented.
    /// `current` is the showing state; no second bool can disagree with it.
    pub(crate) fn is_showing(&self) -> bool {
        self.current.is_some()
    }

    /// Take the next queued batch for display, unless suspended, showing,
    /// or settings owns the screen. Moves the batch into the controller.
    pub(crate) fn poll_show(
        &mut self,
        queue: &mut impl ModalSource,
        screen: Screen,
    ) -> Option<ShowTransition> {
        let Screen::Foreground { can_pause } = screen else {
            return None;
        };
        if self.is_showing() {
            return None;
        }
        let detail = queue.take_modal_batch()?;
        self.current = Some(detail);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.current_id = Some(DialogRequestId(self.next_id));
        let auto_pause = can_pause && !self.auto_paused;
        if auto_pause {
            self.auto_paused = true;
        }
        Some(ShowTransition::Present { auto_pause })
    }

    /// Dismiss hook: resumes only our own auto-pause, then asks for the next batch.
    /// Stale duplicates are no-ops.
    pub(crate) fn on_dismissed(
        &mut self,
        queue: &mut (impl ModalSink + ModalSource),
        screen: Screen,
    ) -> DismissTransition {
        const IDLE: DismissTransition = DismissTransition::Idle;
        if self.current.is_none() {
            return IDLE;
        }
        self.current_id = None;
        queue.modal_closed();
        if matches!(screen, Screen::SettingsOpen | Screen::Suspended) {
            // Preempted, never acked: requeue the head so the ack
            // guarantee survives the replacement in display order.
            if let Some(detail) = self.current.take() {
                queue.requeue_front(detail);
            }
            return IDLE;
        }
        self.current = None;
        if self.auto_paused && queue.modal_pending() == 0 {
            self.auto_paused = false;
            return DismissTransition::ResumeAndReshow;
        }
        DismissTransition::Reshow
    }

    /// Activity suspend kills transient UI: release the gate and requeue
    /// the unacked head; the pause flag survives for the resume path.
    pub(crate) fn on_suspended(&mut self, queue: &mut (impl ModalSink + ModalSource)) {
        self.current_id = None;
        queue.modal_closed();
        if let Some(detail) = self.current.take() {
            queue.requeue_front(detail);
        }
    }

    /// Settings close flushes the queue; an orphaned auto-pause resumes
    /// only when no dialog came up to own it.
    pub(crate) fn on_settings_closed(
        &mut self,
        queue: &mut (impl ModalSink + ModalSource),
        screen: Screen,
    ) -> SettingsClosedTransition {
        // Settings just closed: show unless suspended. A stale
        // settings-owned screen shows conservatively without pausing.
        let unshielded = match screen {
            Screen::Suspended => Screen::Suspended,
            Screen::SettingsOpen => Screen::Foreground { can_pause: false },
            Screen::Foreground { can_pause } => Screen::Foreground { can_pause },
        };
        if let Some(ShowTransition::Present { auto_pause }) = self.poll_show(queue, unshielded) {
            return SettingsClosedTransition::Show { auto_pause };
        }
        if matches!(screen, Screen::Foreground { .. })
            && !self.is_showing()
            && queue.modal_pending() == 0
            && self.auto_paused
        {
            self.auto_paused = false;
            return SettingsClosedTransition::Resume;
        }
        SettingsClosedTransition::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_shell::session::osd::OsdManager;

    fn queued(detail: &str) -> OsdManager {
        let mut manager = OsdManager::new();
        manager.queue_modal(detail.to_string());
        manager
    }

    #[test]
    fn old_ack_cannot_close_next_dialog_or_release_pause() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("first");
        manager.queue_modal("second".into());
        let screen = Screen::Foreground { can_pause: true };
        controller.poll_show(&mut manager, screen).unwrap();
        let old = controller.current_id().unwrap();
        assert_eq!(
            controller.on_dismissed_for(old, &mut manager, screen),
            DismissTransition::Reshow
        );
        assert_eq!(
            controller.poll_show(&mut manager, screen),
            Some(ShowTransition::Present { auto_pause: false })
        );
        let live = controller.current_id().unwrap();
        assert_ne!(old, live);
        assert_eq!(
            controller.on_dismissed_for(old, &mut manager, screen),
            DismissTransition::Idle
        );
        assert_eq!(controller.current_detail(), Some("second"));
        assert!(controller.auto_paused());
        assert_eq!(
            controller.on_dismissed_for(live, &mut manager, screen),
            DismissTransition::ResumeAndReshow
        );
    }

    #[test]
    fn stale_ack_after_suspend_cannot_release_replayed_modal() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("unacknowledged");
        let screen = Screen::Foreground { can_pause: true };
        controller.poll_show(&mut manager, screen).unwrap();
        let old = controller.current_id().unwrap();
        controller.on_suspended(&mut manager);
        controller
            .poll_show(&mut manager, Screen::Foreground { can_pause: false })
            .unwrap();
        assert_eq!(
            controller.on_dismissed_for(old, &mut manager, screen),
            DismissTransition::Idle
        );
        assert_eq!(controller.current_detail(), Some("unacknowledged"));
        assert!(controller.auto_paused());
        let live = controller.current_id().unwrap();
        assert_eq!(
            controller.on_dismissed_for(live, &mut manager, screen),
            DismissTransition::ResumeAndReshow
        );
    }

    #[test]
    fn poll_show_gates_on_resume_showing_settings_and_queue() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("boom");
        assert_eq!(
            controller.poll_show(&mut manager, Screen::show_gate(false, false, true)),
            None
        );
        assert_eq!(
            controller.poll_show(&mut manager, Screen::show_gate(true, true, true)),
            None
        );
        assert_eq!(
            controller.poll_show(&mut manager, Screen::show_gate(true, false, true)),
            Some(ShowTransition::Present { auto_pause: true })
        );
        assert_eq!(controller.current_detail(), Some("boom"));
        // Gate is open: a second poll (or a dismiss race) shows nothing.
        assert_eq!(
            controller.poll_show(&mut manager, Screen::show_gate(true, false, true)),
            None
        );

        let mut empty = OsdManager::new();
        let mut fresh = ErrorDialogController::new();
        assert_eq!(
            fresh.poll_show(&mut empty, Screen::show_gate(true, false, true)),
            None
        );
        assert_eq!(fresh.current_detail(), None);
    }

    #[test]
    fn poll_show_moves_batch_and_records_auto_pause() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("moved");
        let transition = controller
            .poll_show(&mut manager, Screen::show_gate(true, false, false))
            .expect("batch queued");
        assert!(matches!(
            transition,
            ShowTransition::Present { auto_pause: false }
        ));
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
            .poll_show(&mut manager, Screen::show_gate(true, false, true))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(&mut manager, Screen::show_gate(true, false, false));
        assert_eq!(transition, DismissTransition::Reshow);
        assert!(controller.auto_paused());
        // Next batch is still queued behind the released gate.
        let next = controller
            .poll_show(&mut manager, Screen::show_gate(true, false, true))
            .expect("second batch queued");
        assert!(matches!(
            next,
            ShowTransition::Present { auto_pause: false }
        ));
        assert_eq!(controller.current_detail(), Some("second"));
    }

    #[test]
    fn dismiss_without_auto_pause_reshows_without_resume() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("only");
        controller
            .poll_show(&mut manager, Screen::show_gate(true, false, false))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(&mut manager, Screen::show_gate(true, false, false));
        assert_eq!(transition, DismissTransition::Reshow);
    }

    #[test]
    fn dismiss_while_settings_open_requeues_head_and_keeps_pause() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("preempted");
        controller
            .poll_show(&mut manager, Screen::show_gate(true, false, true))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(&mut manager, Screen::show_gate(true, true, false));
        assert_eq!(transition, DismissTransition::Idle);
        assert!(controller.auto_paused());
        assert_eq!(manager.modal_pending(), 1);
        // Settings close replays the same head in display order.
        let replayed =
            controller.on_settings_closed(&mut manager, Screen::show_gate(true, false, true));
        assert_eq!(
            replayed,
            SettingsClosedTransition::Show { auto_pause: false }
        );
        assert_eq!(controller.current_detail(), Some("preempted"));
    }

    #[test]
    fn suspend_requeues_head_and_late_dismiss_is_noop() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("head");
        manager.queue_modal("tail".to_string());
        controller
            .poll_show(&mut manager, Screen::show_gate(true, false, true))
            .expect("batch queued");
        controller.on_suspended(&mut manager);
        assert!(controller.auto_paused());
        assert_eq!(manager.modal_pending(), 2);
        // Late dismiss after the suspend requeue: no double requeue.
        let transition =
            controller.on_dismissed(&mut manager, Screen::show_gate(false, false, false));
        assert_eq!(transition, DismissTransition::Idle);
        assert_eq!(manager.modal_pending(), 2);
        let replayed = controller
            .poll_show(&mut manager, Screen::show_gate(true, false, false))
            .expect("head requeued");
        assert!(matches!(
            replayed,
            ShowTransition::Present { auto_pause: false }
        ));
        assert_eq!(controller.current_detail(), Some("head"));
    }

    #[test]
    fn dismiss_racing_suspend_resumes_nothing_and_keeps_queue() {
        // Suspend has not run yet but the activity is already gone.
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("racing");
        controller
            .poll_show(&mut manager, Screen::show_gate(true, false, true))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(&mut manager, Screen::show_gate(false, false, false));
        assert_eq!(transition, DismissTransition::Idle);
        assert!(controller.auto_paused());
        // Requeue exactly once even if the native suspend follows the callback.
        controller.on_suspended(&mut manager);
        assert_eq!(manager.modal_pending(), 1);
        assert!(controller.auto_paused());
    }

    #[test]
    fn duplicate_dismiss_is_idempotent() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("once");
        controller
            .poll_show(&mut manager, Screen::show_gate(true, false, false))
            .expect("batch queued");
        controller.on_dismissed(&mut manager, Screen::show_gate(true, false, false));
        let again = controller.on_dismissed(&mut manager, Screen::show_gate(true, false, false));
        assert_eq!(again, DismissTransition::Idle);
        assert_eq!(manager.modal_pending(), 0);
    }

    #[test]
    fn settings_close_resumes_orphaned_auto_pause_only_when_idle() {
        // Orphan: paused for a preempted dialog, queue drained elsewhere.
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("preempted");
        controller
            .poll_show(&mut manager, Screen::show_gate(true, false, true))
            .expect("batch queued");
        controller.on_dismissed(&mut manager, Screen::show_gate(true, true, false));
        assert_eq!(manager.take_modal_batch(), Some("preempted".to_string()));
        manager.modal_closed();
        assert_eq!(
            controller.on_settings_closed(&mut manager, Screen::show_gate(true, false, true)),
            SettingsClosedTransition::Resume
        );
        assert!(!controller.auto_paused());

        // A dialog already up owns the pause: stay idle.
        let mut showing = ErrorDialogController::new();
        let mut busy = queued("up");
        showing
            .poll_show(&mut busy, Screen::show_gate(true, false, true))
            .expect("batch queued");
        assert_eq!(
            showing.on_settings_closed(&mut busy, Screen::show_gate(true, false, true)),
            SettingsClosedTransition::Idle
        );
        assert!(showing.auto_paused());
    }
}
