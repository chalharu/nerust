use crate::archive::{
    build_state_archive, load_state_archive, read_state_summary, summary_from_metadata,
};
use crate::error::PersistenceError;
use crate::fs_ops::write_atomic;
use crate::metadata::encode_slot_metadata;
use crate::model::{LoadedStateSlot, StateSlotSummary};
use crate::thumbnail::{ThumbnailSource, encode_thumbnail_png};
use crate::time::{system_time_from_millis, unix_millis};
use fs2::FileExt;
use nerust_contract::PersistenceTarget;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const NEXT_SLOT_ID_ENTRY: &str = ".next_slot_id";

pub fn scan_state_slots(states_dir: &Path) -> Result<Vec<StateSlotSummary>, PersistenceError> {
    scan_state_slots_matching(states_dir, None)
}

pub fn scan_state_slots_for_target(
    states_dir: &Path,
    target: PersistenceTarget,
) -> Result<Vec<StateSlotSummary>, PersistenceError> {
    scan_state_slots_matching(states_dir, Some(target))
}

pub fn allocate_next_slot_id(states_dir: &Path) -> Result<u64, PersistenceError> {
    fs::create_dir_all(states_dir)?;
    let counter_path = states_dir.join(NEXT_SLOT_ID_ENTRY);
    let mut counter = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(counter_path)?;
    counter.lock_exclusive()?;
    let next_slot_id = (read_next_slot_id(&mut counter)?.unwrap_or(1))
        .max(existing_slot_id_max(states_dir)?.saturating_add(1))
        .max(1);
    write_next_slot_id(&mut counter, next_slot_id.saturating_add(1))?;
    counter.unlock()?;
    Ok(next_slot_id)
}

pub fn state_slot_path(states_dir: &Path, slot_id: u64) -> PathBuf {
    states_dir.join(format!("{slot_id}.state"))
}

pub fn write_state_slot(
    states_dir: &Path,
    slot_id: u64,
    machine_state: &[u8],
    target: PersistenceTarget,
    preview: Option<&ThumbnailSource>,
) -> Result<StateSlotSummary, PersistenceError> {
    fs::create_dir_all(states_dir)?;
    let saved_at = system_time_from_millis(unix_millis(SystemTime::now())?);
    let has_thumbnail = preview.is_some();
    let metadata = encode_slot_metadata(slot_id, saved_at, target, has_thumbnail)?;
    let thumbnail_png = preview.map(encode_thumbnail_png).transpose()?;
    let archive_bytes = build_state_archive(&metadata, machine_state, thumbnail_png.as_deref())?;
    let path = state_slot_path(states_dir, slot_id);
    write_atomic(&path, &archive_bytes)?;
    Ok(summary_from_metadata(
        path,
        saved_at,
        &metadata,
        has_thumbnail,
    ))
}

pub fn load_state_slot(path: &Path) -> Result<LoadedStateSlot, PersistenceError> {
    let archive = load_state_archive(path)?;
    let has_thumbnail = archive.thumbnail_png.is_some();
    let summary = summary_from_metadata(
        path.to_path_buf(),
        system_time_from_millis(archive.metadata.saved_at_unix_ms),
        &archive.metadata,
        has_thumbnail,
    );
    Ok(LoadedStateSlot {
        summary,
        machine_state: archive.machine_state,
        thumbnail_png: archive.thumbnail_png,
    })
}

pub fn delete_state_slot(path: &Path) -> Result<(), PersistenceError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn scan_state_slots_matching(
    states_dir: &Path,
    target: Option<PersistenceTarget>,
) -> Result<Vec<StateSlotSummary>, PersistenceError> {
    if !states_dir.exists() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(states_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("state") {
            continue;
        }
        if let Ok(Some(summary)) = read_state_summary(&path, target) {
            result.push(summary);
        }
    }
    result.sort_by_key(|slot| slot.slot_id);
    Ok(result)
}

fn existing_slot_id_max(states_dir: &Path) -> Result<u64, PersistenceError> {
    if !states_dir.exists() {
        return Ok(0);
    }
    let mut max_id = 0;
    for entry in fs::read_dir(states_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("state") {
            continue;
        }
        if let Some(slot_id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u64>().ok())
        {
            max_id = max_id.max(slot_id);
        }
    }
    Ok(max_id)
}

fn read_next_slot_id(file: &mut std::fs::File) -> Result<Option<u64>, PersistenceError> {
    file.seek(SeekFrom::Start(0))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    let value = buf.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        value
            .parse::<u64>()
            .map(Some)
            .map_err(|_| PersistenceError::Validation("invalid slot id counter".into()))
    }
}

fn write_next_slot_id(file: &mut std::fs::File, next_slot_id: u64) -> Result<(), PersistenceError> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    write!(file, "{next_slot_id}")?;
    file.sync_all()?;
    Ok(())
}
