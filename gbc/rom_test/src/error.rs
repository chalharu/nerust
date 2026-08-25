use thiserror::Error;

#[derive(Debug, Error)]
pub enum RomTestError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("YAML parse error: {0}")]
    YamlParse(#[from] serde_saphyr::Error),

    #[error("PNG encoding error: {0}")]
    PngEncoding(#[from] png::EncodingError),

    #[error("PNG decoding error: {0}")]
    PngDecoding(#[from] png::DecodingError),
}

impl RomTestError {
    /// Coarse category for grouping failures in reports.
    pub fn category(&self) -> &'static str {
        match self {
            RomTestError::Io(_) => "io",
            RomTestError::InvalidManifest(_) => "config",
            RomTestError::YamlParse(_) => "config",
            RomTestError::PngEncoding(_) => "png",
            RomTestError::PngDecoding(_) => "png",
        }
    }
}
