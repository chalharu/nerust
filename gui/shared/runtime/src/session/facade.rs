use super::GuiSession;
use super::persistence::PersistenceState;
use nerust_console::video::ConsoleVideo;
use nerust_console::{ConsoleError, ConsoleMetrics};
use nerust_gui_session::core::{SessionCore, WindowSize};
use nerust_gui_session::title::window_title;
use nerust_persistence::model::StateSlotSummary;

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

    pub fn apply_controller_state(&mut self, bytes: Vec<u8>) -> Result<(), ConsoleError> {
        self.core.apply_controller_state(bytes)
    }

    pub fn apply_input_state(&mut self, bytes: Vec<u8>) {
        self.core.apply_input_state(bytes);
    }

    pub fn current_controller_state(&self) -> Result<Vec<u8>, ConsoleError> {
        self.core.current_controller_state()
    }

    pub fn current_input_state(&self) -> Result<Vec<u8>, ConsoleError> {
        self.core.current_input_state()
    }
}
