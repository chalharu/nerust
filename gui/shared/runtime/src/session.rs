mod commands;
mod facade;
mod persistence;

use self::persistence::PersistenceState;
use crate::settings::DesktopSettingsManager;
use nerust_gui_session::core::SessionCore;

#[derive(Debug)]
pub struct GuiSession {
    core: SessionCore,
    persistence: PersistenceState,
    settings: DesktopSettingsManager,
}

#[cfg(test)]
mod tests;
