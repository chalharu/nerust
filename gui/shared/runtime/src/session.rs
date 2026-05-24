mod commands;
mod facade;
mod persistence;

use self::persistence::PersistenceState;
use nerust_gui_session::core::SessionCore;

#[derive(Debug)]
pub struct GuiSession {
    core: SessionCore,
    persistence: PersistenceState,
}

#[cfg(test)]
mod tests;
