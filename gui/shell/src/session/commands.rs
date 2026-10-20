pub use crate::session::error::SlotOpFailure;

use crate::session::{SessionHandle, osd::TickMetrics};

/// Handler behind one registration-table entry: the command plus the pass tick in.
pub type CommandHandler = fn(
    &mut SessionHandle,
    SessionCommand,
    &TickMetrics,
) -> Result<SessionCommandOutcome, super::SessionError>;

/// Registration-table dispatch: the sole mapping from command to handler.
/// Implemented once in `command_exec`; all dispatch goes through it.
pub trait SessionCommandHandler {
    /// Handler for this command from the registration table.
    /// `TogglePause` callers resolve through [`TogglePauseDecorator`] first.
    fn handler(self) -> CommandHandler;
}

/// Data-driven command vocabulary: every per-command answer lives on this trait.
/// The executor matches once; [`SessionCommand`] is the sole implementor.
pub trait SessionCmd {
    /// Stable command name for logs and failure messages.
    fn cmd_name(&self) -> &'static str;
    /// Slot carried for failure messages: explicit-slot commands use their own id.
    /// Active-slot commands resolve to `active`; the rest report `None`.
    fn slot_affinity(&self, active: Option<u64>) -> Option<u64>;
    /// Success notification for an executed command. `None` means silence.
    /// Slot failures map to Warning/Error by reason; transport errors become generic Error.
    fn success_notification(
        &self,
        outcome: &SessionCommandOutcome,
        active_slot: Option<u64>,
    ) -> Option<crate::session::notification::NotificationKind>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCommand {
    Pause,
    Resume,
    TogglePause,
    Reset,
    CreateSlot,
    SaveActiveSlotOrNew,
    LoadActiveSlot,
    SelectActiveSlot(u64),
    SaveSlot(u64),
    LoadSlot(u64),
    DeleteSlot(u64),
    SelectNextSlot,
    SelectPreviousSlot,
}

impl SessionCmd for SessionCommand {
    fn cmd_name(&self) -> &'static str {
        match self {
            SessionCommand::Pause => "Pause",
            SessionCommand::Resume => "Resume",
            SessionCommand::TogglePause => "TogglePause",
            SessionCommand::Reset => "Reset",
            SessionCommand::CreateSlot => "CreateSlot",
            SessionCommand::SaveActiveSlotOrNew => "SaveActiveSlotOrNew",
            SessionCommand::LoadActiveSlot => "LoadActiveSlot",
            SessionCommand::SelectActiveSlot(_) => "SelectActiveSlot",
            SessionCommand::SaveSlot(_) => "SaveSlot",
            SessionCommand::LoadSlot(_) => "LoadSlot",
            SessionCommand::DeleteSlot(_) => "DeleteSlot",
            SessionCommand::SelectNextSlot => "SelectNextSlot",
            SessionCommand::SelectPreviousSlot => "SelectPreviousSlot",
        }
    }

    fn slot_affinity(&self, active: Option<u64>) -> Option<u64> {
        match self {
            SessionCommand::SaveSlot(id)
            | SessionCommand::LoadSlot(id)
            | SessionCommand::DeleteSlot(id)
            | SessionCommand::SelectActiveSlot(id) => Some(*id),
            SessionCommand::SaveActiveSlotOrNew | SessionCommand::LoadActiveSlot => active,
            SessionCommand::Pause
            | SessionCommand::Resume
            | SessionCommand::TogglePause
            | SessionCommand::Reset
            | SessionCommand::CreateSlot
            | SessionCommand::SelectNextSlot
            | SessionCommand::SelectPreviousSlot => None,
        }
    }

