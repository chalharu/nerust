//! Command execution: dispatch plus per-command executors.
//! Orchestration only; the outcome mapping stays in `commands`.

use crate::session::{
    SessionError, SessionHandle,
    commands::{
        CommandHandler, SessionCmd, SessionCommand, SessionCommandHandler, SessionCommandOutcome,
        TogglePauseDecorator, outcome_notification,
    },
    error::SlotOpFailure,
    osd::TickMetrics,
    persistence::SlotDirection,
};

/// Save-side persistence operation behind a slot command, split from [`LoadOp`].
/// A new op fails to compile until it picks a side.
#[derive(Debug, Clone, Copy)]
enum SaveOp {
    CreateSlot,
    SaveActiveSlotOrNew,
    SaveSlot(u64),
    DeleteSlot(u64),
}

/// Load-side persistence operation behind a slot command.
/// Loads run through `run_load_op`, which takes the post-load tick for the redraw decision.
#[derive(Debug, Clone, Copy)]
enum LoadOp {
    LoadActiveSlot,
    LoadSlot(u64),
}

/// One table row: stable name plus its handler. Lookup is by name;
/// [`SessionCommandHandler::handler`] is the sole dispatch point.
struct CommandEntry {
    name: &'static str,
    run: CommandHandler,
}

fn run_pause(
    session: &mut SessionHandle,
    _: SessionCommand,
    tick: &TickMetrics,
) -> Result<SessionCommandOutcome, SessionError> {
    session.cmd_pause(tick)
}

fn run_resume(
    session: &mut SessionHandle,
    _: SessionCommand,
    tick: &TickMetrics,
) -> Result<SessionCommandOutcome, SessionError> {
    session.cmd_resume(tick)
}

/// TogglePause arm: resolves through the decorator, then delegates to the steady handler.
fn run_toggle(
    session: &mut SessionHandle,
    command: SessionCommand,
    tick: &TickMetrics,
) -> Result<SessionCommandOutcome, SessionError> {
    let resolved = TogglePauseDecorator::resolve(command, tick);
    resolved.handler()(session, resolved, tick)
}

fn run_reset(
    session: &mut SessionHandle,
    _: SessionCommand,
    _: &TickMetrics,
) -> Result<SessionCommandOutcome, SessionError> {
    session.cmd_reset()
}

fn run_select_active(
    session: &mut SessionHandle,
    command: SessionCommand,
    _: &TickMetrics,
) -> Result<SessionCommandOutcome, SessionError> {
    let SessionCommand::SelectActiveSlot(id) = command else {
        return Ok(unreachable_outcome());
    };
    Ok(session.cmd_select_active_slot(id))
}

fn run_adjacent_next(
    session: &mut SessionHandle,
    _: SessionCommand,
    _: &TickMetrics,
) -> Result<SessionCommandOutcome, SessionError> {
    Ok(session.cmd_adjacent_slot(SlotDirection::Next))
}

fn run_adjacent_previous(
    session: &mut SessionHandle,
    _: SessionCommand,
    _: &TickMetrics,
) -> Result<SessionCommandOutcome, SessionError> {
    Ok(session.cmd_adjacent_slot(SlotDirection::Previous))
}

fn run_slot_command(
    session: &mut SessionHandle,
    command: SessionCommand,
    tick: &TickMetrics,
) -> Result<SessionCommandOutcome, SessionError> {
    if let Some(op) = command.as_save_op() {
        return Ok(session.run_save_op(op));
    }
    if let Some(op) = command.as_load_op() {
        return Ok(session.run_load_op(op, tick));
    }
    Ok(unreachable_outcome())
}

/// Fallback for slot commands missing their half: total in release, loud in debug.
fn unreachable_outcome() -> SessionCommandOutcome {
    debug_assert!(false, "slot command missing its save/load half");
    SessionCommandOutcome {
        executed: false,
        needs_redraw: false,
        slot_failure: Some(SlotOpFailure::Unavailable),
    }
}

/// Registration table: every command resolves to exactly one handler by stable name.
static COMMAND_TABLE: &[CommandEntry] = &[
    CommandEntry {
        name: "Pause",
        run: run_pause,
    },
    CommandEntry {
        name: "Resume",
        run: run_resume,
    },
    CommandEntry {
        name: "TogglePause",
        run: run_toggle,
    },
    CommandEntry {
        name: "Reset",
        run: run_reset,
    },
    CommandEntry {
        name: "CreateSlot",
        run: run_slot_command,
    },
    CommandEntry {
        name: "SaveActiveSlotOrNew",
        run: run_slot_command,
    },
    CommandEntry {
        name: "LoadActiveSlot",
        run: run_slot_command,
    },
    CommandEntry {
        name: "SelectActiveSlot",
        run: run_select_active,
    },
    CommandEntry {
        name: "SaveSlot",
        run: run_slot_command,
    },
    CommandEntry {
        name: "LoadSlot",
        run: run_slot_command,
    },
    CommandEntry {
        name: "DeleteSlot",
        run: run_slot_command,
    },
    CommandEntry {
        name: "SelectNextSlot",
        run: run_adjacent_next,
    },
    CommandEntry {
        name: "SelectPreviousSlot",
        run: run_adjacent_previous,
    },
];

impl SessionCommandHandler for SessionCommand {
    /// Handler for this command from the registration table.
    /// `TogglePause` callers resolve through [`TogglePauseDecorator`] first.
    fn handler(self) -> CommandHandler {
        let name = self.cmd_name();
        COMMAND_TABLE
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.run)
            .expect("every SessionCommand registers exactly one handler")
    }
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
        // One lock-free tick per command, threaded through the executor and notification.
        let tick = self.tick_metrics();
        let result = self.run_command_inner(command, &tick);
        match &result {
            Ok(outcome) => {
                let active = self.active_slot_id();
                // Notify the entered state: TogglePause from paused
                // must read Resumed, not the invoked command name.
                let notified = TogglePauseDecorator::resolve(command, &tick);
                if let Some(kind) = outcome_notification(notified, outcome, active) {
                    self.notifications.push(kind);
                }
            }
            Err(error) => {
                // Single audit point for transport failures; frontends must not log at call sites.
                // The typed source chain renders at the boundary, never stringified in.
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
        // Single dispatch: the registration table owns the command-to-handler mapping.
        command.handler()(self, command, tick)
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

    /// Run one save op against the persistence manager and the live core.
    /// Absent core reads as [`SlotOpFailure::Empty`]; failures ride the outcome, not the error.
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

    /// Load variant of [`run_save_op`](Self::run_save_op): a cleared stale pause needs a redraw.
    /// One tick per command: the post-load pause re-reads only when a paused load succeeded.
    fn run_load_op(&mut self, op: LoadOp, tick: &TickMetrics) -> SessionCommandOutcome {
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
        // A successful load that clears a stale pause needs a redraw: re-read pause only then.
        let now_paused = if executed && tick.paused {
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
