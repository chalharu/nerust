use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::session::SessionHandle;

#[derive(Debug)]
pub struct RomLoaderError(pub String);

impl std::fmt::Display for RomLoaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub trait RomLoader {
    fn load_rom(&self, path: &Path, session: &mut SessionHandle) -> Result<(), RomLoaderError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaObject {
    pub bytes: Arc<[u8]>,
    pub path: Option<PathBuf>,
    pub extension: Option<String>,
}

impl MediaObject {
    pub fn new(path: Option<PathBuf>, data: Vec<u8>) -> Self {
        let extension = path
            .as_deref()
            .and_then(Path::extension)
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase());
        Self {
            bytes: Arc::from(data),
            path,
            extension,
        }
    }
}

/// System-specific load options, opaque to the shell.
///
/// The contents are interpreted by the `CoreFactory` implementation.
/// For NES: serialized `CoreOptions` bytes for the emulator core.
/// For other systems: defined by their respective factory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemLoadOptions {
    /// Opaque blob; contract between frontend and CoreFactory.
    pub options_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadRequest {
    Auto,
    Explicit { options: SystemLoadOptions },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLoadRequest {
    pub options: SystemLoadOptions,
    /// Opaque options blob for the emulator core.
    /// Interpreted by the CoreFactory / system core implementation.
    pub core_options_bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{LoadRequest, MediaObject, SystemLoadOptions};

    #[test]
    fn media_object_tracks_path_extension() {
        let media = MediaObject::new(Some(PathBuf::from("/tmp/test.NES")), vec![1, 2, 3]);

        assert_eq!(media.extension.as_deref(), Some("nes"));
        assert_eq!(media.bytes.as_ref(), [1, 2, 3]);
    }

    #[test]
    fn explicit_load_requests_preserve_options() {
        assert_eq!(
            LoadRequest::Explicit {
                options: SystemLoadOptions::default(),
            },
            LoadRequest::Explicit {
                options: SystemLoadOptions::default(),
            }
        );
    }
}