    fn success_notification(
        &self,
        outcome: &SessionCommandOutcome,
        active_slot: Option<u64>,
    ) -> Option<crate::session::notification::NotificationKind> {
        use crate::session::notification::NotificationKind;
        if let Some(reason) = outcome.slot_failure {
            let slot = self.slot_affinity(active_slot);
            return Some(NotificationKind::SlotFailed { slot, reason });
        }
        if !outcome.executed {
            return None;
        }
        Some(match self {
            SessionCommand::Pause | SessionCommand::TogglePause => NotificationKind::Paused,
            SessionCommand::Resume => NotificationKind::Resumed,
            SessionCommand::Reset => NotificationKind::Reset,
            SessionCommand::CreateSlot => NotificationKind::CreatedSlot,
            SessionCommand::SaveActiveSlotOrNew => {
                NotificationKind::SavedSlot(active_slot.unwrap_or(0))
            }
            SessionCommand::LoadActiveSlot => {
                NotificationKind::LoadedSlot(active_slot.unwrap_or(0))
            }
            SessionCommand::SelectActiveSlot(id) => NotificationKind::SelectedSlot(*id),
            SessionCommand::SelectNextSlot | SessionCommand::SelectPreviousSlot => {
                NotificationKind::SelectedSlot(active_slot.unwrap_or(0))
            }
            SessionCommand::SaveSlot(id) => NotificationKind::SavedSlot(*id),
            SessionCommand::LoadSlot(id) => NotificationKind::LoadedSlot(*id),
            SessionCommand::DeleteSlot(id) => NotificationKind::DeletedSlot(*id),
        })
    }
}

impl SessionCommand {
    /// Slot carried for failure messages: explicit-slot commands use their own id.
    /// Active-slot commands resolve to `active`; the rest report `None`.
    pub fn associated_slot(&self, active: Option<u64>) -> Option<u64> {
        self.slot_affinity(active)
    }

