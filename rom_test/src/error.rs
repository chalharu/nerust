use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum RomTestError {
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse YAML manifest {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: Box<serde_saphyr::Error>,
    },
    #[error("invalid ROM manifest: {0}")]
    InvalidManifest(String),
    #[error("failed to construct emulator core for {case_id}: {message}")]
    CoreConstruction { case_id: String, message: String },
    #[error("failed to encode screenshot: {0}")]
    ScreenshotEncoding(#[from] png::EncodingError),
}
