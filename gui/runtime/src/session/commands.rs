use super::GuiSession;
use crate::session_api::{SessionCommand, SessionCommandOutcome};
use crate::slots::adjacent_slot_id;

impl GuiSession {
    pub fn run_command(&mut self, command: SessionCommand) -> SessionCommandOutcome {
        match command {
            SessionCommand::Pause => {
                if self.paused() {
                    SessionCommandOutcome::default()
                } else {
                    self.pause();
                    SessionCommandOutcome {
                        executed: true,
                        needs_redraw: false,
                    }
                }
            }
            SessionCommand::Resume => {
                if self.paused() {
                    self.resume();
                    SessionCommandOutcome {
                        executed: true,
                        needs_redraw: self.loaded(),
                    }
                } else {
                    SessionCommandOutcome::default()
                }
            }
            SessionCommand::TogglePause => {
                if self.paused() {
                    self.run_command(SessionCommand::Resume)
                } else {
                    self.run_command(SessionCommand::Pause)
                }
            }
            SessionCommand::Reset => {
                if let Err(error) = self.reset() {
                    log::warn!("reset failed: {error}");
                    SessionCommandOutcome::default()
                } else {
                    SessionCommandOutcome {
                        executed: true,
                        needs_redraw: false,
                    }
                }
            }
            SessionCommand::CreateSlot => {
                self.create_slot();
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::SaveActiveSlotOrNew => {
                self.save_active_slot_or_new();
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::LoadActiveSlot => {
                let was_paused = self.paused();
                let executed = self.load_active_slot();
                SessionCommandOutcome {
                    executed,
                    needs_redraw: redraw_needed_after_pause_change(
                        executed,
                        was_paused,
                        self.paused(),
                    ),
                }
            }
            SessionCommand::SelectActiveSlot(slot_id) => {
                self.select_active_slot(slot_id);
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::SaveSlot(slot_id) => {
                self.save_slot(slot_id, false);
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::LoadSlot(slot_id) => {
                let was_paused = self.paused();
                let executed = self.load_slot(slot_id);
                SessionCommandOutcome {
                    executed,
                    needs_redraw: redraw_needed_after_pause_change(
                        executed,
                        was_paused,
                        self.paused(),
                    ),
                }
            }
            SessionCommand::DeleteSlot(slot_id) => {
                self.delete_slot(slot_id);
                SessionCommandOutcome {
                    executed: true,
                    needs_redraw: false,
                }
            }
            SessionCommand::SelectNextSlot => SessionCommandOutcome {
                executed: self.select_adjacent_slot(true).is_some(),
                needs_redraw: false,
            },
            SessionCommand::SelectPreviousSlot => SessionCommandOutcome {
                executed: self.select_adjacent_slot(false).is_some(),
                needs_redraw: false,
            },
        }
    }

    pub fn select_active_slot(&mut self, slot_id: u64) {
        self.persistence.select_active_slot(slot_id);
    }

    pub fn select_adjacent_slot(&mut self, forward: bool) -> Option<u64> {
        let next_slot_id = adjacent_slot_id(
            self.persistence.slots(),
            self.persistence.active_slot_id(),
            forward,
        )?;
        self.persistence.select_active_slot(next_slot_id);
        Some(next_slot_id)
    }
}

pub(super) fn redraw_needed_after_pause_change(
    executed: bool,
    was_paused: bool,
    paused: bool,
) -> bool {
    executed && was_paused && !paused
}
