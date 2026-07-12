use std::path::PathBuf;

use super::LoadRequest;
use nerust_core_traits::factory::load::{MediaObject, SystemLoadOptions};

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
