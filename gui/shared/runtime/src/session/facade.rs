use super::GuiSession;
use super::persistence::PersistenceState;
use nerust_console::state::RuntimeStateExport;
use nerust_console::video::ConsoleVideo;
use nerust_console::{ConsoleError, ConsoleMetrics};
use nerust_contract_core::persistence::CanonicalMediaIdentity;
use nerust_gui_session::core::{SessionCore, WindowSize};
use nerust_gui_session::title::window_title;
use nerust_persistence::model::StateSlotSummary;

impl GuiSession {
    pub fn from_session_core(core: SessionCore) -> Self {
        Self {
            system_id: nerust_input_schema::SystemId::Nes,
            core,
            persistence: PersistenceState::default(),
        }
    }

    pub fn video(&self) -> &ConsoleVideo {
        self.core.video()
    }

    pub fn with_frame_buffer<T>(&self, f: impl FnOnce(&[u8]) -> T) -> T {
        f(self.core.frame_buffer().as_ref())
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

    pub fn export_state(&self) -> Result<RuntimeStateExport, ConsoleError> {
        self.core.export_state()
    }

    pub fn import_state(&mut self, bytes: Vec<u8>) -> Result<(), ConsoleError> {
        self.core.import_state(bytes)
    }

    pub fn canonical_media_identity(&self) -> Result<CanonicalMediaIdentity, ConsoleError> {
        self.core.canonical_media_identity()
    }
}
