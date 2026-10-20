//! Machine-readable reason a state-slot save/load failed.
//! Home for [`SlotOpFailure`], shared by persistence, command, and notification layers.

use std::io::ErrorKind;

use nerust_persistence::error::PersistenceError;

/// Typed state-codec failure: transport vs undecodable vs rejected.
/// Single classification point; every layer converts through [`SlotOpFailure::from`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateCodecError {
    /// Core thread gone or never answered.
    Unavailable,
    /// Backend I/O failure.
    Storage,
    /// Missing slot file.
    Missing,
    /// Undecodable payload.
    Corrupt,
    /// Core rejected the payload.
    Incompatible,
}

impl From<&PersistenceError> for StateCodecError {
    fn from(error: &PersistenceError) -> Self {
        match error {
            PersistenceError::Io(error) if error.kind() == ErrorKind::NotFound => Self::Missing,
            PersistenceError::Io(_) => Self::Storage,
            _ => Self::Corrupt,
        }
    }
}

impl From<StateCodecError> for SlotOpFailure {
    fn from(error: StateCodecError) -> Self {
        match error {
            StateCodecError::Unavailable => Self::Unavailable,
            StateCodecError::Storage => Self::Storage,
            StateCodecError::Missing => Self::Missing,
            StateCodecError::Corrupt => Self::Corrupt,
            StateCodecError::Incompatible => Self::Incompatible,
        }
    }
}

impl From<crate::emu_core::StateFault> for SlotOpFailure {
    fn from(fault: crate::emu_core::StateFault) -> Self {
        use crate::emu_core::StateFault as F;
        match fault {
            F::Transport => Self::Unavailable,
            F::Storage => Self::Storage,
            F::Missing => Self::Missing,
            F::Corrupt => Self::Corrupt,
            F::Incompatible => Self::Incompatible,
            F::Other(_) => Self::Corrupt,
        }
    }
}

impl From<crate::emu_core::CorePersistenceError> for SlotOpFailure {
    fn from(error: crate::emu_core::CorePersistenceError) -> Self {
        SlotOpFailure::from(error.fault())
    }
}

impl crate::emu_core::CorePersistenceError {
    /// Compat shim: single mapping via [`StateFault`](crate::emu_core::StateFault).
    pub fn slot_failure(&self) -> SlotOpFailure {
        SlotOpFailure::from(self.fault())
    }
}

impl From<&PersistenceError> for SlotOpFailure {
    fn from(error: &PersistenceError) -> Self {
        StateCodecError::from(error).into()
    }
}

/// Why a state-slot save/load did not execute.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SlotOpFailure {
    /// No active slot (nothing saved yet, or no core loaded).
    Empty,
    /// The slot file does not exist.
    Missing,
    /// Storage/backend I/O failure (filesystem, Android SAF, ...).
    Storage,
    /// The slot archive or its payload does not decode.
    Corrupt,
    /// The core rejected the payload (foreign ROM, options or schema
    /// mismatch, invalid state).
    Incompatible,
    /// Cannot operate (no states dir, no media identity, core export
    /// or thread failure).
    #[default]
    Unavailable,
}
