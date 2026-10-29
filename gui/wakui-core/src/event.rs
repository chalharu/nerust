//! Object-safe event boundary. Return owned native effects, never callbacks.

use crate::session::{SessionError, SessionEvent, SessionOutcome};
use crate::{commands::MenuCommand, focus::FocusEvent};

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

pub trait EventHandler<K: Copy + Eq + std::hash::Hash> {
    fn handle_event(
        &mut self,
        event: EventCommand,
        settings_open: bool,
    ) -> Result<EventOutcome<K>, SessionError>;
}

#[derive(Debug, Clone, Copy)]
pub struct EventOutcome<K: Copy + Eq + std::hash::Hash> {
    pub session: SessionOutcome,
    pub effect: Option<CommandEffect<K>>,
    /// False for an event suppressed by settings policy.
    pub handled: bool,
}

impl<K: Copy + Eq + std::hash::Hash> Default for EventOutcome<K> {
    fn default() -> Self {
        Self {
            session: SessionOutcome::default(),
            effect: None,
            // False for an event suppressed by settings policy.
            handled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandEffect<K: Copy + Eq + std::hash::Hash> {
    OpenRom,
    OpenAux(K),
    About,
    Quit,
    ToggleFullscreen,
    LiveTick,
}
