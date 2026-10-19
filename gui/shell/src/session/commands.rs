pub use super::error::SlotOpFailure;

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

/// Map a command outcome to notifications. Success notifies Info;
/// slot failures map to Warning/Error by reason; transport errors
/// become a generic Error.
pub fn outcome_notifications(
    command: SessionCommand,
    outcome: &SessionCommandOutcome,
    active_slot: Option<u64>,
) -> Vec<super::notification::NotificationKind> {
    use super::notification::NotificationKind;
    if let Some(reason) = outcome.slot_failure {
        let slot = command.associated_slot(active_slot);
        return vec![NotificationKind::SlotFailed { slot, reason }];
    }
    if !outcome.executed {
        return Vec::new();
    }
    let kind = match command {
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
    };
    vec![kind]
}

/// Resolve TogglePause to the steady arm entered from `was_paused`.
pub fn resolve_toggle(command: SessionCommand, was_paused: bool) -> SessionCommand {
    match (command, was_paused) {
        (SessionCommand::TogglePause, false) => SessionCommand::Pause,
        (SessionCommand::TogglePause, true) => SessionCommand::Resume,
        (other, _) => other,
    }
}
