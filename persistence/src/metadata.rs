mod legacy_nes;

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

#[derive(serde::Deserialize)]
struct SchemaVersion {
    schema_version: u32,
}

// ---------------------------------------------------------------------------
// Current read/write
// ---------------------------------------------------------------------------

pub(crate) fn read_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<StateArchiveMetadata, PersistenceError> {
    const MAX_METADATA_BYTES: usize = 64 * 1024;

    let mut metadata_file = archive.by_name(METADATA_ENTRY)?;
    let metadata_bytes =
        crate::fs_ops::read_limited(&mut metadata_file, MAX_METADATA_BYTES, "metadata")?;

    let version = rmp_serde::from_slice::<SchemaVersion>(&metadata_bytes).map_err(|_| {
        PersistenceError::Validation("unrecognized state archive metadata format".into())
    })?;
    match version.schema_version {
        STATE_ARCHIVE_SCHEMA_VERSION => match rmp_serde::from_slice(&metadata_bytes) {
            Ok(metadata) => Ok(metadata),
            Err(error) => legacy_nes::decode_mistagged_v3(&metadata_bytes)?
                .ok_or_else(|| PersistenceError::from(error)),
        },
        2 => legacy_nes::decode_v2(&metadata_bytes),
        1 => legacy_nes::decode_v1(&metadata_bytes),
        version => Err(PersistenceError::Validation(format!(
            "unsupported state archive schema version: {version}"
        ))),
    }
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
    (metadata.system_id == identity.system_id
        || legacy_nes::matches_system_id(metadata.system_id.as_ref(), identity.system_id.as_ref()))
        && metadata.identity_bytes == identity.identity_bytes
}
