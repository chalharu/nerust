use thiserror::Error;

#[derive(Debug, Error)]
pub enum RomTestError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml parse error: {0}")]
    YamlParse(#[from] serde_saphyr::Error),
}
