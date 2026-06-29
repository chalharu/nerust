use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Clone)]
pub struct MediaObject {
    pub bytes: Arc<[u8]>,
    pub path: Option<PathBuf>,
    pub extension: Option<String>,
}

impl MediaObject {
    pub fn new(path: Option<PathBuf>, data: Vec<u8>) -> Self {
        let extension = path
            .as_deref()
            .and_then(|p| p.extension())
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());
        Self {
            bytes: Arc::from(data),
            path,
            extension,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemLoadOptions {
    pub options_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLoadRequest {
    pub options: SystemLoadOptions,
    pub core_options_bytes: Vec<u8>,
}
