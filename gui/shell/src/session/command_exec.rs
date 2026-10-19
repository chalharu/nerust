//! Command execution: dispatch plus per-command executors. Colocated
//! with the command vocabulary it feeds (`commands::outcome_notifications`);
//! orchestration only, the mapping itself stays in `commands`.

use crate::{
    emu_core::EmuCore,
    session::{
        SessionError, SessionHandle,
        commands::{
            SessionCommand, SessionCommandOutcome, command_name, outcome_notification,
            resolve_toggle,
        },
        error::SlotOpFailure,
        persistence::{PersistenceManager, SlotDirection},
    },
};

impl SessionHandle {
    pub fn run_command(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionCommandOutcome, SessionError> {
        // Resolve TogglePause to its steady arm so notifications name
        // the state actually entered. One metrics lock per command.
        let was_paused = self.tick_metrics().paused;
        let result = self.run_command_inner(command);
        match &result {
            Ok(outcome) => {
                let active = self.active_slot_id();
                // Notify the entered state: TogglePause from paused
                // must read Resumed, not the invoked command name.
                let notified = match command {
                    SessionCommand::TogglePause => resolve_toggle(command, was_paused),
                    other => other,
                };
                if let Some(kind) = outcome_notification(notified, outcome, active) {
                    self.notifications.push(kind);
                }
            }
            Err(error) => {
                // Single audit point for transport failures: frontends
                // must not log these at their call sites. The push
                // below logs the UX wording; this line keeps the cause.
                let name = command_name(command);
                log::warn!("run_command {name} failed: {error}");
                self.notifications.push(
                    crate::session::notification::NotificationKind::command_failure(
                        crate::session::notification::FailureOrigin::Command,
                        name,
                        error.to_string(),
                    ),
                )
            }
        }
        result
    }

    fn run_command_inner(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionCommandOutcome, SessionError> {
        match command {
            SessionCommand::Pause => self.cmd_pause(),
            SessionCommand::Resume => self.cmd_resume(),
            SessionCommand::TogglePause => self.cmd_toggle_pause(),
            SessionCommand::Reset => self.cmd_reset(),
            SessionCommand::CreateSlot => Ok(self.slot_op_ok(|p, c| p.create_slot(c))),
            SessionCommand::SaveActiveSlotOrNew => {
                Ok(self.slot_op_ok(|p, c| p.save_active_slot_or_new(c)))
            }
            SessionCommand::LoadActiveSlot => Ok(self.load_slot_op(|p, c| p.load_active_slot(c))),
            SessionCommand::SelectActiveSlot(id) => Ok(self.cmd_select_active_slot(id)),
            SessionCommand::SaveSlot(id) => Ok(self.slot_op_ok(|p, c| p.save_slot(id, c, false))),
            SessionCommand::LoadSlot(id) => Ok(self.load_slot_op(|p, c| p.load_slot(id, c))),
            SessionCommand::DeleteSlot(id) => Ok(self.slot_op_ok(|p, c| p.delete_slot(id, c))),
            SessionCommand::SelectNextSlot => Ok(self.cmd_adjacent_slot(SlotDirection::Next)),
            SessionCommand::SelectPreviousSlot => {
                Ok(self.cmd_adjacent_slot(SlotDirection::Previous))
            }
        }
    }

    fn cmd_pause(&mut self) -> Result<SessionCommandOutcome, SessionError> {
        if self.tick_metrics().paused {
            return Ok(SessionCommandOutcome::default());
        }
        self.emu.core()?.pause()?;
        Ok(SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        })
    }

    fn cmd_resume(&mut self) -> Result<SessionCommandOutcome, SessionError> {
        let tick = self.tick_metrics();
        if !tick.paused {
            return Ok(SessionCommandOutcome::default());
        }
        self.emu.core()?.resume()?;
        Ok(SessionCommandOutcome {
            executed: true,
            needs_redraw: tick.loaded,
            slot_failure: None,
        })
    }

    fn cmd_toggle_pause(&mut self) -> Result<SessionCommandOutcome, SessionError> {
        if self.tick_metrics().paused {
            self.cmd_resume()
        } else {
            self.cmd_pause()
        }
    }

    fn cmd_reset(&mut self) -> Result<SessionCommandOutcome, SessionError> {
        self.emu.core()?.reset()?;
        Ok(SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        })
    }

    fn cmd_select_active_slot(&mut self, slot_id: u64) -> SessionCommandOutcome {
        self.media.persistence.select_active_slot(slot_id);
        SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        }
    }

    fn cmd_adjacent_slot(&mut self, direction: SlotDirection) -> SessionCommandOutcome {
        SessionCommandOutcome {
            executed: self
                .media
                .persistence
                .select_adjacent_slot(direction)
                .is_some(),
            needs_redraw: false,
            slot_failure: None,
        }
    }

    fn slot_op_ok(
        &mut self,
        op: impl FnOnce(&mut PersistenceManager, &EmuCore) -> Result<(), SlotOpFailure>,
    ) -> SessionCommandOutcome {
        let slot_failure = if let Some(core) = self.emu.core_opt() {
            op(&mut self.media.persistence, core).err()
        } else {
            Some(SlotOpFailure::Empty)
        };
        SessionCommandOutcome {
            executed: slot_failure.is_none(),
            needs_redraw: false,
            slot_failure,
        }
    }

    fn load_slot_op(
        &mut self,
        op: impl FnOnce(&mut PersistenceManager, &EmuCore) -> Result<(), SlotOpFailure>,
    ) -> SessionCommandOutcome {
        let was_paused = self.tick_metrics().paused;
        let slot_failure = if let Some(core) = self.emu.core_opt() {
            op(&mut self.media.persistence, core).err()
        } else {
            Some(SlotOpFailure::Empty)
        };
        let executed = slot_failure.is_none();
        let now_paused = self.tick_metrics().paused;
        SessionCommandOutcome {
            executed,
            needs_redraw: executed && was_paused && !now_paused,
            slot_failure,
        }
    }
}
