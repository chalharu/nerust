use crate::error::PersistenceError;
use crate::time::unix_millis;
use nerust_contract_core::identity::SystemIdentity;
use nerust_input_schema::SystemId;
use std::io::{Read, Seek};
use std::time::SystemTime;
use zip::ZipArchive;

pub(crate) const METADATA_ENTRY: &str = "metadata.msgpack";
pub(crate) const STATE_ENTRY: &str = "state.bin";
pub(crate) const THUMBNAIL_ENTRY: &str = "thumbnail.png";
pub(crate) const STATE_ARCHIVE_SCHEMA_VERSION: u32 = 2;

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct StateArchiveMetadata {
    pub(crate) schema_version: u32,
    pub(crate) slot_id: u64,
    pub(crate) saved_at_unix_ms: u64,
    pub(crate) has_thumbnail: bool,
    pub(crate) system_id: SystemId,
    #[serde(with = "serde_bytes")]
    pub(crate) identity_bytes: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub(crate) options_bytes: Vec<u8>,
    pub(crate) emulator_version: String,
}

// ---------------------------------------------------------------------------
// v1 backward compat — deserialize old format and convert to v2
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct StateArchiveMetadataV1 {
    schema_version: u32,
    slot_id: u64,
    saved_at_unix_ms: u64,
    has_thumbnail: bool,
    #[serde(default = "default_system_id")]
    system_id: SystemId,
    mapper_type: u32,
    sub_mapper_type: u32,
    prg_rom_crc64: u64,
    chr_rom_crc64: u64,
    trainer_crc64: u64,
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

#[derive(serde::Serialize)]
enum RomFormatConv {
    INes,
    Nes20,
}

#[derive(serde::Serialize)]
enum MirrorModeConv {
    Horizontal,
    Vertical,
    Single0,
    Single1,
    Four,
    Custom([u8; 4]),
}

/// Struct matching the serde field names of `nes_core::rom_identity::RomIdentity`
/// so that v1→v2 conversion produces the same `identity_bytes` as a freshly loaded ROM.
#[derive(serde::Serialize)]
struct V1RomIdentity {
    format: RomFormatConv,
    mapper_type: u16,
    sub_mapper_type: u8,
    mirror_mode: MirrorModeConv,
    has_battery: bool,
    trainer_len: usize,
    prg_rom_len: usize,
    chr_rom_len: usize,
    prg_ram_len: usize,
    save_prg_ram_len: usize,
    chr_ram_len: usize,
    save_chr_ram_len: usize,
    prg_rom_crc64: u64,
    chr_rom_crc64: u64,
    trainer_crc64: u64,
}

fn convert_v1_to_v2(v1: StateArchiveMetadataV1) -> StateArchiveMetadata {
    let mirror_mode = match (v1.mirror_mode_kind, v1.mirror_mode_custom_lut.as_slice()) {
        (0, _) => MirrorModeConv::Horizontal,
        (1, _) => MirrorModeConv::Vertical,
        (2, _) => MirrorModeConv::Single0,
        (3, _) => MirrorModeConv::Single1,
        (4, _) => MirrorModeConv::Four,
        (5, lut) if lut.len() == 4 => {
            let mut arr = [0u8; 4];
            arr.copy_from_slice(lut);
            MirrorModeConv::Custom(arr)
        }
        _ => MirrorModeConv::Horizontal,
    };
    let format = match v1.rom_format {
        0 => RomFormatConv::INes,
        1 => RomFormatConv::Nes20,
        _ => RomFormatConv::INes,
    };
    let identity = V1RomIdentity {
        format,
        mapper_type: v1.mapper_type as u16,
        sub_mapper_type: v1.sub_mapper_type as u8,
        mirror_mode,
        has_battery: v1.has_battery,
        trainer_len: v1.trainer_len as usize,
        prg_rom_len: v1.prg_rom_len as usize,
        chr_rom_len: v1.chr_rom_len as usize,
        prg_ram_len: v1.prg_ram_len as usize,
        save_prg_ram_len: v1.save_prg_ram_len as usize,
        chr_ram_len: v1.chr_ram_len as usize,
        save_chr_ram_len: v1.save_chr_ram_len as usize,
        prg_rom_crc64: v1.prg_rom_crc64,
        chr_rom_crc64: v1.chr_rom_crc64,
        trainer_crc64: v1.trainer_crc64,
    };
    StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id: v1.slot_id,
        saved_at_unix_ms: v1.saved_at_unix_ms,
        has_thumbnail: v1.has_thumbnail,
        system_id: v1.system_id,
        identity_bytes: rmp_serde::to_vec_named(&identity).unwrap_or_default(),
        options_bytes: Vec::new(),
        emulator_version: v1.emulator_version,
    }
}

const fn default_system_id() -> SystemId {
    SystemId::Nes
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

    // Try v2.
    if let Ok(meta) = rmp_serde::from_slice::<StateArchiveMetadata>(metadata_bytes.as_slice()) {
        if meta.schema_version == STATE_ARCHIVE_SCHEMA_VERSION {
            return Ok(meta);
        }
        return Err(PersistenceError::Validation(format!(
            "unsupported state archive schema version: {}",
            meta.schema_version
        )));
    }

    // Fall back to v1 conversion.
    if let Ok(v1) = rmp_serde::from_slice::<StateArchiveMetadataV1>(metadata_bytes.as_slice()) {
        if v1.schema_version == 1 {
            return Ok(convert_v1_to_v2(v1));
        }
        return Err(PersistenceError::Validation(format!(
            "unsupported state archive schema version: {}",
            v1.schema_version
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
        system_id: identity.system_id,
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
