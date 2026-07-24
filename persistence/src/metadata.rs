use std::{
    io::{Read, Seek},
    time::SystemTime,
};

use nerust_core_traits::identity::{SystemId, SystemIdentity};
use zip::ZipArchive;

use crate::{error::PersistenceError, time::unix_millis};

pub(crate) const METADATA_ENTRY: &str = "metadata.msgpack";
pub(crate) const STATE_ENTRY: &str = "state.bin";
pub(crate) const THUMBNAIL_ENTRY: &str = "thumbnail.png";
pub(crate) const STATE_ARCHIVE_SCHEMA_VERSION: u32 = 3;

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct StateArchiveMetadata {
    pub(crate) schema_version: u32,
    pub(crate) slot_id: u64,
    pub(crate) saved_at_unix_ms: u64,
    pub(crate) has_thumbnail: bool,
    pub(crate) system_id: Box<dyn SystemId>,
    #[serde(with = "serde_bytes")]
    pub(crate) identity_bytes: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub(crate) options_bytes: Vec<u8>,
    pub(crate) emulator_version: String,
}

// ---------------------------------------------------------------------------
// v2 read/write
// ---------------------------------------------------------------------------

pub(crate) fn read_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<StateArchiveMetadata, PersistenceError> {
    const MAX_METADATA_BYTES: usize = 64 * 1024;

    let mut metadata_file = archive.by_name(METADATA_ENTRY)?;
    let metadata_bytes =
        crate::fs_ops::read_limited(&mut metadata_file, MAX_METADATA_BYTES, "metadata")?;

    // Try v2.1
    if let Ok(meta) = rmp_serde::from_slice::<StateArchiveMetadata>(metadata_bytes.as_slice()) {
        if meta.schema_version == STATE_ARCHIVE_SCHEMA_VERSION {
            return Ok(meta);
        }
        return Err(PersistenceError::Validation(format!(
            "unsupported state archive schema version: {}",
            meta.schema_version
        )));
    }

    Err(PersistenceError::Validation(
        "unrecognized state archive metadata format".into(),
    ))
}

pub(crate) fn encode_slot_metadata(
    slot_id: u64,
    saved_at: SystemTime,
    identity: &SystemIdentity,
    has_thumbnail: bool,
) -> Result<StateArchiveMetadata, PersistenceError> {
    Ok(StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id,
        saved_at_unix_ms: unix_millis(saved_at)?,
        has_thumbnail,
        system_id: identity.system_id.clone(),
        identity_bytes: identity.identity_bytes.clone(),
        options_bytes: Vec::new(),
        emulator_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

pub(crate) fn slot_matches_identity(
    metadata: &StateArchiveMetadata,
    identity: &SystemIdentity,
) -> bool {
    metadata.system_id == identity.system_id && metadata.identity_bytes == identity.identity_bytes
}
