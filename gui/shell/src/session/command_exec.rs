//! Command execution: dispatch plus per-command executors. Colocated
//! with the command vocabulary it feeds (`commands::outcome_notifications`);
//! orchestration only, the mapping itself stays in `commands`.

use crate::session::{
    SessionError, SessionHandle,
    commands::{
        SessionCommand, SessionCommandOutcome, command_name, outcome_notification, resolve_toggle,
    },
    error::SlotOpFailure,
    osd::TickMetrics,
    persistence::SlotDirection,
};

/// Persistence operation behind a slot command. The executor matches
/// on this instead of taking a closure over `PersistenceManager` and
/// `EmuCore`, so the persistence/core seam never leaks into the
/// dispatch signature.
#[derive(Debug, Clone, Copy)]
enum SlotOp {
    CreateSlot,
    SaveActiveSlotOrNew,
    LoadActiveSlot,
    SaveSlot(u64),
    LoadSlot(u64),
    DeleteSlot(u64),
}

impl SessionHandle {
    pub fn run_command(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionCommandOutcome, SessionError> {
        // One metrics lock per command: the tick threads through the
        // executor and the notification below, so steady frames pay
        // exactly one lock. (Loads re-fetch once on success for the
        // redraw decision; see `run_load_op`.)
        let tick = self.tick_metrics();
        let result = self.run_command_inner(command, &tick);
        match &result {
            Ok(outcome) => {
                let active = self.active_slot_id();
                // Notify the entered state: TogglePause from paused
                // must read Resumed, not the invoked command name.
                let notified = match command {
                    SessionCommand::TogglePause => resolve_toggle(command, tick.paused),
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
        tick: &TickMetrics,
    ) -> Result<SessionCommandOutcome, SessionError> {
        match command {
            SessionCommand::Pause => self.cmd_pause(tick),
            SessionCommand::Resume => self.cmd_resume(tick),
            SessionCommand::TogglePause => self.cmd_toggle_pause(tick),
            SessionCommand::Reset => self.cmd_reset(),
            SessionCommand::CreateSlot => Ok(self.run_slot_op(SlotOp::CreateSlot)),
            SessionCommand::SaveActiveSlotOrNew => {
                Ok(self.run_slot_op(SlotOp::SaveActiveSlotOrNew))
            }
            SessionCommand::LoadActiveSlot => Ok(self.run_load_op(SlotOp::LoadActiveSlot, tick)),
            SessionCommand::SelectActiveSlot(id) => Ok(self.cmd_select_active_slot(id)),
            SessionCommand::SaveSlot(id) => Ok(self.run_slot_op(SlotOp::SaveSlot(id))),
            SessionCommand::LoadSlot(id) => Ok(self.run_load_op(SlotOp::LoadSlot(id), tick)),
            SessionCommand::DeleteSlot(id) => Ok(self.run_slot_op(SlotOp::DeleteSlot(id))),
            SessionCommand::SelectNextSlot => Ok(self.cmd_adjacent_slot(SlotDirection::Next)),
            SessionCommand::SelectPreviousSlot => {
                Ok(self.cmd_adjacent_slot(SlotDirection::Previous))
            }
        }
    }

    fn cmd_pause(&mut self, tick: &TickMetrics) -> Result<SessionCommandOutcome, SessionError> {
        if tick.paused {
            return Ok(SessionCommandOutcome::default());
        }
        self.emu.core()?.pause()?;
        Ok(SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        })
    }

    fn cmd_resume(&mut self, tick: &TickMetrics) -> Result<SessionCommandOutcome, SessionError> {
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

    fn cmd_toggle_pause(
        &mut self,
        tick: &TickMetrics,
    ) -> Result<SessionCommandOutcome, SessionError> {
        if tick.paused {
            self.cmd_resume(tick)
        } else {
            self.cmd_pause(tick)
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

    /// Run one slot op against the persistence manager and the live
    /// core. Absent core reads as [`SlotOpFailure::Empty`]; failures
    /// ride the outcome instead of erroring the command.
    fn run_slot_op(&mut self, op: SlotOp) -> SessionCommandOutcome {
        let slot_failure = match self.emu.core().ok() {
            Some(core) => {
                let persistence = &mut self.media.persistence;
                let result = match op {
                    SlotOp::CreateSlot => persistence.create_slot(core),
                    SlotOp::SaveActiveSlotOrNew => persistence.save_active_slot_or_new(core),
                    SlotOp::SaveSlot(id) => persistence.save_slot(id, core, false),
                    SlotOp::DeleteSlot(id) => persistence.delete_slot(id, core),
                    SlotOp::LoadActiveSlot | SlotOp::LoadSlot(_) => {
                        unreachable!("loads run through run_load_op")
                    }
                };
                result.err()
            }
            None => Some(SlotOpFailure::Empty),
        };
        SessionCommandOutcome {
            executed: slot_failure.is_none(),
            needs_redraw: false,
            slot_failure,
        }
    }

    /// Load variant of [`run_slot_op`]: a successful load that clears
    /// a stale pause needs a redraw. The pause comparison re-fetches
    /// once (the only second lock on this path); everything else
    /// reuses the command tick.
    fn run_load_op(&mut self, op: SlotOp, tick: &TickMetrics) -> SessionCommandOutcome {
        let slot_failure = match self.emu.core().ok() {
            Some(core) => {
                let persistence = &mut self.media.persistence;
                let result = match op {
                    SlotOp::LoadActiveSlot => persistence.load_active_slot(core),
                    SlotOp::LoadSlot(id) => persistence.load_slot(id, core),
                    _ => unreachable!("saves run through run_slot_op"),
                };
                result.err()
            }
            None => Some(SlotOpFailure::Empty),
        };
        let executed = slot_failure.is_none();
        // A successful load that clears a stale pause needs a redraw:
        // re-fetch once for the comparison (the only second lock on
        // this path). Anything else reuses the command tick.
        let now_paused = if executed {
            self.tick_metrics().paused
        } else {
            tick.paused
        };
        SessionCommandOutcome {
            executed,
            needs_redraw: executed && tick.paused && !now_paused,
            slot_failure,
        }
    }
}
