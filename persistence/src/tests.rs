mod archive;
mod sidecar;
mod slots;
mod time;

use crate::metadata::{STATE_ARCHIVE_SCHEMA_VERSION, StateArchiveMetadata};

use crate::time::unix_millis;
use nerust_contract_core::identity::SystemIdentity;
use nerust_contract_input::SystemId;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

fn prepare_test_dir(name: &str) -> PathBuf {
    let dir = test_dir(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn test_identity() -> SystemIdentity {
    SystemIdentity::new(SystemId::Nes, vec![1, 2, 3, 4])
}

fn test_identity_with_bytes(bytes: Vec<u8>) -> SystemIdentity {
    SystemIdentity::new(SystemId::Nes, bytes)
}

fn test_metadata(slot_id: u64, has_thumbnail: bool) -> StateArchiveMetadata {
    StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id,
        saved_at_unix_ms: unix_millis(SystemTime::now()).unwrap(),
        has_thumbnail,
        system_id: SystemId::Nes,
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
