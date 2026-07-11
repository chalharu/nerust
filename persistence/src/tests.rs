mod archive;
mod sidecar;
mod slots;
mod time;

use std::{env, fs, path::PathBuf, time::SystemTime};

use nerust_core_traits::identity::{SystemId, SystemIdentity};

use crate::{
    metadata::{STATE_ARCHIVE_SCHEMA_VERSION, StateArchiveMetadata},
    time::unix_millis,
};

fn prepare_test_dir(name: &str) -> PathBuf {
    let dir = test_dir(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn test_identity() -> SystemIdentity {
    SystemIdentity::new(SystemId::new("nes"), vec![1, 2, 3, 4])
}

fn test_identity_with_bytes(bytes: Vec<u8>) -> SystemIdentity {
    SystemIdentity::new(SystemId::new("nes"), bytes)
}

fn test_metadata(slot_id: u64, has_thumbnail: bool) -> StateArchiveMetadata {
    StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id,
        saved_at_unix_ms: unix_millis(SystemTime::now()).unwrap(),
        has_thumbnail,
        system_id: SystemId::new("nes"),
        identity_bytes: vec![1, 2, 3, 4],
        options_bytes: Vec::new(),
        emulator_version: "test".into(),
    }
}

fn test_dir(name: &str) -> PathBuf {
    env::current_dir()
        .unwrap()
        .join("target")
        .join("persistence-tests")
        .join(name)
}
