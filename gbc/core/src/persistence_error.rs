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
    #[error("invalid machine state: {0}")]
    InvalidState(String),
}
