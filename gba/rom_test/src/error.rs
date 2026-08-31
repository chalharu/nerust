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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_errors() {
        assert_eq!(
            RomTestError::InvalidManifest("x".into()).category(),
            "config"
        );
        assert_eq!(RomTestError::InvalidRom("x".into()).category(), "rom");
        let io = std::io::Error::other("x");
        assert_eq!(RomTestError::from(io).category(), "io");
    }
}
