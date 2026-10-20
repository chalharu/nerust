//! Machine-readable reason a state-slot save/load failed.
//! Home for [`SlotOpFailure`], shared by persistence, command, and notification layers.

use std::io::ErrorKind;

use nerust_persistence::error::PersistenceError;

/// Re-exported from the fault home; error mapping stays here, core mapping in [`emu_core`](crate::emu_core).
pub use crate::emu_core::SlotOpFailure;

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

impl From<&PersistenceError> for SlotOpFailure {
    fn from(error: &PersistenceError) -> Self {
        StateCodecError::from(error).into()
    }
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
}
