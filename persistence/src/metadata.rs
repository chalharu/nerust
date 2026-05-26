use crate::error::PersistenceError;
use crate::time::unix_millis;
use nerust_contract_mirror::MirrorMode;
use nerust_contract_persistence::{CanonicalMediaIdentity, PersistenceIdentity};
use nerust_contract_rom::RomFormat;
use nerust_input_schema::SystemId;
use std::io::{Read, Seek};
use std::time::SystemTime;
use zip::ZipArchive;

pub(crate) const METADATA_ENTRY: &str = "metadata.msgpack";
pub(crate) const STATE_ENTRY: &str = "state.bin";
pub(crate) const THUMBNAIL_ENTRY: &str = "thumbnail.png";
pub(crate) const STATE_ARCHIVE_SCHEMA_VERSION: u32 = 1;

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct StateArchiveMetadata {
    pub(crate) schema_version: u32,
    pub(crate) slot_id: u64,
    pub(crate) saved_at_unix_ms: u64,
    pub(crate) has_thumbnail: bool,
    #[serde(default = "default_system_id")]
    pub(crate) system_id: SystemId,
    pub(crate) mapper_type: u32,
    pub(crate) sub_mapper_type: u32,
    pub(crate) prg_rom_crc64: u64,
    pub(crate) chr_rom_crc64: u64,
    pub(crate) trainer_crc64: u64,
    pub(crate) emulator_version: String,
    pub(crate) rom_format: u32,
    pub(crate) mirror_mode_kind: u32,
    #[serde(with = "serde_bytes")]
    pub(crate) mirror_mode_custom_lut: Vec<u8>,
    pub(crate) has_battery: bool,
    pub(crate) trainer_len: u64,
    pub(crate) prg_rom_len: u64,
    pub(crate) chr_rom_len: u64,
    pub(crate) prg_ram_len: u64,
    pub(crate) save_prg_ram_len: u64,
    pub(crate) chr_ram_len: u64,
    pub(crate) save_chr_ram_len: u64,
}

const fn default_system_id() -> SystemId {
    SystemId::Nes
}

pub(crate) fn read_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<StateArchiveMetadata, PersistenceError> {
    const MAX_METADATA_BYTES: usize = 64 * 1024;

    let mut metadata_file = archive.by_name(METADATA_ENTRY)?;
    let metadata_bytes =
        crate::fs_ops::read_limited(&mut metadata_file, MAX_METADATA_BYTES, "metadata")?;
    let metadata: StateArchiveMetadata = rmp_serde::from_slice(metadata_bytes.as_slice())?;
    if metadata.schema_version != STATE_ARCHIVE_SCHEMA_VERSION {
        return Err(PersistenceError::Validation(format!(
            "unsupported state archive schema version: {}",
            metadata.schema_version
        )));
    }
    Ok(metadata)
}

pub(crate) fn encode_slot_metadata(
    slot_id: u64,
    saved_at: SystemTime,
    identity: PersistenceIdentity,
    has_thumbnail: bool,
) -> Result<StateArchiveMetadata, PersistenceError> {
    let CanonicalMediaIdentity::Rom(rom_identity) = identity.media;
    Ok(StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id,
        saved_at_unix_ms: unix_millis(saved_at)?,
        has_thumbnail,
        system_id: identity.system_id,
        mapper_type: u32::from(rom_identity.mapper_type),
        sub_mapper_type: u32::from(rom_identity.sub_mapper_type),
        prg_rom_crc64: rom_identity.prg_rom_crc64,
        chr_rom_crc64: rom_identity.chr_rom_crc64,
        trainer_crc64: rom_identity.trainer_crc64,
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
    })
}

pub(crate) fn slot_matches_identity(
    metadata: &StateArchiveMetadata,
    identity: PersistenceIdentity,
) -> bool {
    let CanonicalMediaIdentity::Rom(rom_identity) = identity.media;
    let mirror_mode_lut = mirror_mode_custom_lut(rom_identity.mirror_mode);
    metadata.system_id == identity.system_id
        && metadata.mapper_type == u32::from(rom_identity.mapper_type)
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
