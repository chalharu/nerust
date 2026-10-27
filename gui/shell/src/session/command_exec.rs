//! Command execution: dispatch plus per-command executors.
//! Orchestration only; the outcome mapping stays in `commands`.

use crate::session::{
    SessionError, SessionHandle,
    commands::{
        SessionCmd, SessionCommand, SessionCommandOutcome, TogglePauseDecorator,
        outcome_notification,
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

/// Slot half behind a slot command: the single classification site.
/// A new op picks a side here, so dispatch and classification cannot drift apart.
#[derive(Debug, Clone, Copy)]
enum SlotOp {
    Save(SaveOp),
    Load(LoadOp),
}

/// Stable command names live on [`SessionCmd::cmd_name`]; dispatch matches exhaustively.
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
    match command.into_slot_op() {
        Some(SlotOp::Save(op)) => Ok(session.run_save_op(op)),
        Some(SlotOp::Load(op)) => Ok(session.run_load_op(op, tick)),
        None => Ok(unreachable_outcome()),
    }
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

trait CommandSlotOp {
    fn into_slot_op(self) -> Option<SlotOp>;
}

impl CommandSlotOp for SessionCommand {
    /// Slot half behind this command: the single classification site. Explicit
    /// arms (no wildcard) so a new command fails to compile until classified.
    fn into_slot_op(self) -> Option<SlotOp> {
        match self {
            SessionCommand::CreateSlot => Some(SlotOp::Save(SaveOp::CreateSlot)),
            SessionCommand::SaveActiveSlotOrNew => Some(SlotOp::Save(SaveOp::SaveActiveSlotOrNew)),
            SessionCommand::SaveSlot(id) => Some(SlotOp::Save(SaveOp::SaveSlot(id))),
            SessionCommand::DeleteSlot(id) => Some(SlotOp::Save(SaveOp::DeleteSlot(id))),
            SessionCommand::LoadActiveSlot => Some(SlotOp::Load(LoadOp::LoadActiveSlot)),
            SessionCommand::LoadSlot(id) => Some(SlotOp::Load(LoadOp::LoadSlot(id))),
            SessionCommand::Pause
            | SessionCommand::Resume
            | SessionCommand::TogglePause
            | SessionCommand::Reset
            | SessionCommand::SelectActiveSlot(_)
            | SessionCommand::SelectNextSlot
            | SessionCommand::SelectPreviousSlot => None,
        }
    }
}

impl SessionHandle {
    pub fn run_command(
        &mut self,
        command: SessionCommand,
    ) -> Result<SessionCommandOutcome, SessionError> {
        let tick = self.tick_metrics();
        let resolved = TogglePauseDecorator::resolve(command, &tick);
        let result = self.run_command_inner(resolved, &tick);
        match &result {
            Ok(outcome) => {
                let active = self.active_slot_id();
                if let Some(kind) = outcome_notification(resolved, outcome, active) {
                    self.notifications.push(kind);
                }
            }
            Err(error) => {
                let name = command.cmd_name();
                log::warn!("run_command {name} failed: {error}");
                self.notifications.push(
                    crate::session::notification::Message::command_failure_from(
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
        // Exhaustive dispatch: a new command fails to compile until routed
        // here and classified in `into_slot_op`. TogglePause never arrives:
        // the entry resolves it once for execution and notification alike.
        match command {
            SessionCommand::Pause => run_pause(self, command, tick),
            SessionCommand::Resume => run_resume(self, command, tick),
            SessionCommand::Reset => run_reset(self, command, tick),
            SessionCommand::CreateSlot
            | SessionCommand::SaveActiveSlotOrNew
            | SessionCommand::LoadActiveSlot
            | SessionCommand::SaveSlot(_)
            | SessionCommand::LoadSlot(_)
            | SessionCommand::DeleteSlot(_) => run_slot_command(self, command, tick),
            SessionCommand::SelectActiveSlot(_) => run_select_active(self, command, tick),
            SessionCommand::SelectNextSlot => run_adjacent_next(self, command, tick),
            SessionCommand::SelectPreviousSlot => run_adjacent_previous(self, command, tick),
            SessionCommand::TogglePause => {
                debug_assert!(false, "TogglePause must resolve before dispatch");
                Ok(unreachable_outcome())
            }
        }
    }

    fn cmd_pause(&mut self, tick: &TickMetrics) -> Result<SessionCommandOutcome, SessionError> {
        if tick.paused {
            return Ok(SessionCommandOutcome::default());
        }
        crate::emu_core::PlaybackPort::pause(self.emu.require_live()?)?;
        Ok(SessionCommandOutcome {
            executed: true,
            needs_redraw: false,
            slot_failure: None,
        })
    }

    fn cmd_resume(&mut self, tick: &TickMetrics) -> Result<SessionCommandOutcome, SessionError> {
        if !tick.paused || self.execution_held {
            return Ok(SessionCommandOutcome::default());
        }
        crate::emu_core::PlaybackPort::resume(self.emu.require_live()?)?;
        Ok(SessionCommandOutcome {
            executed: true,
            needs_redraw: tick.loaded,
            slot_failure: None,
        })
    }

    fn cmd_reset(&mut self) -> Result<SessionCommandOutcome, SessionError> {
        self.emu.invalidate_debug();
        self.debug_clear_write();
        crate::emu_core::PlaybackPort::reset(self.emu.require_live()?)?;
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

    /// Single live access: absence maps once to `Empty` in `live_or_empty`.
    fn run_save_op(&mut self, op: SaveOp) -> SessionCommandOutcome {
        let live = match self.emu.live_or_empty() {
            Ok(core) => core.as_exporter(),
            Err(failure) => {
                return SessionCommandOutcome {
                    executed: false,
                    needs_redraw: false,
                    slot_failure: Some(failure),
                };
            }
        };
        // Borrow ends before the persistence borrow: copy the dispatch first.
        let slot_failure = {
            let persistence = &mut self.media.persistence;
            let result = match op {
                SaveOp::CreateSlot => persistence.create_slot(live),
                SaveOp::SaveActiveSlotOrNew => persistence.save_active_slot_or_new(live),
                SaveOp::SaveSlot(id) => persistence.save_slot(
                    id,
                    live,
                    crate::session::persistence::SlotActivation::Leave,
                ),
                SaveOp::DeleteSlot(id) => persistence.delete_slot(id),
            };
            if let Err(failure) = &result {
                log::warn!("slot save failed: {failure:?}");
            }
            result.err()
        };
        SessionCommandOutcome {
            executed: slot_failure.is_none(),
            needs_redraw: false,
            slot_failure,
        }
    }

    /// Single live access: absence maps once to `Empty` in `live_or_empty`.
    fn run_load_op(&mut self, op: LoadOp, tick: &TickMetrics) -> SessionCommandOutcome {
        self.emu.invalidate_debug();
        self.debug_clear_write();
        let slot_failure = match self.emu.live_or_empty() {
            Ok(core) => {
                let live = core.as_exporter();
                let persistence = &mut self.media.persistence;
                let result = match op {
                    LoadOp::LoadActiveSlot => persistence.load_active_slot(live),
                    LoadOp::LoadSlot(id) => persistence.load_slot(id, live),
                };
                if let Err(failure) = &result {
                    log::warn!("slot load failed: {failure:?}");
                }
                result.err()
            }
            Err(failure) => Some(failure),
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

#[cfg(test)]
mod tests {
    use crate::session::command_exec::*;
    use crate::session::commands::{SessionCmd, TogglePauseDecorator};

    #[test]
    fn dispatch_routes_every_command_through_the_entry() {
        use crate::session::commands::SessionCommand as C;
        use crate::session::test_util::test_session;
        // End to end through `run_command`: resolution, dispatch,
        // and notification stay wired for every command.
        let mut session = test_session();
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
            assert!(session.run_command(command).is_ok(), "{command:?}");
            assert!(!command.cmd_name().is_empty());
        }
    }

    #[test]
    fn toggle_resolves_once_before_dispatch() {
        use crate::session::commands::SessionCommand as C;
        let tick = TickMetrics {
            paused: true,
            ..TickMetrics::default()
        };
        assert_eq!(
            TogglePauseDecorator::resolve(C::TogglePause, &tick),
            C::Resume
        );
        let tick = TickMetrics {
            paused: false,
            ..TickMetrics::default()
        };
        assert_eq!(
            TogglePauseDecorator::resolve(C::TogglePause, &tick),
            C::Pause
        );
    }

    #[test]
    fn coreless_save_and_load_map_to_empty_once() {
        use crate::session::test_util::test_session;
        let mut session = test_session();
        session.emu.restore(None);
        let saved = session.run_save_op(SaveOp::CreateSlot);
        assert_eq!(saved.slot_failure, Some(SlotOpFailure::Empty));
        assert!(!saved.executed);
        let tick = TickMetrics::default();
        let loaded = session.run_load_op(LoadOp::LoadActiveSlot, &tick);
        assert_eq!(loaded.slot_failure, Some(SlotOpFailure::Empty));
    }
}

#[cfg(test)]
mod exec_cov_tests {
    use crate::session::command_exec::*;
    use crate::session::test_util::test_session;

    #[test]
    fn pause_resume_are_idempotent_on_tick() {
        let mut session = test_session();
        let tick_paused = TickMetrics {
            paused: true,
            ..TickMetrics::default()
        };
        let tick_running = TickMetrics::default();
        assert!(!session.cmd_pause(&tick_paused).unwrap().executed);
        assert!(!session.cmd_resume(&tick_running).unwrap().executed);
    }

    #[test]
    fn select_and_adjacent_cover_all_arms() {
        let mut session = test_session();
        let out = session.cmd_select_active_slot(3);
        assert!(out.executed);
        assert_eq!(session.active_slot_id(), Some(3));
        let next = session.cmd_adjacent_slot(SlotDirection::Next);
        assert!(!next.executed);
        let prev = session.cmd_adjacent_slot(SlotDirection::Previous);
        assert!(!prev.executed);
    }

    #[test]
    fn run_slot_command_routes_both_halves() {
        let mut session = test_session();
        let tick = TickMetrics::default();
        let save = run_slot_command(&mut session, SessionCommand::SaveSlot(1), &tick);
        assert!(save.is_ok());
        let load = run_slot_command(&mut session, SessionCommand::LoadSlot(1), &tick);
        assert!(load.is_ok());
    }

    #[test]
    fn run_select_and_adjacent_handlers_cover_mismatch() {
        use crate::session::commands::SessionCommand as C;
        let mut session = test_session();
        let tick = TickMetrics::default();
        let out = run_select_active(&mut session, C::SelectActiveSlot(7), &tick).unwrap();
        assert!(out.executed);
        let next = run_adjacent_next(&mut session, C::SelectNextSlot, &tick).unwrap();
        assert!(!next.executed);
        let prev = run_adjacent_previous(&mut session, C::SelectPreviousSlot, &tick).unwrap();
        assert!(!prev.executed);
        // TogglePause resolves once at the entry: toggling twice restores
        // the live pause state, whichever arm it starts on.
        let before = session.tick_metrics().paused;
        let toggle = session.run_command(C::TogglePause).unwrap();
        assert!(toggle.executed);
        assert_eq!(session.tick_metrics().paused, !before);
        let back = session.run_command(C::TogglePause).unwrap();
        assert!(back.executed);
        assert_eq!(session.tick_metrics().paused, before);
        let reset = run_reset(&mut session, C::Reset, &tick).unwrap();
        assert!(reset.executed);
    }

    #[test]
    fn save_op_maps_unavailable_besides_nocore() {
        let mut session = test_session();
        session.emu.restore(None);
        let tick = TickMetrics::default();
        for op in [
            SaveOp::SaveActiveSlotOrNew,
            SaveOp::SaveSlot(2),
            SaveOp::DeleteSlot(2),
        ] {
            let out = session.run_save_op(op);
            assert_eq!(out.slot_failure, Some(SlotOpFailure::Empty));
        }
        let load = session.run_load_op(LoadOp::LoadSlot(2), &tick);
        assert_eq!(load.slot_failure, Some(SlotOpFailure::Empty));
    }
}
