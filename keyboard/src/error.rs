#![cfg(feature = "error")]

use thiserror::Error;

#[derive(Debug, Error)]
pub enum KeyboardError {
    #[error("Failed to convert key code: {0}")]
    KeyCodeConversionError(String),
}
