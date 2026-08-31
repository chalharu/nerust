use thiserror::Error;

#[derive(Debug, Error)]
pub enum RomTestError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml parse error: {0}")]
    YamlParse(#[from] serde_saphyr::Error),
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("invalid GBA ROM: {0}")]
    InvalidRom(String),
}

impl RomTestError {
    pub fn category(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::YamlParse(_) | Self::InvalidManifest(_) => "config",
            Self::InvalidRom(_) => "rom",
        }
    }
}
