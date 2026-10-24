//! Single-owner error-dialog state machine (host-testable).
//! The shell `ModalQueue` stays the only queue gate.
use nerust_gui_shell::session::osd::ModalQueue;

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
        queue: &mut ModalQueue,
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
        queue: &mut ModalQueue,
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
        queue: &mut ModalQueue,
        screen: Screen,
    ) -> DismissTransition {
        const IDLE: DismissTransition = DismissTransition::Idle;
        if self.current.is_none() {
            return IDLE;
        }
        self.current_id = None;
        if matches!(screen, Screen::SettingsOpen | Screen::Suspended) {
            // Preempted, never acked: hand the batch back so a later take re-renders.
            // Requeue also releases the gate; modal close would drop unacked lines.
            let _ = self.current.take();
            queue.requeue_showing();
            return IDLE;
        }
        self.current = None;
        queue.modal_closed();
        if self.auto_paused && queue.modal_pending() == 0 {
            self.auto_paused = false;
            return DismissTransition::ResumeAndReshow;
        }
        DismissTransition::Reshow
    }

    /// Activity suspend kills transient UI: release the gate and hand the
    /// unacked head back structured; the pause flag survives for the resume path.
    pub(crate) fn on_suspended(&mut self, queue: &mut ModalQueue) {
        self.current_id = None;
        let _ = self.current.take();
        queue.requeue_showing();
    }

    /// Settings close flushes the queue; an orphaned auto-pause resumes
    /// only when no dialog came up to own it.
    pub(crate) fn on_settings_closed(
        &mut self,
        queue: &mut ModalQueue,
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
        manager.modal_queue().queue_modal(detail.to_string());
        manager
    }

    #[test]
    fn old_ack_cannot_close_next_dialog_or_release_pause() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("first");
        manager.modal_queue().queue_modal("second".into());
        let screen = Screen::Foreground { can_pause: true };
        controller.poll_show(manager.modal_queue(), screen).unwrap();
        let old = controller.current_id().unwrap();
        assert_eq!(
            controller.on_dismissed_for(old, manager.modal_queue(), screen),
            DismissTransition::Reshow
        );
        assert_eq!(
            controller.poll_show(manager.modal_queue(), screen),
            Some(ShowTransition::Present { auto_pause: false })
        );
        let live = controller.current_id().unwrap();
        assert_ne!(old, live);
        assert_eq!(
            controller.on_dismissed_for(old, manager.modal_queue(), screen),
            DismissTransition::Idle
        );
        assert_eq!(controller.current_detail(), Some("second"));
        assert!(controller.auto_paused());
        assert_eq!(
            controller.on_dismissed_for(live, manager.modal_queue(), screen),
            DismissTransition::ResumeAndReshow
        );
    }

    #[test]
    fn stale_ack_after_suspend_cannot_release_replayed_modal() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("unacknowledged");
        let screen = Screen::Foreground { can_pause: true };
        controller.poll_show(manager.modal_queue(), screen).unwrap();
        let old = controller.current_id().unwrap();
        controller.on_suspended(manager.modal_queue());
        controller
            .poll_show(
                manager.modal_queue(),
                Screen::Foreground { can_pause: false },
            )
            .unwrap();
        assert_eq!(
            controller.on_dismissed_for(old, manager.modal_queue(), screen),
            DismissTransition::Idle
        );
        assert_eq!(controller.current_detail(), Some("unacknowledged"));
        assert!(controller.auto_paused());
        let live = controller.current_id().unwrap();
        assert_eq!(
            controller.on_dismissed_for(live, manager.modal_queue(), screen),
            DismissTransition::ResumeAndReshow
        );
    }

    #[test]
    fn poll_show_gates_on_resume_showing_settings_and_queue() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("boom");
        assert_eq!(
            controller.poll_show(manager.modal_queue(), Screen::show_gate(false, false, true)),
            None
        );
        assert_eq!(
            controller.poll_show(manager.modal_queue(), Screen::show_gate(true, true, true)),
            None
        );
        assert_eq!(
            controller.poll_show(manager.modal_queue(), Screen::show_gate(true, false, true)),
            Some(ShowTransition::Present { auto_pause: true })
        );
        assert_eq!(controller.current_detail(), Some("boom"));
        // Gate is open: a second poll (or a dismiss race) shows nothing.
        assert_eq!(
            controller.poll_show(manager.modal_queue(), Screen::show_gate(true, false, true)),
            None
        );

        let mut empty = OsdManager::new();
        let mut fresh = ErrorDialogController::new();
        assert_eq!(
            fresh.poll_show(empty.modal_queue(), Screen::show_gate(true, false, true)),
            None
        );
        assert_eq!(fresh.current_detail(), None);
    }

    #[test]
    fn poll_show_moves_batch_and_records_auto_pause() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("moved");
        let transition = controller
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, false))
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
        manager.modal_queue().queue_modal("second".to_string());
        controller
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, true))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(manager.modal_queue(), Screen::show_gate(true, false, false));
        assert_eq!(transition, DismissTransition::Reshow);
        assert!(controller.auto_paused());
        // Next batch is still queued behind the released gate.
        let next = controller
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, true))
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
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, false))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(manager.modal_queue(), Screen::show_gate(true, false, false));
        assert_eq!(transition, DismissTransition::Reshow);
    }

    #[test]
    fn dismiss_while_settings_open_requeues_head_and_keeps_pause() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("preempted");
        controller
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, true))
            .expect("batch queued");
        let transition =
            controller.on_dismissed(manager.modal_queue(), Screen::show_gate(true, true, false));
        assert_eq!(transition, DismissTransition::Idle);
        assert!(controller.auto_paused());
        assert_eq!(manager.modal_pending(), 1);
        // Settings close replays the same head in display order.
        let replayed = controller
            .on_settings_closed(manager.modal_queue(), Screen::show_gate(true, false, true));
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
        manager.modal_queue().queue_modal("tail".to_string());
        controller
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, true))
            .expect("batch queued");
        controller.on_suspended(manager.modal_queue());
        assert!(controller.auto_paused());
        assert_eq!(manager.modal_pending(), 2);
        // Late dismiss after the suspend requeue: no double requeue.
        let transition = controller.on_dismissed(
            manager.modal_queue(),
            Screen::show_gate(false, false, false),
        );
        assert_eq!(transition, DismissTransition::Idle);
        assert_eq!(manager.modal_pending(), 2);
        let replayed = controller
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, false))
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
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, true))
            .expect("batch queued");
        let transition = controller.on_dismissed(
            manager.modal_queue(),
            Screen::show_gate(false, false, false),
        );
        assert_eq!(transition, DismissTransition::Idle);
        assert!(controller.auto_paused());
        // Requeue exactly once even if the native suspend follows the callback.
        controller.on_suspended(manager.modal_queue());
        assert_eq!(manager.modal_pending(), 1);
        assert!(controller.auto_paused());
    }

    #[test]
    fn duplicate_dismiss_is_idempotent() {
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("once");
        controller
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, false))
            .expect("batch queued");
        controller.on_dismissed(manager.modal_queue(), Screen::show_gate(true, false, false));
        let again =
            controller.on_dismissed(manager.modal_queue(), Screen::show_gate(true, false, false));
        assert_eq!(again, DismissTransition::Idle);
        assert_eq!(manager.modal_pending(), 0);
    }

    #[test]
    fn settings_close_resumes_orphaned_auto_pause_only_when_idle() {
        // Orphan: paused for a preempted dialog, queue drained elsewhere.
        let mut controller = ErrorDialogController::new();
        let mut manager = queued("preempted");
        controller
            .poll_show(manager.modal_queue(), Screen::show_gate(true, false, true))
            .expect("batch queued");
        controller.on_dismissed(manager.modal_queue(), Screen::show_gate(true, true, false));
        assert_eq!(manager.take_modal_batch(), Some("preempted".to_string()));
        manager.modal_closed();
        assert_eq!(
            controller
                .on_settings_closed(manager.modal_queue(), Screen::show_gate(true, false, true)),
            SettingsClosedTransition::Resume
        );
        assert!(!controller.auto_paused());

        // A dialog already up owns the pause: stay idle.
        let mut showing = ErrorDialogController::new();
        let mut busy = queued("up");
        showing
            .poll_show(busy.modal_queue(), Screen::show_gate(true, false, true))
            .expect("batch queued");
        assert_eq!(
            showing.on_settings_closed(busy.modal_queue(), Screen::show_gate(true, false, true)),
            SettingsClosedTransition::Idle
        );
        assert!(showing.auto_paused());
    }

    /// An interrupted batch redelivers under the settings at redelivery time.
    #[test]
    fn interrupted_batch_applies_new_language_and_level_on_redelivery() {
        use nerust_gui_shell::session::notification::{Message, NotificationLevel, UiNotification};
        use nerust_gui_shell::session::osd::{FrameChannel, OsdManager, TransientPolicy};
        use nerust_settings_core::model::language::AppLanguage;
        use std::time::Instant;

        fn policy(warning_enabled: bool) -> TransientPolicy {
            TransientPolicy {
                info_enabled: true,
                warning_enabled,
                error_enabled: true,
                info_duration_ms: 2_000,
                warning_duration_ms: 2_000,
                info_rgb: (0, 0, 0),
                warning_rgb: (0, 0, 0),
                error_rgb: (200, 0, 0),
                error_modal: false,
            }
        }
        fn notify() -> UiNotification {
            UiNotification {
                level: NotificationLevel::Warning,
                message: Message::persistence_unavailable(),
                count: 1,
                seq: 0,
                at: std::time::UNIX_EPOCH,
            }
        }

        // English warning shown, then preempted by settings.
        let now = Instant::now();
        let mut controller = ErrorDialogController::new();
        let mut manager = OsdManager::new();
        manager.push_all_with_policy(
            vec![notify()],
            &policy(true),
            AppLanguage::English,
            now,
            FrameChannel::Unpresented,
        );
        assert!(manager.pump_modal());
        let screen = Screen::Foreground { can_pause: true };
        controller
            .poll_show(manager.modal_queue(), screen)
            .expect("batch queued");
        assert_eq!(controller.current_detail(), Some("• Saves unavailable"));
        controller.on_dismissed(manager.modal_queue(), Screen::show_gate(true, true, false));
        // Japanese, warnings still on: redelivery translates.
        manager.push_all_with_policy(
            Vec::new(),
            &policy(true),
            AppLanguage::Japanese,
            now,
            FrameChannel::Composited,
        );
        let replayed = controller
            .on_settings_closed(manager.modal_queue(), Screen::show_gate(true, false, false));
        assert_eq!(
            replayed,
            SettingsClosedTransition::Show { auto_pause: false }
        );
        assert!(
            controller
                .current_detail()
                .is_some_and(|detail| detail.contains("保存機能が利用できません")),
            "{:?}",
            controller.current_detail()
        );

        // Japanese, warnings off: the stale warning never reshows, and the
        // orphaned auto-pause releases (no dialog owns it anymore).
        let mut controller = ErrorDialogController::new();
        let mut manager = OsdManager::new();
        manager.push_all_with_policy(
            vec![notify()],
            &policy(true),
            AppLanguage::English,
            now,
            FrameChannel::Unpresented,
        );
        assert!(manager.pump_modal());
        controller
            .poll_show(manager.modal_queue(), screen)
            .expect("batch queued");
        controller.on_dismissed(manager.modal_queue(), Screen::show_gate(true, true, false));
        manager.push_all_with_policy(
            Vec::new(),
            &policy(false),
            AppLanguage::Japanese,
            now,
            FrameChannel::Composited,
        );
        assert_eq!(
            controller
                .on_settings_closed(manager.modal_queue(), Screen::show_gate(true, false, true)),
            SettingsClosedTransition::Resume
        );
        assert!(!controller.auto_paused());
        assert_eq!(controller.current_detail(), None);
    }
}
