//! Machine-readable reason a state-slot save/load failed.
//! Home for [`SlotOpFailure`], shared by persistence, command, and notification layers.

use std::io::ErrorKind;

use nerust_persistence::error::PersistenceError;

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

impl From<&PersistenceError> for SlotOpFailure {
    fn from(error: &PersistenceError) -> Self {
        match error {
            PersistenceError::Io(error) if error.kind() == ErrorKind::NotFound => Self::Missing,
            PersistenceError::Io(_) => Self::Storage,
            _ => Self::Corrupt,
        }
    }
}

#[cfg(test)]
mod error_cov_tests {
    use crate::session::error::*;
    use std::io::{Error, ErrorKind};

    #[test]
    fn codec_maps_not_found_to_missing() {
        let err = PersistenceError::Io(Error::new(ErrorKind::NotFound, "gone"));
        assert_eq!(SlotOpFailure::from(&err), SlotOpFailure::Missing);
    }

    #[test]
    fn codec_maps_other_io_to_storage() {
        let err = PersistenceError::Io(Error::other("disk down"));
        assert_eq!(SlotOpFailure::from(&err), SlotOpFailure::Storage);
    }

    #[test]
    fn codec_maps_non_io_to_corrupt() {
        let err = PersistenceError::Validation("bad".into());
        assert_eq!(SlotOpFailure::from(&err), SlotOpFailure::Corrupt);
    }

    #[test]
    fn failure_defaults_to_unavailable() {
        assert_eq!(SlotOpFailure::default(), SlotOpFailure::Unavailable);
    }
}
