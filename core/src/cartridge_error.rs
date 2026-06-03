#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CartridgeError {
    #[error("data integrity error in data")]
    DataError,
    #[error("file ends unexpectedly")]
    UnexpectedEof,
    #[allow(dead_code)]
    #[error("unexpected error")]
    Unexpected,
}
