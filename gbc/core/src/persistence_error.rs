#[derive(Debug, thiserror::Error)]
pub enum GbcPersistenceError {
    #[error("persistence encode failed: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("persistence decode failed: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("unsupported persistence schema version: {0}")]
    UnsupportedVersion(u32),
    #[error("ROM identity mismatch")]
    RomIdentityMismatch,
    #[error("core options mismatch")]
    OptionsMismatch,
    #[error("invalid machine state: unsupported {component} version: {version}")]
    UnsupportedMapperVersion {
        component: &'static str,
        version: u32,
    },
    #[error(
        "invalid machine state: {component} kind mismatch: expected {expected:?}, got {actual:?}"
    )]
    MapperKindMismatch {
        component: &'static str,
        expected: crate::cartridge_mbc::MbcKind,
        actual: crate::cartridge_mbc::MbcKind,
    },
    #[error("invalid machine state: {0}")]
    InvalidState(String),
}

impl From<GbcPersistenceError> for nerust_core_traits::CoreError {
    fn from(error: GbcPersistenceError) -> Self {
        use nerust_core_traits::CoreFault;
        let kind = match &error {
            GbcPersistenceError::UnsupportedVersion(_)
            | GbcPersistenceError::RomIdentityMismatch
            | GbcPersistenceError::OptionsMismatch
            | GbcPersistenceError::UnsupportedMapperVersion { .. }
            | GbcPersistenceError::MapperKindMismatch { .. } => CoreFault::Incompatible,
            GbcPersistenceError::Encode(_)
            | GbcPersistenceError::Decode(_)
            | GbcPersistenceError::InvalidState(_) => CoreFault::Corrupt,
        };
        Self::Persistence {
            kind,
            source: Box::new(error),
        }
    }
}

// Device value validation is corruption, never compatibility inferred from its wording.
impl From<String> for GbcPersistenceError {
    fn from(message: String) -> Self {
        Self::InvalidState(message)
    }
}

impl From<&str> for GbcPersistenceError {
    fn from(message: &str) -> Self {
        Self::InvalidState(message.to_owned())
    }
}
