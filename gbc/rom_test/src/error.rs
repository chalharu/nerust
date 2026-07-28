use thiserror::Error;

#[derive(Debug, Error)]
pub enum RomTestError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("ROM case `{0}` failed: {1}")]
    CaseFailed(String, String),

    #[error("unexpected serial output for `{0}`")]
    SerialMismatch(String),

    #[error("unexpected frame hash for `{0}`")]
    FrameMismatch(String),

    #[error("unexpected audio hash for `{0}`")]
    AudioMismatch(String),

    #[error("YAML parse error: {0}")]
    YamlParse(#[from] serde_saphyr::Error),

    #[error("PNG encoding error: {0}")]
    PngEncoding(#[from] png::EncodingError),
}
