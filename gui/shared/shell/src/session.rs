pub mod input;
mod lifecycle;
#[cfg(test)]
mod tests;

use nerust_gui_runtime::session::GuiSession;
use nerust_input_nes::input::NesInputState;

#[derive(Debug)]
pub struct NesSession {
    pub(super) session: GuiSession,
    pub(super) input: NesInputState,
}

impl Default for NesSession {
    fn default() -> Self {
        Self::new()
    }
}