    /// Single mapping from outcome to notification; all entry points delegate here.
    pub fn notification_for(
        &self,
        outcome: &SessionCommandOutcome,
        active_slot: Option<u64>,
    ) -> Option<crate::session::notification::NotificationKind> {
        self.success_notification(outcome, active_slot)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionCommandOutcome {
    pub executed: bool,
    pub needs_redraw: bool,
    /// Why a slot save/load did not execute.
    /// `None` on success and for commands that cannot fail this way.
    pub slot_failure: Option<SlotOpFailure>,
}

/// Single mapping from command outcome to notification.
pub fn outcome_notification(
    command: SessionCommand,
    outcome: &SessionCommandOutcome,
    active_slot: Option<u64>,
) -> Option<crate::session::notification::NotificationKind> {
    notification_for(command, outcome, active_slot)
}

/// Single source mapping command outcomes to notifications.
pub fn notification_for(
    command: SessionCommand,
    outcome: &SessionCommandOutcome,
    active_slot: Option<u64>,
) -> Option<crate::session::notification::NotificationKind> {
    command.success_notification(outcome, active_slot)
}

/// Backwards-compatible batch form kept for callers migrating off
/// the 0-or-1 `Vec`. Prefer [`outcome_notification`].
pub fn outcome_notifications(
    command: SessionCommand,
    outcome: &SessionCommandOutcome,
    active_slot: Option<u64>,
) -> Vec<crate::session::notification::NotificationKind> {
    outcome_notification(command, outcome, active_slot)
        .into_iter()
        .collect()
}

/// Decorator resolving `TogglePause` to the steady arm entered from the pass tick.
/// Both dispatch and notification resolve through here, never inline.
pub struct TogglePauseDecorator;

impl TogglePauseDecorator {
    /// Resolve `TogglePause` against the pass tick; other commands pass through.
    pub fn resolve(
        command: SessionCommand,
        tick: &crate::session::osd::TickMetrics,
    ) -> SessionCommand {
        match (command, tick.paused) {
            (SessionCommand::TogglePause, false) => SessionCommand::Pause,
            (SessionCommand::TogglePause, true) => SessionCommand::Resume,
            (other, _) => other,
        }
    }
}

/// Resolve TogglePause to the steady arm entered from `was_paused`.
pub fn resolve_toggle(command: SessionCommand, was_paused: bool) -> SessionCommand {
    TogglePauseDecorator::resolve(
        command,
        &crate::session::osd::TickMetrics {
            paused: was_paused,
            ..crate::session::osd::TickMetrics::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::session::commands::{
        SessionCmd as _, SessionCommand, SessionCommandOutcome, outcome_notifications,
        resolve_toggle,
    };
    use crate::session::error::SlotOpFailure;
    use crate::session::notification::{NotificationKind, NotificationLevel};

    #[test]
    fn slot_failure_maps_to_warning_or_error() {
        let warn = outcome_notifications(
            SessionCommand::LoadSlot(2),
            &SessionCommandOutcome {
                executed: false,
                needs_redraw: false,
                slot_failure: Some(SlotOpFailure::Missing),
            },
            None,
        );
        assert_eq!(
            warn,
            vec![NotificationKind::SlotFailed {
                slot: Some(2),
                reason: SlotOpFailure::Missing,
            }]
        );
        assert_eq!(warn[0].level(), NotificationLevel::Warning);
        let err = outcome_notifications(
            SessionCommand::SaveSlot(1),
            &SessionCommandOutcome {
                executed: false,
                needs_redraw: false,
                slot_failure: Some(SlotOpFailure::Storage),
            },
            None,
        );
        assert_eq!(err[0].level(), NotificationLevel::Error);
    }

    #[test]
    fn toggle_resolves_to_the_entered_state() {
        use crate::session::commands::SessionCommand as C;
        assert_eq!(resolve_toggle(C::TogglePause, false), C::Pause);
        assert_eq!(resolve_toggle(C::TogglePause, true), C::Resume);
        assert_eq!(resolve_toggle(C::Resume, true), C::Resume);
        // The resolved arm drives the notification: toggling out of
        // pause reads Resumed, never Paused.
        let ok = SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        };
        assert_eq!(
            outcome_notifications(resolve_toggle(C::TogglePause, true), &ok, None),
            [NotificationKind::Resumed]
        );
    }

    #[test]
    fn outcome_success_maps_each_command() {
        let ok = SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        };
        let cases = [
            (SessionCommand::Pause, NotificationKind::Paused),
            (SessionCommand::Resume, NotificationKind::Resumed),
            (SessionCommand::Reset, NotificationKind::Reset),
            (SessionCommand::CreateSlot, NotificationKind::CreatedSlot),
            (
                SessionCommand::SelectNextSlot,
                NotificationKind::SelectedSlot(0),
            ),
            (
                SessionCommand::SelectPreviousSlot,
                NotificationKind::SelectedSlot(0),
            ),
            (SessionCommand::SaveSlot(2), NotificationKind::SavedSlot(2)),
            (SessionCommand::LoadSlot(2), NotificationKind::LoadedSlot(2)),
            (
                SessionCommand::DeleteSlot(2),
                NotificationKind::DeletedSlot(2),
            ),
            (
                SessionCommand::LoadActiveSlot,
                NotificationKind::LoadedSlot(0),
            ),
        ];
        for (command, expected) in cases {
            assert_eq!(outcome_notifications(command, &ok, Some(0)), [expected]);
        }
        // Unexecuted commands notify nothing.
        let skipped = SessionCommandOutcome {
            executed: false,
            needs_redraw: false,
            slot_failure: None,
        };
        assert!(outcome_notifications(SessionCommand::Pause, &skipped, Some(0)).is_empty());
    }

    #[test]
    fn associated_slot_covers_every_command() {
        use crate::session::commands::SessionCommand as C;
        for (command, expected) in [
            (C::SaveSlot(4), Some(4)),
            (C::LoadSlot(4), Some(4)),
            (C::DeleteSlot(4), Some(4)),
            (C::SelectActiveSlot(4), Some(4)),
            (C::SaveActiveSlotOrNew, Some(7)),
            (C::LoadActiveSlot, Some(7)),
            (C::Pause, None),
            (C::Resume, None),
            (C::TogglePause, None),
            (C::Reset, None),
            (C::CreateSlot, None),
            (C::SelectNextSlot, None),
            (C::SelectPreviousSlot, None),
        ] {
            assert_eq!(command.associated_slot(Some(7)), expected);
        }
        // Active-slot commands without an active slot report none.
        assert_eq!(C::SaveActiveSlotOrNew.associated_slot(None), None);
        assert_eq!(C::LoadActiveSlot.associated_slot(None), None);
        // Failure routing follows the slot: active-slot ops carry it.
        let failed = SessionCommandOutcome {
            executed: false,
            needs_redraw: false,
            slot_failure: Some(SlotOpFailure::Missing),
        };
        assert_eq!(
            outcome_notifications(C::SaveActiveSlotOrNew, &failed, Some(7)),
            [NotificationKind::SlotFailed {
                slot: Some(7),
                reason: SlotOpFailure::Missing,
            }]
        );
        assert_eq!(
            outcome_notifications(C::Pause, &failed, Some(7)),
            [NotificationKind::SlotFailed {
                slot: None,
                reason: SlotOpFailure::Missing,
            }]
        );
    }

    #[test]
    fn session_cmd_trait_is_the_single_source() {
        use crate::session::commands::SessionCommand as C;
        // Names cover every command exactly once.
        for command in [
            C::Pause,
            C::Resume,
            C::TogglePause,
            C::Reset,
            C::CreateSlot,
            C::SaveActiveSlotOrNew,
            C::LoadActiveSlot,
            C::SelectActiveSlot(1),
            C::SaveSlot(1),
            C::LoadSlot(1),
            C::DeleteSlot(1),
            C::SelectNextSlot,
            C::SelectPreviousSlot,
        ] {
            assert!(!command.cmd_name().is_empty());
            // Free functions delegate to the trait.
            assert_eq!(
                command.associated_slot(Some(7)),
                command.slot_affinity(Some(7))
            );
        }
        // Slot affinity matches the legacy expectations.
        assert_eq!(C::SaveSlot(4).slot_affinity(Some(7)), Some(4));
        assert_eq!(C::SaveActiveSlotOrNew.slot_affinity(Some(7)), Some(7));
        assert_eq!(C::SaveActiveSlotOrNew.slot_affinity(None), None);
        assert_eq!(C::Pause.slot_affinity(Some(7)), None);
    }

    #[test]
    fn outcome_success_covers_active_slot_commands() {
        let ok = SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        };
        assert_eq!(
            outcome_notifications(SessionCommand::SaveActiveSlotOrNew, &ok, Some(3)),
            [NotificationKind::SavedSlot(3)]
        );
        assert_eq!(
            outcome_notifications(SessionCommand::SelectActiveSlot(5), &ok, Some(3)),
            [NotificationKind::SelectedSlot(5)]
        );
    }
}

#[cfg(test)]
mod mapping_tests {
    use crate::emu_core::StateFault;
    use crate::session::commands::*;
    use crate::session::error::SlotOpFailure;

    #[test]
    fn notification_for_is_the_single_mapping() {
        let ok = SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        };
        assert_eq!(
            notification_for(SessionCommand::Pause, &ok, None),
            outcome_notification(SessionCommand::Pause, &ok, None)
        );
        assert_eq!(
            SessionCommand::Pause.notification_for(&ok, None),
            outcome_notification(SessionCommand::Pause, &ok, None)
        );
    }

    #[test]
    fn state_faults_map_to_slot_failures_once() {
        use crate::emu_core::state_fault_to_failure;
        assert_eq!(
            state_fault_to_failure(StateFault::Transport),
            SlotOpFailure::Unavailable
        );
        assert_eq!(
            state_fault_to_failure(StateFault::Missing),
            SlotOpFailure::Missing
        );
        assert_eq!(
            state_fault_to_failure(StateFault::Storage),
            SlotOpFailure::Storage
        );
        assert_eq!(
            state_fault_to_failure(StateFault::Corrupt),
            SlotOpFailure::Corrupt
        );
        assert_eq!(
            state_fault_to_failure(StateFault::Incompatible),
            SlotOpFailure::Incompatible
        );
    }
}
