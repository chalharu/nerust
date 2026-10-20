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

#[cfg(test)]
mod error_cov_tests {
    use crate::session::error::*;
    use std::io::{Error, ErrorKind};

    #[test]
    fn codec_maps_not_found_to_missing() {
        let err = PersistenceError::Io(Error::new(ErrorKind::NotFound, "gone"));
        assert_eq!(StateCodecError::from(&err), StateCodecError::Missing);
        assert_eq!(SlotOpFailure::from(&err), SlotOpFailure::Missing);
    }

    #[test]
    fn codec_maps_other_io_to_storage() {
        let err = PersistenceError::Io(Error::other("disk down"));
        assert_eq!(StateCodecError::from(&err), StateCodecError::Storage);
        assert_eq!(SlotOpFailure::from(&err), SlotOpFailure::Storage);
    }

    #[test]
    fn codec_maps_non_io_to_corrupt() {
        let err = PersistenceError::Validation("bad".into());
        assert_eq!(StateCodecError::from(&err), StateCodecError::Corrupt);
        assert_eq!(SlotOpFailure::from(&err), SlotOpFailure::Corrupt);
    }

    #[test]
    fn codec_round_trips_every_variant() {
        for (codec, failure) in [
            (StateCodecError::Unavailable, SlotOpFailure::Unavailable),
            (StateCodecError::Storage, SlotOpFailure::Storage),
            (StateCodecError::Missing, SlotOpFailure::Missing),
            (StateCodecError::Corrupt, SlotOpFailure::Corrupt),
            (StateCodecError::Incompatible, SlotOpFailure::Incompatible),
        ] {
            assert_eq!(SlotOpFailure::from(codec), failure);
        }
        assert_eq!(SlotOpFailure::default(), SlotOpFailure::Unavailable);
    }

    #[test]
    fn fault_maps_every_arm() {
        use crate::emu_core::StateFault as F;
        for (fault, failure) in [
            (F::Transport, SlotOpFailure::Unavailable),
            (F::Storage, SlotOpFailure::Storage),
            (F::Missing, SlotOpFailure::Missing),
            (F::Corrupt, SlotOpFailure::Corrupt),
            (F::Incompatible, SlotOpFailure::Incompatible),
            (F::Other("x".into()), SlotOpFailure::Corrupt),
        ] {
            assert_eq!(SlotOpFailure::from(fault), failure);
        }
    }

    #[test]
    fn core_error_shim_matches_fault() {
        use crate::emu_core::CorePersistenceError as E;
        assert_eq!(
            SlotOpFailure::from(E::WorkerUnavailable),
            SlotOpFailure::Unavailable
        );
        assert_eq!(SlotOpFailure::from(E::NoReply), SlotOpFailure::Unavailable);
        assert_eq!(
            E::Core("slot file not found".into()).slot_failure(),
            SlotOpFailure::Missing
        );
    }
}
