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
    cpu.import_state(payload.cpu, |opcode| {
        crate::cpu_opcodes::handler_table()[usize::from(opcode)]
    })
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

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use crate::{
        core_options::RtcSyncPolicy,
        system::{GbcSystem, HardwareModel},
    };

    use super::*;

    fn rom(cartridge_type: u8, ram_size: u8, marker: u8) -> Vec<u8> {
        let mut rom = vec![0; 0x8000];
        rom[0x0100] = 0x18;
        rom[0x0101] = 0xFE;
        rom[0x0143] = 0x80;
        rom[0x0147] = cartridge_type;
        rom[0x0148] = 0;
        rom[0x0149] = ram_size;
        rom[0x2000] = marker;
        crate::cartridge_header::finalize_test_rom(&mut rom);
        rom
    }

    fn options(model: HardwareModel) -> GbcCoreOptions {
        GbcCoreOptions {
            hardware_model: model,
            rtc_sync: RtcSyncPolicy::Off,
        }
    }

    fn system(rom: &[u8], model: HardwareModel) -> GbcSystem {
        GbcSystem::from_rom(model, rom.to_vec()).unwrap()
    }

    #[test]
    fn machine_state_round_trip_and_validation_errors() {
        let rom_bytes = rom(0, 0, 0);
        let identity = GbcRomIdentity::from_rom(&rom_bytes).unwrap();
        let core_options = options(HardwareModel::CgbD);
        let source = system(&rom_bytes, HardwareModel::CgbD);
        let bytes = export_machine_state(&source, identity, core_options).unwrap();
        let mut target = system(&rom_bytes, HardwareModel::CgbD);
        import_machine_state(&mut target, &bytes, identity, core_options).unwrap();

        let wrong_options = options(HardwareModel::Dmg);
        assert!(matches!(
            import_machine_state(&mut target, &bytes, identity, wrong_options),
            Err(GbcPersistenceError::OptionsMismatch)
        ));
        let other_identity = GbcRomIdentity::from_rom(&rom(0, 0, 1)).unwrap();
        assert!(matches!(
            import_machine_state(&mut target, &bytes, other_identity, core_options),
            Err(GbcPersistenceError::RomIdentityMismatch)
        ));
        assert!(matches!(
            import_machine_state(&mut target, b"not msgpack", identity, core_options),
            Err(GbcPersistenceError::Decode(_))
        ));
    }

    #[test]
    fn machine_state_rejects_unknown_version() {
        let rom = rom(0, 0, 0);
        let identity = GbcRomIdentity::from_rom(&rom).unwrap();
        let options = options(HardwareModel::CgbD);
        let source = system(&rom, HardwareModel::CgbD);
        let bytes = export_machine_state(&source, identity, options).unwrap();
        let mut payload: MachineStatePayload = rmp_serde::from_slice(&bytes).unwrap();
        payload.schema_version += 1;
        let bytes = rmp_serde::to_vec_named(&payload).unwrap();
        let mut target = system(&rom, HardwareModel::CgbD);
        assert!(matches!(
            import_machine_state(&mut target, &bytes, identity, options),
            Err(GbcPersistenceError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn mapper_save_handles_absent_battery_and_identity_mismatch() {
        let plain_rom = rom(0, 0, 0);
        let plain_identity = GbcRomIdentity::from_rom(&plain_rom).unwrap();
        let plain = system(&plain_rom, HardwareModel::CgbD);
        assert!(
            export_mapper_save(&plain, plain_identity, UNIX_EPOCH)
                .unwrap()
                .is_none()
        );

        let battery_rom = rom(0x03, 0x02, 0);
        let battery_identity = GbcRomIdentity::from_rom(&battery_rom).unwrap();
        let source = system(&battery_rom, HardwareModel::CgbD);
        let bytes = export_mapper_save(&source, battery_identity, UNIX_EPOCH)
            .unwrap()
            .unwrap();
        let mut target = system(&battery_rom, HardwareModel::CgbD);
        import_mapper_save(&mut target, &bytes, battery_identity).unwrap();

        assert!(matches!(
            import_mapper_save(&mut target, &bytes, plain_identity),
            Err(GbcPersistenceError::RomIdentityMismatch)
        ));
    }

    #[test]
    fn mapper_save_rejects_unknown_version_and_invalid_bytes() {
        let rom = rom(0x03, 0x02, 0);
        let identity = GbcRomIdentity::from_rom(&rom).unwrap();
        let source = system(&rom, HardwareModel::CgbD);
        let bytes = export_mapper_save(&source, identity, UNIX_EPOCH)
            .unwrap()
            .unwrap();
        let mut payload: MapperSavePayload = rmp_serde::from_slice(&bytes).unwrap();
        payload.schema_version += 1;
        let bytes = rmp_serde::to_vec_named(&payload).unwrap();
        let mut target = system(&rom, HardwareModel::CgbD);
        assert!(matches!(
            import_mapper_save(&mut target, &bytes, identity),
            Err(GbcPersistenceError::UnsupportedVersion(2))
        ));
        assert!(matches!(
            import_mapper_save(&mut target, b"invalid", identity),
            Err(GbcPersistenceError::Decode(_))
        ));
    }
}
