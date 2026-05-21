// Copyright (c) 2024 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Persistence-side archive and sidecar handling.
//!
//! This crate owns only the outer save-slot archive schema and file-system behavior. `state.bin`
//! stores opaque console state bytes, and the nested core compatibility checks remain owned by the
//! console/core crates during import. Bump `STATE_ARCHIVE_SCHEMA_VERSION` only when archive entry
//! names, metadata fields, or this crate's validation/interpretation rules change.

use fs2::FileExt;
use nerust_core::{CoreOptions, MirrorMode, Mmc3IrqVariant, RomFormat, RomIdentity};
use png::{BitDepth, ColorType, Encoder};
use std::fs::{self, File, OpenOptions};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const METADATA_ENTRY: &str = "metadata.msgpack";
const STATE_ENTRY: &str = "state.bin";
const THUMBNAIL_ENTRY: &str = "thumbnail.png";
const THUMBNAIL_TARGET_WIDTH: u32 = 320;
/// Compatibility version for the zip archive metadata/layout owned by this crate.
const STATE_ARCHIVE_SCHEMA_VERSION: u32 = 1;
const NEXT_SLOT_ID_ENTRY: &str = ".next_slot_id";
const MAX_METADATA_BYTES: usize = 64 * 1024;
const MAX_MAPPER_SAVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_MACHINE_STATE_BYTES: usize = 64 * 1024 * 1024;
const MAX_THUMBNAIL_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("PNG encoding error: {0}")]
    Png(#[from] png::EncodingError),
    #[error("msgpack decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("msgpack encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("invalid state archive: {0}")]
    Validation(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarPaths {
    pub mapper_save_path: PathBuf,
    pub states_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSlotSummary {
    pub schema_version: u32,
    pub slot_id: u64,
    pub path: PathBuf,
    pub saved_at: SystemTime,
    pub has_thumbnail: bool,
    pub emulator_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedStateSlot {
    pub summary: StateSlotSummary,
    pub machine_state: Vec<u8>,
    pub thumbnail_png: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailSource {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Archive metadata owned by the persistence crate.
///
/// The metadata duplicates the persistence target needed for slot filtering, while `state.bin`
/// remains an opaque console payload. Thumbnail bytes are handled as an opaque PNG blob here; this
/// crate tracks presence and storage, but does not interpret image contents during load.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct StateArchiveMetadata {
    schema_version: u32,
    slot_id: u64,
    saved_at_unix_ms: u64,
    has_thumbnail: bool,
    mapper_type: u32,
    sub_mapper_type: u32,
    prg_rom_crc64: u64,
    chr_rom_crc64: u64,
    trainer_crc64: u64,
    mmc3_irq_variant: u32,
    emulator_version: String,
    rom_format: u32,
    mirror_mode_kind: u32,
    #[serde(with = "serde_bytes")]
    mirror_mode_custom_lut: Vec<u8>,
    has_battery: bool,
    trainer_len: u64,
    prg_rom_len: u64,
    chr_rom_len: u64,
    prg_ram_len: u64,
    save_prg_ram_len: u64,
    chr_ram_len: u64,
    save_chr_ram_len: u64,
}

pub fn resolve_sidecars(rom_path: &Path) -> SidecarPaths {
    let rom_name = rom_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("game");
    let base_dir = rom_path.parent().unwrap_or_else(|| Path::new("."));
    SidecarPaths {
        mapper_save_path: base_dir.join(format!("{rom_name}.sav")),
        states_dir: base_dir.join(format!("{rom_name}.states")),
    }
}

pub fn load_mapper_save(path: &Path) -> Result<Option<Vec<u8>>, PersistenceError> {
    match File::open(path) {
        Ok(mut file) => Ok(Some(read_limited(
            &mut file,
            MAX_MAPPER_SAVE_BYTES,
            "mapper save",
        )?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn write_mapper_save(path: &Path, bytes: &[u8]) -> Result<(), PersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(path, bytes)
}

pub fn write_recovery_mapper_save(
    original_path: &Path,
    bytes: &[u8],
) -> Result<PathBuf, PersistenceError> {
    let parent = original_path.parent().unwrap_or_else(|| Path::new("."));
    let base_name = original_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sidecar");
    for _ in 0..1024 {
        let nonce = temp_nonce();
        let recovery_path = parent.join(format!("{base_name}.recovered.{nonce}"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&recovery_path)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                file.sync_all()?;
                drop(file);
                sync_parent_dir(&recovery_path)?;
                return Ok(recovery_path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(PersistenceError::Validation(
        "failed to create unique mapper save recovery path".into(),
    ))
}

pub fn scan_state_slots(states_dir: &Path) -> Result<Vec<StateSlotSummary>, PersistenceError> {
    scan_state_slots_matching(states_dir, None)
}

pub fn scan_state_slots_for_target(
    states_dir: &Path,
    rom_identity: RomIdentity,
    options: CoreOptions,
) -> Result<Vec<StateSlotSummary>, PersistenceError> {
    scan_state_slots_matching(states_dir, Some((rom_identity, options)))
}

fn scan_state_slots_matching(
    states_dir: &Path,
    target: Option<(RomIdentity, CoreOptions)>,
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
    rom_identity: RomIdentity,
    options: CoreOptions,
    preview: Option<&ThumbnailSource>,
) -> Result<StateSlotSummary, PersistenceError> {
    fs::create_dir_all(states_dir)?;
    let saved_at = SystemTime::now();
    let has_thumbnail = preview.is_some();
    let metadata = StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id,
        saved_at_unix_ms: unix_millis(saved_at)?,
        has_thumbnail,
        mapper_type: u32::from(rom_identity.mapper_type),
        sub_mapper_type: u32::from(rom_identity.sub_mapper_type),
        prg_rom_crc64: rom_identity.prg_rom_crc64,
        chr_rom_crc64: rom_identity.chr_rom_crc64,
        trainer_crc64: rom_identity.trainer_crc64,
        mmc3_irq_variant: mmc3_irq_variant_to_u32(options.mmc3_irq_variant),
        emulator_version: env!("CARGO_PKG_VERSION").to_string(),
        rom_format: rom_format_to_u32(rom_identity.format),
        mirror_mode_kind: mirror_mode_kind_to_u32(rom_identity.mirror_mode),
        mirror_mode_custom_lut: mirror_mode_custom_lut(rom_identity.mirror_mode),
        has_battery: rom_identity.has_battery,
        trainer_len: rom_identity.trainer_len as u64,
        prg_rom_len: rom_identity.prg_rom_len as u64,
        chr_rom_len: rom_identity.chr_rom_len as u64,
        prg_ram_len: rom_identity.prg_ram_len as u64,
        save_prg_ram_len: rom_identity.save_prg_ram_len as u64,
        chr_ram_len: rom_identity.chr_ram_len as u64,
        save_chr_ram_len: rom_identity.save_chr_ram_len as u64,
    };
    let thumbnail_png = preview.map(encode_thumbnail_png).transpose()?;
    let archive_bytes = build_state_archive(&metadata, machine_state, thumbnail_png.as_deref())?;
    let path = state_slot_path(states_dir, slot_id);
    write_atomic(&path, &archive_bytes)?;
    Ok(summary_from_metadata(path, saved_at, &metadata))
}

pub fn load_state_slot(path: &Path) -> Result<LoadedStateSlot, PersistenceError> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let metadata = read_metadata(&mut archive)?;
    let machine_state = {
        let mut machine_state_file = archive.by_name(STATE_ENTRY)?;
        read_limited(
            &mut machine_state_file,
            MAX_MACHINE_STATE_BYTES,
            "machine state",
        )?
    };
    let thumbnail_png = match archive.by_name(THUMBNAIL_ENTRY) {
        Ok(mut file) => Some(read_limited(&mut file, MAX_THUMBNAIL_BYTES, "thumbnail")?),
        Err(zip::result::ZipError::FileNotFound) => None,
        Err(error) => return Err(error.into()),
    };
    let summary = StateSlotSummary {
        has_thumbnail: thumbnail_png.is_some(),
        ..summary_from_metadata(
            path.to_path_buf(),
            system_time_from_millis(metadata.saved_at_unix_ms),
            &metadata,
        )
    };
    Ok(LoadedStateSlot {
        summary,
        machine_state,
        thumbnail_png,
    })
}

pub fn delete_state_slot(path: &Path) -> Result<(), PersistenceError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn read_state_summary(
    path: &Path,
    target: Option<(RomIdentity, CoreOptions)>,
) -> Result<Option<StateSlotSummary>, PersistenceError> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let metadata = read_metadata(&mut archive)?;
    if let Some((rom_identity, options)) = target
        && !metadata_matches_target(&metadata, rom_identity, options)
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
        &StateArchiveMetadata {
            has_thumbnail,
            ..metadata
        },
    )))
}

fn read_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<StateArchiveMetadata, PersistenceError> {
    let mut metadata_file = archive.by_name(METADATA_ENTRY)?;
    let metadata_bytes = read_limited(&mut metadata_file, MAX_METADATA_BYTES, "metadata")?;
    let metadata: StateArchiveMetadata = rmp_serde::from_slice(metadata_bytes.as_slice())?;
    if metadata.schema_version != STATE_ARCHIVE_SCHEMA_VERSION {
        return Err(PersistenceError::Validation(format!(
            "unsupported state archive schema version: {}",
            metadata.schema_version
        )));
    }
    Ok(metadata)
}

fn build_state_archive(
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

fn summary_from_metadata(
    path: PathBuf,
    saved_at: SystemTime,
    metadata: &StateArchiveMetadata,
) -> StateSlotSummary {
    StateSlotSummary {
        schema_version: metadata.schema_version,
        slot_id: metadata.slot_id,
        path,
        saved_at,
        has_thumbnail: metadata.has_thumbnail,
        emulator_version: metadata.emulator_version.clone(),
    }
}

fn metadata_matches_target(
    metadata: &StateArchiveMetadata,
    rom_identity: RomIdentity,
    options: CoreOptions,
) -> bool {
    let mirror_mode_lut = mirror_mode_custom_lut(rom_identity.mirror_mode);
    metadata.mapper_type == u32::from(rom_identity.mapper_type)
        && metadata.sub_mapper_type == u32::from(rom_identity.sub_mapper_type)
        && metadata.prg_rom_crc64 == rom_identity.prg_rom_crc64
        && metadata.chr_rom_crc64 == rom_identity.chr_rom_crc64
        && metadata.trainer_crc64 == rom_identity.trainer_crc64
        && metadata.rom_format == rom_format_to_u32(rom_identity.format)
        && metadata.mirror_mode_kind == mirror_mode_kind_to_u32(rom_identity.mirror_mode)
        && metadata.mirror_mode_custom_lut == mirror_mode_lut
        && metadata.has_battery == rom_identity.has_battery
        && metadata.trainer_len == rom_identity.trainer_len as u64
        && metadata.prg_rom_len == rom_identity.prg_rom_len as u64
        && metadata.chr_rom_len == rom_identity.chr_rom_len as u64
        && metadata.prg_ram_len == rom_identity.prg_ram_len as u64
        && metadata.save_prg_ram_len == rom_identity.save_prg_ram_len as u64
        && metadata.chr_ram_len == rom_identity.chr_ram_len as u64
        && metadata.save_chr_ram_len == rom_identity.save_chr_ram_len as u64
        && metadata.mmc3_irq_variant == mmc3_irq_variant_to_u32(options.mmc3_irq_variant)
}

fn mmc3_irq_variant_to_u32(variant: Option<Mmc3IrqVariant>) -> u32 {
    match variant {
        Some(Mmc3IrqVariant::Sharp) => 1,
        Some(Mmc3IrqVariant::Nec) => 2,
        None => 0,
    }
}

fn rom_format_to_u32(format: RomFormat) -> u32 {
    match format {
        RomFormat::INes => 0,
        RomFormat::Nes20 => 1,
    }
}

fn mirror_mode_kind_to_u32(mode: MirrorMode) -> u32 {
    match mode {
        MirrorMode::Horizontal => 0,
        MirrorMode::Vertical => 1,
        MirrorMode::Single0 => 2,
        MirrorMode::Single1 => 3,
        MirrorMode::Four => 4,
        MirrorMode::Custom(_) => 5,
    }
}

fn mirror_mode_custom_lut(mode: MirrorMode) -> Vec<u8> {
    match mode {
        MirrorMode::Custom(lut) => lut.to_vec(),
        _ => Vec::new(),
    }
}

fn encode_thumbnail_png(source: &ThumbnailSource) -> Result<Vec<u8>, PersistenceError> {
    if source.width == 0 || source.height == 0 {
        return Err(PersistenceError::Validation(
            "thumbnail source dimensions must be non-zero".into(),
        ));
    }
    if source.rgba.len() != (source.width as usize) * (source.height as usize) * 4 {
        return Err(PersistenceError::Validation(
            "thumbnail RGBA buffer length mismatch".into(),
        ));
    }
    let target_width = THUMBNAIL_TARGET_WIDTH;
    let target_height =
        ((u64::from(source.height) * u64::from(target_width)) / u64::from(source.width)) as u32;
    let resized = resize_rgba_nearest(source, target_width, target_height.max(1));
    let mut png_bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut png_bytes, target_width, target_height.max(1));
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&resized)?;
    }
    Ok(png_bytes)
}

fn resize_rgba_nearest(source: &ThumbnailSource, width: u32, height: u32) -> Vec<u8> {
    let mut resized = vec![0; (width as usize) * (height as usize) * 4];
    for y in 0..height {
        let src_y = (u64::from(y) * u64::from(source.height) / u64::from(height)) as usize;
        for x in 0..width {
            let src_x = (u64::from(x) * u64::from(source.width) / u64::from(width)) as usize;
            let src_offset = (src_y * source.width as usize + src_x) * 4;
            let dst_offset = (y as usize * width as usize + x as usize) * 4;
            resized[dst_offset..dst_offset + 4]
                .copy_from_slice(&source.rgba[src_offset..src_offset + 4]);
        }
    }
    resized
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PersistenceError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sidecar");
    let (mut file, temp_path) = create_temp_file(parent, temp_name)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    replace_path(&temp_path, path)?;
    sync_parent_dir(path)?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_path(from: &Path, to: &Path) -> Result<(), PersistenceError> {
    fs::rename(from, to)?;
    Ok(())
}

#[cfg(windows)]
fn replace_path(from: &Path, to: &Path) -> Result<(), PersistenceError> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(iter::once(0)).collect();
    let from_wide: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    // ReplaceFileW preserves the destination until replacement succeeds.
    let replaced = unsafe {
        ReplaceFileW(
            to_wide.as_ptr(),
            from_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced != 0 {
        Ok(())
    } else {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            fs::rename(from, to)?;
            Ok(())
        } else {
            Err(error.into())
        }
    }
}

#[cfg(not(windows))]
fn sync_parent_dir(path: &Path) -> Result<(), PersistenceError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn sync_parent_dir(_path: &Path) -> Result<(), PersistenceError> {
    Ok(())
}

fn read_limited(
    reader: &mut impl Read,
    max_len: usize,
    label: &str,
) -> Result<Vec<u8>, PersistenceError> {
    let mut bytes = Vec::new();
    reader.take(max_len as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_len {
        return Err(PersistenceError::Validation(format!(
            "{label} entry exceeds {max_len} bytes"
        )));
    }
    Ok(bytes)
}

fn create_temp_file(
    parent: &Path,
    base_name: &str,
) -> Result<(std::fs::File, PathBuf), PersistenceError> {
    for _ in 0..1024 {
        let nonce = temp_nonce();
        let temp_path = parent.join(format!(".{base_name}.{}.tmp", nonce));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((file, temp_path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(PersistenceError::Validation(
        "failed to create unique temporary sidecar".into(),
    ))
}

fn temp_nonce() -> String {
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    format!(
        "{}.{}.{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed),
    )
}

fn unix_millis(time: SystemTime) -> Result<u64, PersistenceError> {
    Ok(time
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PersistenceError::Validation(error.to_string()))?
        .as_millis() as u64)
}

fn system_time_from_millis(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::io::Write;

    const STATE_ARCHIVE_FIXTURE_HEX: &str = "504b0304140000000800000021004ce2ae73e30000006a010000100000006d657461646174612e6d73677061636b55904d4ec3400c851d7eaec09a3b80b88ee54ca664443c133c9ea8ddb1e720958802597181b245bd013beec124226abafc9e3fd94ffe819bf7686acb849d95e8822ff6b1098aaeba1e2375b642524cde6d91e311a0783dfed630d41451ebc4a527d71c7aa6b6b582ba6bedd5474c25ae02185a7944098c46ccc37d31985a4e7831a890f3d99df172643677e8e4193b12475e61b49c1ad2204bc1fdc66d35897d9b966c823065899d48563854169f9cafe0739d981435cb4dd22fe8a7ee25a95ad91dfae57a633df44bd10cdf2fd02f4527bcfd9fd28c30bf0657c924ccfe99b04efe00504b030414000000080000002100def67eb017000000150000000900000073746174652e62696e4bcbac28292d4ad5cd4d4ccec8cc4bd52d2e492c490500504b030414000000080000002100f860c12e0b000000090000000d0000007468756d626e61696c2e706e67eb0cf073e7e592e2fa0f00504b01021403140000000800000021004ce2ae73e30000006a010000100000000000000000000000a481000000006d657461646174612e6d73677061636b504b0102140314000000080000002100def67eb01700000015000000090000000000000000000000a4811101000073746174652e62696e504b0102140314000000080000002100f860c12e0b000000090000000d0000000000000000000000a4814f0100007468756d626e61696c2e706e67504b05060000000003000300b0000000850100000000";

    fn fixture_bytes(hex: &str) -> Vec<u8> {
        assert!(
            !hex.trim().is_empty(),
            "fixture hex must be populated before running persistence tests"
        );
        let hex = hex.trim();
        assert_eq!(hex.len() % 2, 0, "fixture hex length must be even");
        hex.as_bytes()
            .chunks_exact(2)
            .map(|chunk| {
                let text = std::str::from_utf8(chunk).expect("fixture hex should be valid utf-8");
                u8::from_str_radix(text, 16).expect("fixture hex should decode")
            })
            .collect()
    }

    fn fixture_slot_path(name: &str) -> PathBuf {
        let dir = test_dir(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        state_slot_path(&dir, 99)
    }

    fn write_fixture_archive(name: &str) -> PathBuf {
        let path = fixture_slot_path(name);
        fs::write(&path, fixture_bytes(STATE_ARCHIVE_FIXTURE_HEX)).unwrap();
        path
    }

    fn test_rom_identity() -> RomIdentity {
        RomIdentity {
            format: nerust_core::RomFormat::INes,
            mapper_type: 4,
            sub_mapper_type: 0,
            mirror_mode: nerust_core::MirrorMode::Horizontal,
            has_battery: true,
            trainer_len: 0,
            prg_rom_len: 0x8000,
            chr_rom_len: 0x2000,
            prg_ram_len: 0,
            save_prg_ram_len: 0x2000,
            chr_ram_len: 0,
            save_chr_ram_len: 0,
            prg_rom_crc64: 1,
            chr_rom_crc64: 2,
            trainer_crc64: 3,
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        env::current_dir()
            .unwrap()
            .join("target")
            .join("persistence-tests")
            .join(name)
    }

    #[test]
    fn resolve_sidecars_appends_to_full_rom_filename() {
        let nes = resolve_sidecars(Path::new("/tmp/game.nes"));
        let fds = resolve_sidecars(Path::new("/tmp/game.fds"));

        assert_eq!(nes.mapper_save_path, PathBuf::from("/tmp/game.nes.sav"));
        assert_eq!(nes.states_dir, PathBuf::from("/tmp/game.nes.states"));
        assert_eq!(fds.mapper_save_path, PathBuf::from("/tmp/game.fds.sav"));
        assert_eq!(fds.states_dir, PathBuf::from("/tmp/game.fds.states"));
        assert_ne!(nes.mapper_save_path, fds.mapper_save_path);
        assert_ne!(nes.states_dir, fds.states_dir);
    }

    #[test]
    fn slot_id_allocation_is_monotonic_across_deletions() {
        let dir = test_dir("slot-id-allocation");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(allocate_next_slot_id(&dir).unwrap(), 1);
        write_state_slot(
            &dir,
            1,
            b"a",
            test_rom_identity(),
            CoreOptions::default(),
            None,
        )
        .unwrap();
        write_state_slot(
            &dir,
            2,
            b"b",
            test_rom_identity(),
            CoreOptions::default(),
            None,
        )
        .unwrap();
        delete_state_slot(&state_slot_path(&dir, 1)).unwrap();

        assert_eq!(allocate_next_slot_id(&dir).unwrap(), 3);
    }

    #[test]
    fn slot_id_allocation_persists_without_writing_slot_files() {
        let dir = test_dir("slot-id-counter");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        assert_eq!(allocate_next_slot_id(&dir).unwrap(), 1);
        assert_eq!(allocate_next_slot_id(&dir).unwrap(), 2);
    }

    #[test]
    fn corrupt_slot_does_not_hide_valid_slots_or_block_allocation() {
        let dir = test_dir("corrupt-slot-scan");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_state_slot(
            &dir,
            1,
            b"ok",
            test_rom_identity(),
            CoreOptions::default(),
            None,
        )
        .unwrap();
        fs::write(state_slot_path(&dir, 2), b"not-a-zip-archive").unwrap();

        let slots = scan_state_slots(&dir).unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].slot_id, 1);
        assert_eq!(allocate_next_slot_id(&dir).unwrap(), 3);
    }

    #[test]
    fn metadata_only_archive_is_not_listed_as_state_slot() {
        let dir = test_dir("metadata-only-slot");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = state_slot_path(&dir, 3);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        let mut writer = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let metadata = StateArchiveMetadata {
            schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
            slot_id: 3,
            saved_at_unix_ms: unix_millis(SystemTime::now()).unwrap(),
            has_thumbnail: false,
            mapper_type: 4,
            sub_mapper_type: 0,
            prg_rom_crc64: 1,
            chr_rom_crc64: 2,
            trainer_crc64: 3,
            mmc3_irq_variant: 0,
            emulator_version: "test".into(),
            rom_format: 0,
            mirror_mode_kind: 0,
            mirror_mode_custom_lut: Vec::new(),
            has_battery: false,
            trainer_len: 0,
            prg_rom_len: 0,
            chr_rom_len: 0,
            prg_ram_len: 0,
            save_prg_ram_len: 0,
            chr_ram_len: 0,
            save_chr_ram_len: 0,
        };
        writer.start_file(METADATA_ENTRY, options).unwrap();
        writer
            .write_all(&rmp_serde::to_vec_named(&metadata).unwrap())
            .unwrap();
        writer.finish().unwrap();

        assert!(scan_state_slots(&dir).unwrap().is_empty());
        assert!(load_state_slot(&path).is_err());
    }

    #[test]
    fn state_archive_fixture_loads_metadata_state_and_thumbnail() {
        let path = write_fixture_archive("fixture-archive");

        let loaded = load_state_slot(&path).expect("fixture archive should load");
        assert_eq!(loaded.summary.schema_version, STATE_ARCHIVE_SCHEMA_VERSION);
        assert_eq!(loaded.summary.slot_id, 5);
        assert_eq!(loaded.machine_state, b"fixture-machine-state");
        assert!(loaded.summary.has_thumbnail);
        assert_eq!(
            loaded.thumbnail_png,
            Some(vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF])
        );

        let summaries = scan_state_slots(path.parent().unwrap()).expect("fixture slot should scan");
        assert_eq!(summaries, vec![loaded.summary.clone()]);
    }

    #[test]
    fn state_archive_round_trip_preserves_metadata_and_thumbnail() {
        let dir = test_dir("state-archive-round-trip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let summary = write_state_slot(
            &dir,
            7,
            b"machine-state",
            test_rom_identity(),
            CoreOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
            },
            Some(&ThumbnailSource {
                width: 2,
                height: 1,
                rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
            }),
        )
        .unwrap();
        let loaded = load_state_slot(&summary.path).unwrap();

        assert_eq!(loaded.summary.slot_id, 7);
        assert_eq!(loaded.machine_state, b"machine-state");
        assert!(loaded.thumbnail_png.is_some());
        assert_eq!(loaded.summary.schema_version, STATE_ARCHIVE_SCHEMA_VERSION);
    }

    #[test]
    fn scan_state_slots_for_target_filters_incompatible_slots() {
        let dir = test_dir("target-filtered-slots");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let matching_identity = test_rom_identity();
        let mismatched_identity = RomIdentity {
            prg_rom_crc64: 100,
            ..matching_identity
        };
        let header_corrected_identity = RomIdentity {
            format: RomFormat::Nes20,
            ..matching_identity
        };

        write_state_slot(
            &dir,
            1,
            b"matching",
            matching_identity,
            CoreOptions::default(),
            None,
        )
        .unwrap();
        write_state_slot(
            &dir,
            2,
            b"mismatched-rom",
            mismatched_identity,
            CoreOptions::default(),
            None,
        )
        .unwrap();
        write_state_slot(
            &dir,
            3,
            b"mismatched-options",
            matching_identity,
            CoreOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
            },
            None,
        )
        .unwrap();
        write_state_slot(
            &dir,
            4,
            b"header-corrected",
            header_corrected_identity,
            CoreOptions::default(),
            None,
        )
        .unwrap();

        let slots =
            scan_state_slots_for_target(&dir, matching_identity, CoreOptions::default()).unwrap();
        let slot_ids = slots.iter().map(|slot| slot.slot_id).collect::<Vec<_>>();

        assert_eq!(slot_ids, vec![1]);
    }

    #[test]
    fn state_archive_rejects_schema_mismatch() {
        let dir = test_dir("schema-mismatch");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = state_slot_path(&dir, 1);
        let metadata = StateArchiveMetadata {
            schema_version: STATE_ARCHIVE_SCHEMA_VERSION + 1,
            slot_id: 1,
            saved_at_unix_ms: unix_millis(SystemTime::now()).unwrap(),
            has_thumbnail: false,
            mapper_type: 4,
            sub_mapper_type: 0,
            prg_rom_crc64: 1,
            chr_rom_crc64: 2,
            trainer_crc64: 3,
            mmc3_irq_variant: 0,
            emulator_version: "test".into(),
            rom_format: 0,
            mirror_mode_kind: 0,
            mirror_mode_custom_lut: Vec::new(),
            has_battery: false,
            trainer_len: 0,
            prg_rom_len: 0,
            chr_rom_len: 0,
            prg_ram_len: 0,
            save_prg_ram_len: 0,
            chr_ram_len: 0,
            save_chr_ram_len: 0,
        };
        let archive = build_state_archive(&metadata, b"state", None).unwrap();
        fs::write(&path, archive).unwrap();

        let error = load_state_slot(&path).expect_err("schema mismatch should reject");
        assert!(
            error
                .to_string()
                .contains("unsupported state archive schema version")
        );
    }

    #[test]
    fn missing_thumbnail_is_reported_consistently_even_if_metadata_claims_presence() {
        let dir = test_dir("missing-thumbnail");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = state_slot_path(&dir, 4);
        let metadata = StateArchiveMetadata {
            schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
            slot_id: 4,
            saved_at_unix_ms: unix_millis(SystemTime::now()).unwrap(),
            has_thumbnail: true,
            mapper_type: 4,
            sub_mapper_type: 0,
            prg_rom_crc64: 1,
            chr_rom_crc64: 2,
            trainer_crc64: 3,
            mmc3_irq_variant: 0,
            emulator_version: "test".into(),
            rom_format: 0,
            mirror_mode_kind: 0,
            mirror_mode_custom_lut: Vec::new(),
            has_battery: false,
            trainer_len: 0,
            prg_rom_len: 0,
            chr_rom_len: 0,
            prg_ram_len: 0,
            save_prg_ram_len: 0,
            chr_ram_len: 0,
            save_chr_ram_len: 0,
        };
        fs::write(
            &path,
            build_state_archive(&metadata, b"state", None).unwrap(),
        )
        .unwrap();

        let summary = scan_state_slots(&dir).unwrap().pop().unwrap();
        let loaded = load_state_slot(&path).unwrap();
        assert!(!summary.has_thumbnail);
        assert!(!loaded.summary.has_thumbnail);
        assert_eq!(loaded.thumbnail_png, None);
    }

    #[test]
    fn invalid_thumbnail_bytes_are_preserved_as_opaque_blob() {
        let dir = test_dir("invalid-thumbnail");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = state_slot_path(&dir, 8);
        let metadata = StateArchiveMetadata {
            schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
            slot_id: 8,
            saved_at_unix_ms: unix_millis(SystemTime::now()).unwrap(),
            has_thumbnail: true,
            mapper_type: 4,
            sub_mapper_type: 0,
            prg_rom_crc64: 1,
            chr_rom_crc64: 2,
            trainer_crc64: 3,
            mmc3_irq_variant: 0,
            emulator_version: "test".into(),
            rom_format: 0,
            mirror_mode_kind: 0,
            mirror_mode_custom_lut: Vec::new(),
            has_battery: false,
            trainer_len: 0,
            prg_rom_len: 0,
            chr_rom_len: 0,
            prg_ram_len: 0,
            save_prg_ram_len: 0,
            chr_ram_len: 0,
            save_chr_ram_len: 0,
        };
        let cursor = Cursor::new(Vec::<u8>::new());
        let mut writer = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer.start_file(METADATA_ENTRY, options).unwrap();
        writer
            .write_all(&rmp_serde::to_vec_named(&metadata).unwrap())
            .unwrap();
        writer.start_file(STATE_ENTRY, options).unwrap();
        writer.write_all(b"state").unwrap();
        writer.start_file(THUMBNAIL_ENTRY, options).unwrap();
        writer
            .write_all(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF])
            .unwrap();
        fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();

        let summary = scan_state_slots(&dir).unwrap().pop().unwrap();
        let loaded = load_state_slot(&path).unwrap();
        assert!(summary.has_thumbnail);
        assert!(loaded.summary.has_thumbnail);
        assert_eq!(
            loaded.thumbnail_png,
            Some(vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF])
        );
    }

    #[test]
    fn strict_target_filtering_requires_all_identity_and_option_fields() {
        let dir = test_dir("strict-target-filtering");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let matching_identity = test_rom_identity();
        write_state_slot(
            &dir,
            1,
            b"matching",
            matching_identity,
            CoreOptions::default(),
            None,
        )
        .unwrap();
        write_state_slot(
            &dir,
            2,
            b"battery-mismatch",
            RomIdentity {
                has_battery: false,
                ..matching_identity
            },
            CoreOptions::default(),
            None,
        )
        .unwrap();
        write_state_slot(
            &dir,
            3,
            b"save-ram-mismatch",
            RomIdentity {
                save_prg_ram_len: matching_identity.save_prg_ram_len + 1,
                ..matching_identity
            },
            CoreOptions::default(),
            None,
        )
        .unwrap();
        write_state_slot(
            &dir,
            4,
            b"option-mismatch",
            matching_identity,
            CoreOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Nec),
            },
            None,
        )
        .unwrap();

        let slots =
            scan_state_slots_for_target(&dir, matching_identity, CoreOptions::default()).unwrap();
        assert_eq!(
            slots.iter().map(|slot| slot.slot_id).collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn save_slot_summary_matches_loaded_summary() {
        let dir = test_dir("summary-consistency");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let written = write_state_slot(
            &dir,
            11,
            b"state",
            test_rom_identity(),
            CoreOptions::default(),
            Some(&ThumbnailSource {
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 4],
            }),
        )
        .unwrap();

        let mut scanned_slots = scan_state_slots(&dir).unwrap();
        let scanned = scanned_slots.pop().unwrap();
        let loaded = load_state_slot(&written.path).unwrap();
        assert_eq!(scanned.schema_version, written.schema_version);
        assert_eq!(scanned.slot_id, written.slot_id);
        assert_eq!(scanned.path, written.path);
        assert_eq!(scanned.has_thumbnail, written.has_thumbnail);
        assert_eq!(scanned.emulator_version, written.emulator_version);
        assert_eq!(loaded.summary, scanned);
    }

    #[test]
    fn mapper_save_sidecar_and_recovery_paths_preserve_bytes() {
        let dir = test_dir("mapper-save-sidecars");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let sidecar = dir.join("game.nes.sav");

        write_mapper_save(&sidecar, b"primary").unwrap();
        assert_eq!(
            load_mapper_save(&sidecar).unwrap(),
            Some(b"primary".to_vec())
        );

        let recovery = write_recovery_mapper_save(&sidecar, b"recovered").unwrap();
        assert_ne!(recovery, sidecar);
        assert_eq!(
            load_mapper_save(&sidecar).unwrap(),
            Some(b"primary".to_vec())
        );
        assert_eq!(fs::read(recovery).unwrap(), b"recovered");
    }
}
