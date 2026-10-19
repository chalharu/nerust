pub use crate::session::error::SlotOpFailure;

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

impl SessionCommand {
    /// Slot carried for failure messages. Explicit-slot commands use
    /// their own id; active-slot commands resolve to `active`, the
    /// rest report `None`. New variants must pick an arm here.
    pub fn associated_slot(&self, active: Option<u64>) -> Option<u64> {
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
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionCommandOutcome {
    pub executed: bool,
    pub needs_redraw: bool,
    /// Why a slot save/load did not execute (`None` on success and for
    /// commands that cannot fail this way). Frontends map this to a
    /// user-facing message; the full detail stays in the logs.
    pub slot_failure: Option<SlotOpFailure>,
}

/// Map a command outcome to its notification, if any. Success
/// notifies Info; slot failures map to Warning/Error by reason;
/// transport errors become a generic Error. `None` means silence.
pub fn outcome_notification(
    command: SessionCommand,
    outcome: &SessionCommandOutcome,
    active_slot: Option<u64>,
) -> Option<crate::session::notification::NotificationKind> {
    use crate::session::notification::NotificationKind;
    if let Some(reason) = outcome.slot_failure {
        let slot = command.associated_slot(active_slot);
        return Some(NotificationKind::SlotFailed { slot, reason });
    }
    if !outcome.executed {
        return None;
    }
    Some(match command {
        SessionCommand::Pause | SessionCommand::TogglePause => NotificationKind::Paused,
        SessionCommand::Resume => NotificationKind::Resumed,
        SessionCommand::Reset => NotificationKind::Reset,
        SessionCommand::CreateSlot => NotificationKind::CreatedSlot,
        SessionCommand::SaveActiveSlotOrNew => {
            NotificationKind::SavedSlot(active_slot.unwrap_or(0))
        }
        SessionCommand::LoadActiveSlot => NotificationKind::LoadedSlot(active_slot.unwrap_or(0)),
        SessionCommand::SelectActiveSlot(id) => NotificationKind::SelectedSlot(id),
        SessionCommand::SelectNextSlot | SessionCommand::SelectPreviousSlot => {
            NotificationKind::SelectedSlot(active_slot.unwrap_or(0))
        }
        SessionCommand::SaveSlot(id) => NotificationKind::SavedSlot(id),
        SessionCommand::LoadSlot(id) => NotificationKind::LoadedSlot(id),
        SessionCommand::DeleteSlot(id) => NotificationKind::DeletedSlot(id),
    })
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

/// Resolve TogglePause to the steady arm entered from `was_paused`.
pub fn resolve_toggle(command: SessionCommand, was_paused: bool) -> SessionCommand {
    match (command, was_paused) {
        (SessionCommand::TogglePause, false) => SessionCommand::Pause,
        (SessionCommand::TogglePause, true) => SessionCommand::Resume,
        (other, _) => other,
    }
}

/// Stable command name for logs and failure messages. Single source
/// so the transport audit and the UI wording cannot drift apart.
pub(crate) fn command_name(command: SessionCommand) -> &'static str {
    match command {
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

#[cfg(test)]
mod tests {
    use crate::session::commands::{
        SessionCommand, SessionCommandOutcome, outcome_notifications, resolve_toggle,
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
