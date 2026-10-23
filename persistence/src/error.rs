use thiserror::Error;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error: {0}")]
    Zip(zip::result::ZipError),
    #[error("PNG encoding error: {0}")]
    Png(#[from] png::EncodingError),
    #[error("msgpack decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("msgpack encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("mapper save export failed: {0}")]
    Export(String),
    #[error("invalid state archive: {0}")]
    Validation(String),
}

impl From<zip::result::ZipError> for PersistenceError {
    fn from(error: zip::result::ZipError) -> Self {
        // Transport failure inside the archive stays retryable instead of
        // collapsing into corruption: a transient seek/read must not trigger
        // quarantine or delete paths (NER-321).
        match error {
            zip::result::ZipError::Io(error) => Self::Io(error),
            error => Self::Zip(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NER-321: transport failure inside the archive stays an I/O error so
    /// retryable paths never mistake it for confirmed corruption.
    #[test]
    fn zip_transport_failure_stays_retryable_io() {
        let io = std::io::Error::new(std::io::ErrorKind::TimedOut, "transient");
        let mapped = PersistenceError::from(zip::result::ZipError::Io(io));
        assert!(matches!(mapped, PersistenceError::Io(_)));
        let corrupt = PersistenceError::from(zip::result::ZipError::FileNotFound);
        assert!(matches!(corrupt, PersistenceError::Zip(_)));
    }
}
