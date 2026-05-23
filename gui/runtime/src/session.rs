mod persistence;

use self::persistence::PersistenceState;
use crate::StateSlotSummary;
use crate::slots::adjacent_slot_id;
use nerust_console::{ConsoleMetrics, ControllerInputs};
use nerust_gui_session::{
    ConsoleError, ConsoleVideo, ControllerPort, SessionCommand, SessionCommandOutcome, SessionCore,
    WindowSize, window_title,
};

pub trait ConsoleSessionFactory {
    fn build_session(&self) -> GuiSession;
}

#[derive(Debug)]
pub struct GuiSession {
    core: SessionCore,
    persistence: PersistenceState,
}

impl GuiSession {
    pub fn from_session_core(core: SessionCore) -> Self {
        Self {
            core,
            persistence: PersistenceState::default(),
        }
    }

    pub fn video(&self) -> &ConsoleVideo {
        self.core.video()
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        self.core.with_frame_buffer(f)
    }

    pub fn window_size(&self) -> WindowSize {
        self.core.window_size()
    }

    pub fn metrics(&self) -> ConsoleMetrics {
        self.core.metrics()
    }

    pub fn window_title(&self) -> String {
        window_title(self.paused(), self.metrics())
    }

    pub fn paused(&self) -> bool {
        self.core.paused()
    }

    pub fn loaded(&self) -> bool {
        self.core.loaded()
    }

    pub fn can_pause(&self) -> bool {
        self.core.can_pause()
    }

    pub fn can_resume(&self) -> bool {
        self.core.can_resume()
    }

    pub fn slots(&self) -> &[StateSlotSummary] {
        self.persistence.slots()
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.persistence.active_slot_id()
    }

    pub fn reset(&self) -> Result<(), ConsoleError> {
        self.core.reset()
    }

    pub fn pause(&mut self) {
        self.core.pause();
    }

    pub fn resume(&mut self) {
        self.core.resume();
    }

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

    pub fn set_port_inputs(&mut self, port: ControllerPort, inputs: ControllerInputs) {
        self.core.set_port_inputs(port, inputs);
    }

    pub fn clear_all_inputs(&mut self) {
        self.core.clear_all_inputs();
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

fn redraw_needed_after_pause_change(executed: bool, was_paused: bool, paused: bool) -> bool {
    executed && was_paused && !paused
}

#[cfg(test)]
mod tests {
    use super::{GuiSession, redraw_needed_after_pause_change};
    use crate::{SessionCore, window_title};
    use nerust_console::{Console, ConsoleMetrics};
    use nerust_screen_buffer::ScreenBuffer;
    use nerust_sound_traits::{MixerInput, Sound};

    #[derive(Default)]
    struct TestSpeaker;

    impl Sound for TestSpeaker {
        fn start(&mut self) {}

        fn pause(&mut self) {}
    }

    impl MixerInput for TestSpeaker {
        fn push(&mut self, _: f32) {}
    }

    fn test_session() -> GuiSession {
        GuiSession::from_session_core(SessionCore::from_console(Console::new(
            TestSpeaker,
            ScreenBuffer::new_nes_gpu_default(),
        )))
    }

    #[test]
    fn window_title_surfaces_runtime_metrics() {
        let title = window_title(
            false,
            ConsoleMetrics {
                loaded: true,
                emulation_fps: 59.9,
                speed_multiplier: 1.01,
                ..ConsoleMetrics::default()
            },
        );

        assert!(title.contains("FPS 59.9"));
        assert!(title.contains("Speed x1.01"));
    }

    #[test]
    fn window_title_marks_no_rom() {
        assert!(window_title(true, ConsoleMetrics::default()).contains("Paused"));
        assert!(window_title(true, ConsoleMetrics::default()).contains("No ROM"));
    }

    #[test]
    fn redraw_is_only_requested_when_a_command_resumes_emulation() {
        assert!(redraw_needed_after_pause_change(true, true, false));
        assert!(!redraw_needed_after_pause_change(true, false, false));
        assert!(!redraw_needed_after_pause_change(true, true, true));
        assert!(!redraw_needed_after_pause_change(false, true, false));
    }

    #[test]
    fn test_session_builds_gui_session() {
        let session = test_session();

        assert!(!session.loaded());
        assert!(session.paused());
        assert!(session.window_size().width > 0.0);
    }
}
