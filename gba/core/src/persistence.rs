//! Phase 10 persistence orchestration: envelope codec plus the system
//! export/import編成. Device DTOs live in their own modules; this file only
//! sequences version/identity/options checks and the candidate swap.

use crate::cartridge::Cartridge;
use crate::cartridge::save::SaveTypeSer;
use crate::core_options::GbaCoreOptions;
use crate::persistence_error::GbaPersistenceError;
use crate::rom_identity::GbaRomIdentity;
use crate::system::{GbaSystem, GbaSystemState};

const MIN_SUPPORTED_SCHEMA_VERSION: u32 = 1;
const PERSISTENCE_SCHEMA_VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct MachineStatePayload {
    schema_version: u32,
    rom_identity: GbaRomIdentity,
    options: GbaCoreOptions,
    audio_rate: u32,
    system: GbaSystemState,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MapperSavePayload {
    schema_version: u32,
    rom_identity: GbaRomIdentity,
    save_kind: SaveTypeSer,
    cartridge: serde_bytes::ByteBuf,
}

fn validate_version(version: u32) -> Result<(), GbaPersistenceError> {
    if (MIN_SUPPORTED_SCHEMA_VERSION..=PERSISTENCE_SCHEMA_VERSION).contains(&version) {
        Ok(())
    } else {
        Err(GbaPersistenceError::UnsupportedVersion(version))
    }
}

pub(crate) fn export_machine_state(
    system: &GbaSystem,
    rom_identity: GbaRomIdentity,
    options: GbaCoreOptions,
    audio_rate: u32,
) -> Result<Vec<u8>, GbaPersistenceError> {
    let system = system
        .export_state()
        .map_err(GbaPersistenceError::InvalidState)?;
    Ok(rmp_serde::to_vec_named(&MachineStatePayload {
        schema_version: PERSISTENCE_SCHEMA_VERSION,
        rom_identity,
        options,
        audio_rate,
        system,
    })?)
}

/// Decode and validate a machine state payload, then materialize it into a
/// fresh candidate system built from the retained ROM. The caller swaps the
/// candidate in only on success, so a failed import never mutates live state.
pub(crate) fn import_machine_state(
    data: &[u8],
    rom: &[u8],
    expected_identity: &GbaRomIdentity,
    expected_options: GbaCoreOptions,
    audio_rate: u32,
) -> Result<GbaSystem, GbaPersistenceError> {
    let payload: MachineStatePayload = rmp_serde::from_slice(data)?;
    validate_version(payload.schema_version)?;
    if &payload.rom_identity != expected_identity {
        return Err(GbaPersistenceError::RomIdentityMismatch);
    }
    if payload.options != expected_options {
        return Err(GbaPersistenceError::OptionsMismatch);
    }
    if payload.audio_rate != audio_rate {
        return Err(GbaPersistenceError::AudioRateMismatch(
            payload.audio_rate,
            audio_rate,
        ));
    }
    let mut candidate = GbaSystem::from_test_rom(rom.to_vec())
        .or_else(|| GbaSystem::from_rom(rom.to_vec()))
        .ok_or(GbaPersistenceError::InvalidState(
            "retained ROM no longer parses".to_string(),
        ))?;
    candidate
        .import_state(payload.system)
        .map_err(GbaPersistenceError::InvalidState)?;
    Ok(candidate)
}

pub(crate) fn export_mapper_save(
    cartridge: &Cartridge,
    rom_identity: GbaRomIdentity,
) -> Result<Option<Vec<u8>>, GbaPersistenceError> {
    let Some(ram) = cartridge.save.ram_data() else {
        return Ok(None);
    };
    Ok(Some(rmp_serde::to_vec_named(&MapperSavePayload {
        schema_version: PERSISTENCE_SCHEMA_VERSION,
        rom_identity,
        save_kind: SaveTypeSer::from(cartridge.save.save_type()),
        cartridge: serde_bytes::ByteBuf::from(ram.to_vec()),
    })?))
}

/// Validate the mapper-save envelope fully before touching the backend, then
/// apply and read back. Other devices are never modified.
pub(crate) fn import_mapper_save(
    cartridge: &mut Cartridge,
    data: &[u8],
    expected_identity: &GbaRomIdentity,
) -> Result<(), GbaPersistenceError> {
    let payload: MapperSavePayload = rmp_serde::from_slice(data)?;
    validate_version(payload.schema_version)?;
    if &payload.rom_identity != expected_identity {
        return Err(GbaPersistenceError::RomIdentityMismatch);
    }
    let expected_kind = SaveTypeSer::from(cartridge.save.save_type());
    if payload.save_kind != expected_kind {
        return Err(GbaPersistenceError::Cartridge(format!(
            "mapper save kind mismatch: wire={:?} actual={expected_kind:?}",
            payload.save_kind
        )));
    }
    let Some(current) = cartridge.save.ram_data() else {
        return Err(GbaPersistenceError::Cartridge(
            "mapper save for a battery-less cartridge".to_string(),
        ));
    };
    if payload.cartridge.len() != current.len() {
        return Err(GbaPersistenceError::Cartridge(format!(
            "mapper save length mismatch: wire={} actual={}",
            payload.cartridge.len(),
            current.len()
        )));
    }
    // `ram_restore` truncates silently and reports nothing, so the read-back
    // below is the real acceptance check.
    cartridge.save.ram_restore(&payload.cartridge);
    let applied = cartridge.save.ram_data().ok_or_else(|| {
        GbaPersistenceError::Cartridge("backend lost its RAM on restore".to_string())
    })?;
    if applied != payload.cartridge.as_ref() {
        return Err(GbaPersistenceError::Cartridge(
            "mapper save read-back mismatch".to_string(),
        ));
    }
    Ok(())
}
