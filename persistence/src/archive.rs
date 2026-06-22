use crate::error::PersistenceError;
use crate::metadata::{
    METADATA_ENTRY, STATE_ENTRY, StateArchiveMetadata, THUMBNAIL_ENTRY, read_metadata,
    slot_matches_identity,
};
use crate::model::StateSlotSummary;
use crate::time::system_time_from_millis;
use nerust_contract_core::identity::SystemIdentity;
use std::fs::File;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const MAX_MACHINE_STATE_BYTES: usize = 64 * 1024 * 1024;
const MAX_THUMBNAIL_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct LoadedArchive {
    pub(crate) metadata: StateArchiveMetadata,
    pub(crate) machine_state: Vec<u8>,
    pub(crate) thumbnail_png: Option<Vec<u8>>,
}

pub(crate) fn read_state_summary(
    path: &Path,
    identity: Option<&SystemIdentity>,
) -> Result<Option<StateSlotSummary>, PersistenceError> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let metadata = read_metadata(&mut archive)?;
    if let Some(identity) = identity
        && !slot_matches_identity(&metadata, identity)
    {
        return Ok(None);
    }
    if archive.by_name(STATE_ENTRY).is_err() {
        return Err(PersistenceError::Validation(
            "state archive is missing machine state entry".into(),
        ));
    }
    let has_thumbnail = archive.by_name(THUMBNAIL_ENTRY).is_ok();
    Ok(Some(summary_from_metadata(
        path.to_path_buf(),
        system_time_from_millis(metadata.saved_at_unix_ms),
        &metadata,
        has_thumbnail,
    )))
}

pub(crate) fn load_state_archive(path: &Path) -> Result<LoadedArchive, PersistenceError> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let metadata = read_metadata(&mut archive)?;
    let machine_state = {
        let mut machine_state_file = archive.by_name(STATE_ENTRY)?;
        crate::fs_ops::read_limited(
            &mut machine_state_file,
            MAX_MACHINE_STATE_BYTES,
            "machine state",
        )?
    };
    let thumbnail_png = match archive.by_name(THUMBNAIL_ENTRY) {
        Ok(mut file) => Some(crate::fs_ops::read_limited(
            &mut file,
            MAX_THUMBNAIL_BYTES,
            "thumbnail",
        )?),
        Err(zip::result::ZipError::FileNotFound) => None,
        Err(error) => return Err(error.into()),
    };

    Ok(LoadedArchive {
        metadata,
        machine_state,
        thumbnail_png,
    })
}

pub(crate) fn build_state_archive(
    metadata: &StateArchiveMetadata,
    machine_state: &[u8],
    thumbnail_png: Option<&[u8]>,
) -> Result<Vec<u8>, PersistenceError> {
    let cursor = Cursor::new(Vec::<u8>::new());
    let mut writer = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer.start_file(METADATA_ENTRY, options)?;
    writer.write_all(&rmp_serde::to_vec_named(metadata)?)?;
    writer.start_file(STATE_ENTRY, options)?;
    writer.write_all(machine_state)?;
    if let Some(thumbnail_png) = thumbnail_png {
        writer.start_file(THUMBNAIL_ENTRY, options)?;
        writer.write_all(thumbnail_png)?;
    }
    Ok(writer.finish()?.into_inner())
}

pub(crate) fn summary_from_metadata(
    path: PathBuf,
    saved_at: SystemTime,
    metadata: &StateArchiveMetadata,
    has_thumbnail: bool,
) -> StateSlotSummary {
    StateSlotSummary {
        schema_version: metadata.schema_version,
        slot_id: metadata.slot_id,
        path,
        saved_at,
        has_thumbnail,
        emulator_version: metadata.emulator_version.clone(),
    }
}
