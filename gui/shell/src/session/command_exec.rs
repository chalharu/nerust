//! Command execution: dispatch plus per-command executors. Colocated
//! with the command vocabulary it feeds (`commands::outcome_notifications`);
//! orchestration only, the mapping itself stays in `commands`.

use crate::session::{
    SessionError, SessionHandle,
    commands::{
        SessionCmd, SessionCommand, SessionCommandOutcome, outcome_notification, resolve_toggle,
    },
    error::SlotOpFailure,
    osd::TickMetrics,
    persistence::SlotDirection,
};

/// Save-side persistence operation behind a slot command. Split from
/// [`LoadOp`] so the `unreachable!` legs disappear: the executor
/// matches on the half it runs, and a new op fails to compile until
/// it picks a side.
#[derive(Debug, Clone, Copy)]
enum SaveOp {
    CreateSlot,
    SaveActiveSlotOrNew,
    SaveSlot(u64),
    DeleteSlot(u64),
}

/// Load-side persistence operation behind a slot command. Loads run
/// through [`run_load_op`](SessionHandle::run_load_op), which takes
/// the post-load tick for the redraw decision.
#[derive(Debug, Clone, Copy)]
enum LoadOp {
    LoadActiveSlot,
    LoadSlot(u64),
}

impl SessionCommand {
    /// Save half of this command, if it saves.
    fn as_save_op(self) -> Option<SaveOp> {
        match self {
            SessionCommand::CreateSlot => Some(SaveOp::CreateSlot),
            SessionCommand::SaveActiveSlotOrNew => Some(SaveOp::SaveActiveSlotOrNew),
            SessionCommand::SaveSlot(id) => Some(SaveOp::SaveSlot(id)),
            SessionCommand::DeleteSlot(id) => Some(SaveOp::DeleteSlot(id)),
            _ => None,
        }
    }

    /// Load half of this command, if it loads.
    fn as_load_op(self) -> Option<LoadOp> {
        match self {
            SessionCommand::LoadActiveSlot => Some(LoadOp::LoadActiveSlot),
            SessionCommand::LoadSlot(id) => Some(LoadOp::LoadSlot(id)),
            _ => None,
        }
    }
}

impl SessionHandle {
    pub fn run_command(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionCommandOutcome, SessionError> {
        // One metrics lock per command: the tick threads through the
        // executor and the notification below, so steady frames pay
        // exactly one lock. Loads fetch once more on success for the
        // redraw decision (see `run_load_op`): the executor takes
        // that post tick as a parameter instead of re-locking.
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
                // The typed source chain renders at the boundary, so no
                // `error.to_string()` escapes into the notification.
                let name = command.cmd_name();
                log::warn!("run_command {name} failed: {error}");
                self.notifications.push(
                    crate::session::notification::NotificationKind::command_failure_from(
                        crate::session::notification::FailureOrigin::Command,
                        name,
                        error,
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
        // Slot commands dispatch through their save/load half: one
        // match, no second dispatch inside the runners.
        if let Some(op) = command.as_save_op() {
            return Ok(self.run_save_op(op));
        }
        if let Some(op) = command.as_load_op() {
            // The redraw decision needs the post-load pause state, so
            // the caller fetches once and threads it in: the runner
            // itself never re-locks.
            let post_tick = self.tick_metrics();
            return Ok(self.run_load_op(op, tick, &post_tick));
        }
        match command {
            SessionCommand::Pause => self.cmd_pause(tick),
            SessionCommand::Resume => self.cmd_resume(tick),
            SessionCommand::TogglePause => self.cmd_toggle_pause(tick),
            SessionCommand::Reset => self.cmd_reset(),
            SessionCommand::SelectActiveSlot(id) => Ok(self.cmd_select_active_slot(id)),
            SessionCommand::SelectNextSlot => Ok(self.cmd_adjacent_slot(SlotDirection::Next)),
            SessionCommand::SelectPreviousSlot => {
                Ok(self.cmd_adjacent_slot(SlotDirection::Previous))
            }
            // Save/load arms return above; reaching one here means the
            // halves disagree with the command set. The assert pins
            // the invariant with a real condition; the fallback keeps
            // the command total even in release.
            SessionCommand::CreateSlot
            | SessionCommand::SaveActiveSlotOrNew
            | SessionCommand::SaveSlot(_)
            | SessionCommand::DeleteSlot(_)
            | SessionCommand::LoadActiveSlot
            | SessionCommand::LoadSlot(_) => {
                debug_assert!(
                    command.as_save_op().is_some() || command.as_load_op().is_some(),
                    "slot command missing its save/load half",
                );
                Ok(SessionCommandOutcome {
                    executed: false,
                    needs_redraw: false,
                    slot_failure: Some(SlotOpFailure::Unavailable),
                })
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

    /// Run one save op against the persistence manager and the live
    /// core. Absent core reads as [`SlotOpFailure::Empty`]; failures
    /// ride the outcome instead of erroring the command.
    fn run_save_op(&mut self, op: SaveOp) -> SessionCommandOutcome {
        let exporter = self.emu.exporter().ok();
        let slot_failure = match exporter {
            Some(core) => {
                let persistence = &mut self.media.persistence;
                let result = match op {
                    SaveOp::CreateSlot => persistence.create_slot(core),
                    SaveOp::SaveActiveSlotOrNew => persistence.save_active_slot_or_new(core),
                    SaveOp::SaveSlot(id) => persistence.save_slot(
                        id,
                        core,
                        crate::session::persistence::SlotActivation::Leave,
                    ),
                    SaveOp::DeleteSlot(id) => persistence.delete_slot(id, core),
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

    /// Load variant of [`run_save_op`](Self::run_save_op): a successful
    /// load that clears a stale pause needs a redraw. The pause
    /// comparison uses the caller-threaded `post_tick`, so this runner
    /// itself never re-locks.
    fn run_load_op(
        &mut self,
        op: LoadOp,
        tick: &TickMetrics,
        post_tick: &TickMetrics,
    ) -> SessionCommandOutcome {
        let exporter = self.emu.exporter().ok();
        let slot_failure = match exporter {
            Some(core) => {
                let persistence = &mut self.media.persistence;
                let resolve = crate::state::resolve_state_format;
                let result = match op {
                    LoadOp::LoadActiveSlot => persistence.load_active_slot(core, resolve),
                    LoadOp::LoadSlot(id) => persistence.load_slot(id, core, resolve),
                };
                result.err()
            }
            None => Some(SlotOpFailure::Empty),
        };
        let executed = slot_failure.is_none();
        // A successful load that clears a stale pause needs a redraw:
        // compare against the caller-threaded post tick instead of
        // re-locking here.
        let now_paused = if executed {
            post_tick.paused
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
