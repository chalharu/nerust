use nerust_core_traits::{declare_system_id, identity::SystemId};

use super::{STATE_ARCHIVE_SCHEMA_VERSION, StateArchiveMetadata};
use crate::error::PersistenceError;

declare_system_id!(LegacyNesSystemId, "nes-legacy");

pub(super) fn matches_system_id(stored: &dyn SystemId, current: &dyn SystemId) -> bool {
    stored == &LegacyNesSystemId as &dyn SystemId && current.to_string() == "nes"
}

fn validate_nes_system_id(system_id: &str) -> Result<(), PersistenceError> {
    if matches!(system_id, "Nes" | "nes") {
        Ok(())
    } else {
        Err(PersistenceError::Validation(format!(
            "unsupported legacy state archive system: {system_id}"
        )))
    }
}

#[derive(serde::Deserialize)]
struct StateArchiveMetadataV2 {
    schema_version: u32,
    slot_id: u64,
    saved_at_unix_ms: u64,
    has_thumbnail: bool,
    system_id: String,
    #[serde(with = "serde_bytes")]
    identity_bytes: Vec<u8>,
    #[serde(with = "serde_bytes")]
    options_bytes: Vec<u8>,
    emulator_version: String,
}

pub(super) fn decode_v2(bytes: &[u8]) -> Result<StateArchiveMetadata, PersistenceError> {
    let v2: StateArchiveMetadataV2 = rmp_serde::from_slice(bytes)?;
    debug_assert_eq!(v2.schema_version, 2);
    validate_nes_system_id(&v2.system_id)?;
    Ok(StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id: v2.slot_id,
        saved_at_unix_ms: v2.saved_at_unix_ms,
        has_thumbnail: v2.has_thumbnail,
        system_id: Box::new(LegacyNesSystemId),
        identity_bytes: v2.identity_bytes,
        options_bytes: v2.options_bytes,
        emulator_version: v2.emulator_version,
    })
}

#[derive(serde::Deserialize)]
#[serde(default)]
struct StateArchiveMetadataV1 {
    schema_version: u32,
    slot_id: u64,
    saved_at_unix_ms: u64,
    has_thumbnail: bool,
    system_id: String,
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

impl Default for StateArchiveMetadataV1 {
    fn default() -> Self {
        Self {
            schema_version: 1,
            slot_id: 0,
            saved_at_unix_ms: 0,
            has_thumbnail: false,
            system_id: "nes".into(),
            mapper_type: 0,
            sub_mapper_type: 0,
            prg_rom_crc64: 0,
            chr_rom_crc64: 0,
            trainer_crc64: 0,
            emulator_version: String::new(),
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
        }
    }
}

#[derive(serde::Serialize)]
enum RomFormatV1 {
    INes,
    Nes20,
}

#[derive(serde::Serialize)]
enum MirrorModeV1 {
    Horizontal,
    Vertical,
    Single0,
    Single1,
    Four,
    Custom([u8; 4]),
}

#[derive(serde::Serialize)]
struct RomIdentityV1 {
    format: RomFormatV1,
    mapper_type: u16,
    sub_mapper_type: u8,
    mirror_mode: MirrorModeV1,
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

pub(super) fn decode_v1(bytes: &[u8]) -> Result<StateArchiveMetadata, PersistenceError> {
    let v1: StateArchiveMetadataV1 = rmp_serde::from_slice(bytes)?;
    debug_assert_eq!(v1.schema_version, 1);
    validate_nes_system_id(&v1.system_id)?;
    let mirror_mode = match (v1.mirror_mode_kind, v1.mirror_mode_custom_lut.as_slice()) {
        (0, _) => MirrorModeV1::Horizontal,
        (1, _) => MirrorModeV1::Vertical,
        (2, _) => MirrorModeV1::Single0,
        (3, _) => MirrorModeV1::Single1,
        (4, _) => MirrorModeV1::Four,
        (5, lut) if lut.len() == 4 => MirrorModeV1::Custom(lut.try_into().unwrap()),
        _ => MirrorModeV1::Horizontal,
    };
    let identity = RomIdentityV1 {
        format: if v1.rom_format == 1 {
            RomFormatV1::Nes20
        } else {
            RomFormatV1::INes
        },
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
    let identity_bytes = rmp_serde::to_vec_named(&identity).map_err(|error| {
        PersistenceError::Validation(format!("v1 identity encoding failed: {error}"))
    })?;
    Ok(StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id: v1.slot_id,
        saved_at_unix_ms: v1.saved_at_unix_ms,
        has_thumbnail: v1.has_thumbnail,
        system_id: Box::new(LegacyNesSystemId),
        identity_bytes,
        options_bytes: Vec::new(),
        emulator_version: v1.emulator_version,
    })
}
