mod archive;
mod sidecar;
mod slots;
mod time;

use std::{any::TypeId, env, fs, hash::Hash, path::PathBuf, time::SystemTime};

use nerust_core_traits::identity::{SystemId, SystemIdentity};
use serde::{Deserialize, Serialize};

use crate::{
    metadata::{STATE_ARCHIVE_SCHEMA_VERSION, StateArchiveMetadata},
    time::unix_millis,
};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct DummySystemId;

#[typetag::serde]
impl SystemId for DummySystemId {}

impl ToString for DummySystemId {
    fn to_string(&self) -> String {
        "dummy".to_string()
    }
}

impl Hash for DummySystemId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        TypeId::of::<Self>().hash(state);
    }
}

fn prepare_test_dir(name: &str) -> PathBuf {
    let dir = test_dir(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn test_identity() -> SystemIdentity {
    SystemIdentity::new(Box::new(DummySystemId), vec![1, 2, 3, 4])
}

fn test_identity_with_bytes(bytes: Vec<u8>) -> SystemIdentity {
    SystemIdentity::new(Box::new(DummySystemId), bytes)
}

fn test_metadata(slot_id: u64, has_thumbnail: bool) -> StateArchiveMetadata {
    StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id,
        saved_at_unix_ms: unix_millis(SystemTime::now()).unwrap(),
        has_thumbnail,
        system_id: Some(Box::new(DummySystemId)),
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
