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
    ) -> Option<crate::session::notification::Message>;
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
    ) -> Option<crate::session::notification::Message> {
        use crate::session::notification::Message;
        if let Some(reason) = outcome.slot_failure {
            let slot = self.slot_affinity(active_slot);
            return Some(Message::slot_failed(slot, reason));
        }
        if !outcome.executed {
            return None;
        }
        Some(match self {
            SessionCommand::Pause | SessionCommand::TogglePause => Message::paused(),
            SessionCommand::Resume => Message::resumed(),
            SessionCommand::Reset => Message::reset(),
            SessionCommand::CreateSlot => Message::created_slot(),
            SessionCommand::SaveActiveSlotOrNew => Message::saved_slot(active_slot.unwrap_or(0)),
            SessionCommand::LoadActiveSlot => Message::loaded_slot(active_slot.unwrap_or(0)),
            SessionCommand::SelectActiveSlot(id) => Message::selected_slot(*id),
            SessionCommand::SelectNextSlot | SessionCommand::SelectPreviousSlot => {
                Message::selected_slot(active_slot.unwrap_or(0))
            }
            SessionCommand::SaveSlot(id) => Message::saved_slot(*id),
            SessionCommand::LoadSlot(id) => Message::loaded_slot(*id),
            SessionCommand::DeleteSlot(id) => Message::deleted_slot(*id),
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
    ) -> Option<crate::session::notification::Message> {
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
) -> Option<crate::session::notification::Message> {
    notification_for(command, outcome, active_slot)
}

/// Single source mapping command outcomes to notifications.
pub fn notification_for(
    command: SessionCommand,
    outcome: &SessionCommandOutcome,
    active_slot: Option<u64>,
) -> Option<crate::session::notification::Message> {
    command.success_notification(outcome, active_slot)
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
        SessionCmd as _, SessionCommand, SessionCommandOutcome, outcome_notification,
        resolve_toggle,
    };
    use crate::session::error::SlotOpFailure;
    use crate::session::notification::{Message, NotificationLevel};

    #[test]
    fn slot_failure_maps_to_warning_or_error() {
        let warn = outcome_notification(
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
            Some(Message::slot_failed(Some(2), SlotOpFailure::Missing,))
        );
        assert_eq!(warn.unwrap().level(), NotificationLevel::Warning);
        let err = outcome_notification(
            SessionCommand::SaveSlot(1),
            &SessionCommandOutcome {
                executed: false,
                needs_redraw: false,
                slot_failure: Some(SlotOpFailure::Storage),
            },
            None,
        );
        assert_eq!(err.unwrap().level(), NotificationLevel::Error);
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
            outcome_notification(resolve_toggle(C::TogglePause, true), &ok, None),
            Some(Message::resumed())
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
            (SessionCommand::Pause, Message::paused()),
            (SessionCommand::Resume, Message::resumed()),
            (SessionCommand::Reset, Message::reset()),
            (SessionCommand::CreateSlot, Message::created_slot()),
            (SessionCommand::SelectNextSlot, Message::selected_slot(0)),
            (
                SessionCommand::SelectPreviousSlot,
                Message::selected_slot(0),
            ),
            (SessionCommand::SaveSlot(2), Message::saved_slot(2)),
            (SessionCommand::LoadSlot(2), Message::loaded_slot(2)),
            (SessionCommand::DeleteSlot(2), Message::deleted_slot(2)),
            (SessionCommand::LoadActiveSlot, Message::loaded_slot(0)),
        ];
        for (command, expected) in cases {
            assert_eq!(outcome_notification(command, &ok, Some(0)), Some(expected));
        }
        // Unexecuted commands notify nothing.
        let skipped = SessionCommandOutcome {
            executed: false,
            needs_redraw: false,
            slot_failure: None,
        };
        assert!(outcome_notification(SessionCommand::Pause, &skipped, Some(0)).is_none());
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
            outcome_notification(C::SaveActiveSlotOrNew, &failed, Some(7)),
            Some(Message::slot_failed(Some(7), SlotOpFailure::Missing,))
        );
        assert_eq!(
            outcome_notification(C::Pause, &failed, Some(7)),
            Some(Message::slot_failed(None, SlotOpFailure::Missing,))
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
            outcome_notification(SessionCommand::SaveActiveSlotOrNew, &ok, Some(3)),
            Some(Message::saved_slot(3))
        );
        assert_eq!(
            outcome_notification(SessionCommand::SelectActiveSlot(5), &ok, Some(3)),
            Some(Message::selected_slot(5))
        );
    }
}

#[cfg(test)]
mod mapping_tests {
    use crate::session::commands::*;

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
}
