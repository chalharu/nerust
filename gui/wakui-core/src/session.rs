//! Framework input vocabulary: opaque commands and keys in, outcomes out.
//!
//! View definitions and native frontends share these boundary types. Native
//! adapters translate callbacks into these intents without naming shell or
//! view types; meaning (key bindings, modal guards, suspension adoption,
//! aux routing) resolves view-side, the same direction as
//! [`crate::commands`]: new meanings are added outside this file, never as
//! new enum variants here.

use crate::Key;
use crate::commands::CommandRef;

/// Toolkit-independent input event. Native adapters translate callbacks
/// into these intents; policy and ordering live view-side.
#[derive(Debug, Clone, Copy)]
pub enum SessionEvent {
    Command(CommandRef),
    Keyboard {
        key: Key,
        pressed: bool,
    },
    /// Reset held-input state (e.g. on window close or content replacement).
    ClearInput,
}

/// Command execution stripped to what native adapters observe.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandOutcome {
    pub executed: bool,
    pub needs_redraw: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionOutcome {
    pub command: Option<CommandOutcome>,
}

/// Boundary error: the domain cause rendered once, never matched on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError(pub String);

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SessionError {}
