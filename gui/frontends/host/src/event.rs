//! Object-safe event boundary. Return owned native effects, never callbacks.

use crate::{
    commands::MenuCommand,
    focus::{AuxKind, FocusEvent},
    session::{SessionError, SessionEvent, SessionOutcome},
};

#[derive(Debug, Clone, Copy)]
pub enum EventCommand {
    Menu(MenuCommand),
    Session(SessionEvent),
    Focus(FocusEvent),
    LiveTick,
}

/// False means the transport is dead. Rc-based native transports are welcome;
/// neither Send nor Sync is imposed on the UI boundary.
pub trait EventSink {
    fn send(&self, event: EventCommand) -> bool;
}

pub trait EventHandler {
    fn handle_event(
        &mut self,
        event: EventCommand,
        settings_open: bool,
    ) -> Result<EventOutcome, SessionError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct EventOutcome {
    pub session: SessionOutcome,
    pub effect: Option<CommandEffect>,
    /// False for an event suppressed by settings policy.
    pub handled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandEffect {
    OpenRom,
    OpenAux(AuxKind),
    About,
    Quit,
    ToggleFullscreen,
    LiveTick,
}
