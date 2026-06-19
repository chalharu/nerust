mod commands;
mod facade;
mod persistence;

use self::persistence::PersistenceState;
use nerust_gui_session::core::SessionCore;
use nerust_input_schema::SystemId;

pub struct GuiSession {
    system_id: SystemId,
    core: SessionCore,
    persistence: PersistenceState,
}

#[cfg(test)]
mod tests;
