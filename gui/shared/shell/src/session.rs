pub mod input;
mod lifecycle;
#[cfg(test)]
mod tests;

use nerust_gui_runtime::session::GuiSession;
use nerust_gui_runtime::settings::DesktopSettingsManager;
use nerust_input_nes::input::state::NesInputState;

#[derive(Debug)]
pub struct NesSession {
    pub(super) session: GuiSession,
    pub(super) input: NesInputState,
    pub(super) settings: DesktopSettingsManager,
}

impl Default for NesSession {
    fn default() -> Self {
        Self::new(crate::settings::defaults::manager::load_settings_manager())
    }
}
