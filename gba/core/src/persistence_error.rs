/// Phase 10 persistence errors (mirrors `gbc` `persistence_error.rs`).
///
/// Wire errors (encode/decode/version/identity) and state errors (invalid
/// field values on import) stay distinct so the ConsoleCore boundary can map
/// them to `CoreError::Core` with context, while ROM problems keep the
/// `CoreError::RomParse` mapping.
#[derive(Debug, thiserror::Error)]
pub enum GbaPersistenceError {
    #[error("encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("unsupported schema version: {0}")]
    UnsupportedVersion(u32),
    #[error("ROM identity mismatch")]
    RomIdentityMismatch,
    #[error("core options mismatch")]
    OptionsMismatch,
    #[error("audio rate mismatch: state={0} backend={1}")]
    AudioRateMismatch(u32, u32),
    #[error("invalid state: {0}")]
    InvalidState(String),
    #[error("cartridge error: {0}")]
    Cartridge(String),
}

/// Load-time errors (ROM parse / input). Promoted from the private
/// `GbaCoreError` in `console_core.rs` so both entry points share them.
#[derive(Debug, thiserror::Error)]
pub enum GbaLoadError {
    #[error("invalid or unsupported GBA ROM")]
    InvalidRom,
    #[error("GBA input buffer has the wrong concrete type")]
    InvalidInputBuffer,
    #[error("save state ROM mismatch")]
    RomMismatch,
}
