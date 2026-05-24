mod commands;
mod persistence;

use self::persistence::PersistenceState;
use nerust_console::video::ConsoleVideo;
use nerust_console::{ConsoleError, ConsoleMetrics, NesInputFrame};
use nerust_gui_session::core::{SessionCore, WindowSize};
use nerust_gui_session::title::window_title;
use nerust_persistence::model::StateSlotSummary;

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

    pub fn apply_nes_input_frame(&mut self, frame: NesInputFrame) {
        self.core.apply_nes_input_frame(frame);
    }

    pub fn current_nes_input_frame(&self) -> NesInputFrame {
        self.core.current_nes_input_frame()
    }

    pub fn clear_nes_input_frame(&mut self) {
        self.core.clear_nes_input_frame();
    }
}

#[cfg(test)]
mod tests;
