//! Object-safe event boundary. Return owned native effects, never callbacks.

use crate::input::session::{SessionError, SessionEvent, SessionOutcome};
use crate::{input::commands::CommandRef, input::focus::FocusEvent};

#[derive(Debug, Clone, Copy)]
pub enum EventCommand {
    Menu(CommandRef),
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
    /// Modal guards (e.g. an open modal dialog) are enforced view-side;
    /// flavors forward every native callback unchanged.
    fn handle_event(&mut self, event: EventCommand) -> Result<EventOutcome<K>, SessionError>;
}

#[derive(Debug, Clone, Copy)]
pub struct EventOutcome<K: Copy + Eq + std::hash::Hash> {
    pub session: SessionOutcome,
    pub effect: Option<CommandEffect<K>>,
    /// False for an event suppressed by the view-side modal guard.
    pub handled: bool,
}

impl<K: Copy + Eq + std::hash::Hash> Default for EventOutcome<K> {
    fn default() -> Self {
        Self {
            session: SessionOutcome::default(),
            effect: None,
            // False for an event suppressed by the modal guard.
            handled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandEffect<K: Copy + Eq + std::hash::Hash> {
    /// Open a file (values like ROM wording arrive via [`crate::chrome::menu::FileDialogTitles`]).
    OpenFile,
    OpenAux(K),
    About,
    Quit,
    ToggleFullscreen,
    LiveTick,
}
