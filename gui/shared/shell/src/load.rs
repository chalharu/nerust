use nerust_input_schema::SystemId;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemLoadOptions {
    pub options_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadRequest {
    Auto,
    Explicit {
        system_id: SystemId,
        options: SystemLoadOptions,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLoadRequest {
    pub system_id: SystemId,
    pub options: SystemLoadOptions,
    pub core_options_bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::{LoadRequest, MediaObject, SystemLoadOptions};
    use nerust_input_schema::SystemId;
    use std::path::PathBuf;

    #[test]
    fn media_object_tracks_path_extension() {
        let media = MediaObject::new(Some(PathBuf::from("/tmp/test.NES")), vec![1, 2, 3]);

        assert_eq!(media.extension.as_deref(), Some("nes"));
        assert_eq!(media.bytes.as_ref(), [1, 2, 3]);
    }

    #[test]
    fn explicit_load_requests_preserve_selected_system() {
        assert_eq!(
            LoadRequest::Explicit {
                system_id: SystemId::Nes,
                options: SystemLoadOptions::default(),
            },
            LoadRequest::Explicit {
                system_id: SystemId::Nes,
                options: SystemLoadOptions::default(),
            }
        );
    }
}
