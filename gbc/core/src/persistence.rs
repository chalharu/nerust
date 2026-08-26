use crate::{
    core_options::GbcCoreOptions, cpu_core::CpuState, memory::MemoryState,
    persistence_error::GbcPersistenceError, rom_identity::GbcRomIdentity, system::GbcSystem,
};

const PERSISTENCE_SCHEMA_VERSION: u32 = 1;

#[derive(serde::Serialize, serde::Deserialize)]
struct MachineStatePayload {
    schema_version: u32,
    rom_identity: GbcRomIdentity,
    options: GbcCoreOptions,
    cpu: CpuState,
    memory: MemoryState,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MapperSavePayload {
    schema_version: u32,
    rom_identity: GbcRomIdentity,
    cartridge: Vec<u8>,
}

pub(crate) fn export_machine_state(
    system: &GbcSystem,
    rom_identity: GbcRomIdentity,
    options: GbcCoreOptions,
) -> Result<Vec<u8>, GbcPersistenceError> {
    Ok(rmp_serde::to_vec_named(&MachineStatePayload {
        schema_version: PERSISTENCE_SCHEMA_VERSION,
        rom_identity,
        options,
        cpu: system.cpu.export_state(),
        memory: system
            .bus
            .export_state()
            .map_err(GbcPersistenceError::InvalidState)?,
    })?)
}

pub(crate) fn import_machine_state(
    system: &mut GbcSystem,
    data: &[u8],
    expected_identity: GbcRomIdentity,
    expected_options: GbcCoreOptions,
) -> Result<(), GbcPersistenceError> {
    let payload: MachineStatePayload = rmp_serde::from_slice(data)?;
    validate_version(payload.schema_version)?;
    if payload.rom_identity != expected_identity {
        return Err(GbcPersistenceError::RomIdentityMismatch);
    }
    if payload.options != expected_options {
        return Err(GbcPersistenceError::OptionsMismatch);
    }
    let mut cpu = crate::cpu_core::Lr35902Cpu::new();
    cpu.import_state(payload.cpu)
        .map_err(GbcPersistenceError::InvalidState)?;
    system
        .bus
        .import_state(payload.memory)
        .map_err(GbcPersistenceError::InvalidState)?;
    system.cpu = cpu;
    Ok(())
}

pub(crate) fn export_mapper_save(
    system: &GbcSystem,
    rom_identity: GbcRomIdentity,
    now: std::time::SystemTime,
) -> Result<Option<Vec<u8>>, GbcPersistenceError> {
    let Some(cartridge) = system
        .bus
        .export_cartridge_save(now)
        .map_err(GbcPersistenceError::InvalidState)?
    else {
        return Ok(None);
    };
    Ok(Some(rmp_serde::to_vec_named(&MapperSavePayload {
        schema_version: PERSISTENCE_SCHEMA_VERSION,
        rom_identity,
        cartridge,
    })?))
}

pub(crate) fn import_mapper_save(
    system: &mut GbcSystem,
    data: &[u8],
    expected_identity: GbcRomIdentity,
) -> Result<(), GbcPersistenceError> {
    let payload: MapperSavePayload = rmp_serde::from_slice(data)?;
    validate_version(payload.schema_version)?;
    if payload.rom_identity != expected_identity {
        return Err(GbcPersistenceError::RomIdentityMismatch);
    }
    system
        .bus
        .import_cartridge_save(&payload.cartridge)
        .map_err(GbcPersistenceError::InvalidState)
}

fn validate_version(version: u32) -> Result<(), GbcPersistenceError> {
    if version == PERSISTENCE_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(GbcPersistenceError::UnsupportedVersion(version))
    }
}
