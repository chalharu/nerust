use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

use nerust_core_traits::identity::SystemIdentity;

use crate::{
    archive::{build_state_archive, load_state_archive, read_state_summary, summary_from_metadata},
    error::PersistenceError,
    fs_ops::write_atomic,
    metadata::encode_slot_metadata,
    model::{LoadedStateSlot, StateSlotSummary},
    thumbnail::{ThumbnailSource, encode_thumbnail_png},
    time::{system_time_from_millis, unix_millis},
};

const AUTOSAVE_SLOT_ENTRY: &str = ".autosave_slot";
const AUTOSAVE_SLOT_ID: u64 = 0;
const NEXT_SLOT_ID_ENTRY: &str = ".next_slot_id";

pub fn scan_state_slots(states_dir: &Path) -> Result<Vec<StateSlotSummary>, PersistenceError> {
    scan_state_slots_matching(states_dir, None)
}

pub fn scan_state_slots_for_identity(
    states_dir: &Path,
    identity: &SystemIdentity,
) -> Result<Vec<StateSlotSummary>, PersistenceError> {
    scan_state_slots_matching(states_dir, Some(identity))
}

pub fn allocate_next_slot_id(states_dir: &Path) -> Result<u64, PersistenceError> {
    fs::create_dir_all(states_dir)?;
    let counter_path = states_dir.join(NEXT_SLOT_ID_ENTRY);

    // Try to use the counter file with advisory locking if supported.
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&counter_path)
    {
        Ok(mut counter) => {
            match counter.lock() {
                Ok(()) => {
                    let next_slot_id = (read_next_slot_id(&mut counter)?.unwrap_or(1))
                        .max(existing_slot_id_max(states_dir)?.saturating_add(1))
                        .max(1);
                    write_next_slot_id(&mut counter, next_slot_id.saturating_add(1))?;
                    if let Err(e) = counter.unlock() {
                        log::warn!("allocate_next_slot_id: unlock() failed: {e}");
                    }
                    return Ok(next_slot_id);
                }
                Err(err) => {
                    log::warn!(
                        "allocate_next_slot_id: exclusive lock unavailable: {}; falling back to reservation",
                        err
                    );
                    // fall through to fallback reservation path
                }
            }
        }
        Err(err) => {
            log::warn!(
                "allocate_next_slot_id: failed to open counter file {}: {}; falling back to reservation",
                counter_path.display(),
                err
            );
        }
    }

    // Fallback: reserve an id by creating a per-id reservation file with O_EXCL.
    let start = existing_slot_id_max(states_dir)?.saturating_add(1).max(1);
    for id in start..start + 10000 {
        let reserve_path = states_dir.join(format!(".slot_reserve.{id}"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&reserve_path)
        {
            Ok(mut f) => {
                // record reservation info (pid, timestamp)
                use std::time::{SystemTime, UNIX_EPOCH};
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let _ = write!(f, "{} {}", std::process::id(), ts);
                let _ = f.sync_all();
                log::info!("allocate_next_slot_id: reserved slot {}", id);
                return Ok(id);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }

    Err(PersistenceError::Validation(
        "failed to allocate slot id via fallback reservation".into(),
    ))
}

pub fn state_slot_path(states_dir: &Path, slot_id: u64) -> PathBuf {
    states_dir.join(format!("{slot_id}.state"))
}

pub fn autosave_state_slot_path(states_dir: &Path) -> PathBuf {
    states_dir.join(AUTOSAVE_SLOT_ENTRY)
}

pub fn write_state_slot(
    states_dir: &Path,
    slot_id: u64,
    machine_state: &[u8],
    identity: &SystemIdentity,
    preview: Option<&ThumbnailSource>,
) -> Result<StateSlotSummary, PersistenceError> {
    write_state_slot_to_path(
        state_slot_path(states_dir, slot_id),
        slot_id,
        machine_state,
        identity,
        preview,
    )
}

pub fn write_autosave_state_slot(
    states_dir: &Path,
    machine_state: &[u8],
    identity: &SystemIdentity,
    preview: Option<&ThumbnailSource>,
) -> Result<StateSlotSummary, PersistenceError> {
    write_state_slot_to_path(
        autosave_state_slot_path(states_dir),
        AUTOSAVE_SLOT_ID,
        machine_state,
        identity,
        preview,
    )
}

pub fn load_state_slot_for_identity(
    path: &Path,
    identity: &SystemIdentity,
) -> Result<Option<LoadedStateSlot>, PersistenceError> {
    let archive = load_state_archive(path)?;
    if !crate::metadata::slot_matches_identity(&archive.metadata, identity) {
        return Ok(None);
    }
    Ok(Some(loaded_state_slot_from_archive(path, archive)))
}

fn write_state_slot_to_path(
    path: PathBuf,
    slot_id: u64,
    machine_state: &[u8],
    identity: &SystemIdentity,
    preview: Option<&ThumbnailSource>,
) -> Result<StateSlotSummary, PersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let saved_at = system_time_from_millis(unix_millis(SystemTime::now())?);
    let has_thumbnail = preview.is_some();
    let metadata = encode_slot_metadata(slot_id, saved_at, identity, has_thumbnail)?;
    let thumbnail_png = preview.map(encode_thumbnail_png).transpose()?;
    let archive_bytes = build_state_archive(&metadata, machine_state, thumbnail_png.as_deref())?;
    write_atomic(&path, &archive_bytes)?;

    // If we created a reservation file earlier, attempt to remove it now that the
    // real state file has been written. This keeps the storage directory tidy when
    // using the fallback reservation strategy on platforms that lack file locks.
    if let Some(parent) = path.parent() {
        let reserve_path = parent.join(format!(".slot_reserve.{}", slot_id));
        let _ = fs::remove_file(reserve_path);
    }

    Ok(summary_from_metadata(
        path,
        saved_at,
        &metadata,
        has_thumbnail,
    ))
}

pub fn load_state_slot(path: &Path) -> Result<LoadedStateSlot, PersistenceError> {
    let archive = load_state_archive(path)?;
    Ok(loaded_state_slot_from_archive(path, archive))
}

pub(crate) fn loaded_state_slot_from_archive(
    path: &Path,
    archive: crate::archive::LoadedArchive,
) -> LoadedStateSlot {
    let has_thumbnail = archive.thumbnail_png.is_some();
    let summary = summary_from_metadata(
        path.to_path_buf(),
        system_time_from_millis(archive.metadata.saved_at_unix_ms),
        &archive.metadata,
        has_thumbnail,
    );
    LoadedStateSlot {
        summary,
        machine_state: archive.machine_state,
        thumbnail_png: archive.thumbnail_png,
    }
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
    identity: Option<&SystemIdentity>,
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
        if let Ok(Some(summary)) = read_state_summary(&path, identity) {
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
