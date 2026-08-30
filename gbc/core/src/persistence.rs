use crate::{
    core_options::GbcCoreOptions, cpu_core::CpuState, memory::MemoryState,
    persistence_error::GbcPersistenceError, rom_identity::GbcRomIdentity, system::GbcSystem,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MIN_SUPPORTED_SCHEMA_VERSION: u32 = 1;
const PERSISTENCE_SCHEMA_VERSION: u32 = 2;

#[derive(serde::Serialize, serde::Deserialize)]
struct MachineStatePayload {
    schema_version: u32,
    #[serde(default)]
    captured_at_unix_seconds: Option<u64>,
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
    now: SystemTime,
) -> Result<Vec<u8>, GbcPersistenceError> {
    Ok(rmp_serde::to_vec_named(&MachineStatePayload {
        schema_version: PERSISTENCE_SCHEMA_VERSION,
        captured_at_unix_seconds: Some(unix_seconds(now)),
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
    now: SystemTime,
) -> Result<(), GbcPersistenceError> {
    let payload: MachineStatePayload = rmp_serde::from_slice(data)?;
    validate_version(payload.schema_version)?;
    if payload.rom_identity != expected_identity {
        return Err(GbcPersistenceError::RomIdentityMismatch);
    }
    if payload.options.hardware_model != expected_options.hardware_model {
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
    if expected_options.rtc_sync.syncs_snapshots()
        && let Some(captured_at) = payload.captured_at_unix_seconds
        && let Some(captured_at) = UNIX_EPOCH.checked_add(Duration::from_secs(captured_at))
    {
        system.bus.sync_cartridge_rtc_from(captured_at, now);
    }
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
    if (MIN_SUPPORTED_SCHEMA_VERSION..=PERSISTENCE_SCHEMA_VERSION).contains(&version) {
        Ok(())
    } else {
        Err(GbcPersistenceError::UnsupportedVersion(version))
    }
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use crate::{
        core_options::RtcSyncPolicy,
        system::{GbcSystem, HardwareModel},
    };

    use super::*;

    #[derive(serde::Serialize)]
    struct LegacyMachineStatePayload {
        schema_version: u32,
        rom_identity: GbcRomIdentity,
        options: GbcCoreOptions,
        cpu: CpuState,
        memory: MemoryState,
    }

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
        let bytes = export_machine_state(&source, identity, core_options, UNIX_EPOCH).unwrap();
        let mut target = system(&rom_bytes, HardwareModel::CgbD);
        import_machine_state(&mut target, &bytes, identity, core_options, UNIX_EPOCH).unwrap();

        let changed_rtc_policy = GbcCoreOptions {
            rtc_sync: RtcSyncPolicy::SaveDataOnly,
            ..core_options
        };
        import_machine_state(
            &mut target,
            &bytes,
            identity,
            changed_rtc_policy,
            UNIX_EPOCH,
        )
        .unwrap();

        let wrong_options = options(HardwareModel::Dmg);
        assert!(matches!(
            import_machine_state(&mut target, &bytes, identity, wrong_options, UNIX_EPOCH),
            Err(GbcPersistenceError::OptionsMismatch)
        ));
        let other_identity = GbcRomIdentity::from_rom(&rom(0, 0, 1)).unwrap();
        assert!(matches!(
            import_machine_state(
                &mut target,
                &bytes,
                other_identity,
                core_options,
                UNIX_EPOCH
            ),
            Err(GbcPersistenceError::RomIdentityMismatch)
        ));
        assert!(matches!(
            import_machine_state(
                &mut target,
                b"not msgpack",
                identity,
                core_options,
                UNIX_EPOCH
            ),
            Err(GbcPersistenceError::Decode(_))
        ));
    }

    #[test]
    fn machine_state_rejects_unknown_version() {
        let rom = rom(0, 0, 0);
        let identity = GbcRomIdentity::from_rom(&rom).unwrap();
        let options = options(HardwareModel::CgbD);
        let source = system(&rom, HardwareModel::CgbD);
        let bytes = export_machine_state(&source, identity, options, UNIX_EPOCH).unwrap();
        let mut payload: MachineStatePayload = rmp_serde::from_slice(&bytes).unwrap();
        payload.schema_version += 1;
        let bytes = rmp_serde::to_vec_named(&payload).unwrap();
        let mut target = system(&rom, HardwareModel::CgbD);
        assert!(matches!(
            import_machine_state(&mut target, &bytes, identity, options, UNIX_EPOCH),
            Err(GbcPersistenceError::UnsupportedVersion(3))
        ));
    }

    fn rtc_options(policy: RtcSyncPolicy) -> GbcCoreOptions {
        GbcCoreOptions {
            hardware_model: HardwareModel::CgbD,
            rtc_sync: policy,
        }
    }

    fn write_rtc_seconds(system: &mut GbcSystem, seconds: u8) {
        system.bus.write(0x0000, 0x0A);
        system.bus.write(0x4000, 0x08);
        system.bus.write(0xA000, seconds);
    }

    fn read_rtc_seconds(system: &mut GbcSystem) -> u8 {
        system.bus.write(0x6000, 0);
        system.bus.write(0x6000, 1);
        system.bus.read(0xA000)
    }

    fn huc3_command(system: &mut GbcSystem, command: u8, argument: u8) -> u8 {
        system.bus.write(0x0000, 0x0B);
        system.bus.write(0xA000, command << 4 | argument);
        system.bus.write(0x0000, 0x0D);
        system.bus.write(0xA000, 0xFE);
        system.bus.write(0x0000, 0x0C);
        system.bus.read(0xA000)
    }

    fn set_huc3_address(system: &mut GbcSystem, address: u8) {
        huc3_command(system, 4, address & 0x0F);
        huc3_command(system, 5, address >> 4);
    }

    fn read_huc3_rtc_nibble(system: &mut GbcSystem, address: u8) -> u8 {
        set_huc3_address(system, address);
        huc3_command(system, 1, 0) & 0x0F
    }

    #[test]
    fn system_time_snapshot_sync_applies_elapsed_time() {
        let rom = rom(0x0F, 0, 0);
        let identity = GbcRomIdentity::from_rom(&rom).unwrap();
        let options = rtc_options(RtcSyncPolicy::SystemTime);
        let mut source = system(&rom, HardwareModel::CgbD);
        write_rtc_seconds(&mut source, 10);
        let captured_at = UNIX_EPOCH + Duration::from_secs(100);
        let bytes = export_machine_state(&source, identity, options, captured_at).unwrap();

        let mut target = system(&rom, HardwareModel::CgbD);
        import_machine_state(
            &mut target,
            &bytes,
            identity,
            options,
            captured_at + Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(read_rtc_seconds(&mut target), 15);
    }

    #[test]
    fn save_data_only_snapshot_sync_restores_exact_rtc() {
        let rom = rom(0x0F, 0, 0);
        let identity = GbcRomIdentity::from_rom(&rom).unwrap();
        let options = rtc_options(RtcSyncPolicy::SaveDataOnly);
        let mut source = system(&rom, HardwareModel::CgbD);
        write_rtc_seconds(&mut source, 10);
        let captured_at = UNIX_EPOCH + Duration::from_secs(100);
        let bytes = export_machine_state(&source, identity, options, captured_at).unwrap();

        let mut target = system(&rom, HardwareModel::CgbD);
        import_machine_state(
            &mut target,
            &bytes,
            identity,
            options,
            captured_at + Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(read_rtc_seconds(&mut target), 10);
    }

    #[test]
    fn legacy_snapshot_without_timestamp_restores_exact_rtc() {
        let rom = rom(0x0F, 0, 0);
        let identity = GbcRomIdentity::from_rom(&rom).unwrap();
        let options = rtc_options(RtcSyncPolicy::SystemTime);
        let mut source = system(&rom, HardwareModel::CgbD);
        write_rtc_seconds(&mut source, 10);
        let bytes = rmp_serde::to_vec_named(&LegacyMachineStatePayload {
            schema_version: 1,
            rom_identity: identity,
            options,
            cpu: source.cpu.export_state(),
            memory: source.bus.export_state().unwrap(),
        })
        .unwrap();

        let mut target = system(&rom, HardwareModel::CgbD);
        import_machine_state(
            &mut target,
            &bytes,
            identity,
            options,
            UNIX_EPOCH + Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(read_rtc_seconds(&mut target), 10);
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
            Err(GbcPersistenceError::UnsupportedVersion(3))
        ));
        assert!(matches!(
            import_mapper_save(&mut target, b"invalid", identity),
            Err(GbcPersistenceError::Decode(_))
        ));
    }

    #[test]
    fn huc_mapper_saves_round_trip_and_reject_cross_mapper_payload() {
        let huc1_rom = rom(0xFF, 0x02, 0);
        let huc1_identity = GbcRomIdentity::from_rom(&huc1_rom).unwrap();
        let mut huc1_source = system(&huc1_rom, HardwareModel::CgbD);
        huc1_source.bus.write(0xA000, 0x51);
        let huc1_save = export_mapper_save(&huc1_source, huc1_identity, UNIX_EPOCH)
            .unwrap()
            .unwrap();
        let mut huc1_target = system(&huc1_rom, HardwareModel::CgbD);
        import_mapper_save(&mut huc1_target, &huc1_save, huc1_identity).unwrap();
        assert_eq!(huc1_target.bus.read(0xA000), 0x51);

        let huc3_rom = rom(0xFE, 0x02, 0);
        let huc3_identity = GbcRomIdentity::from_rom(&huc3_rom).unwrap();
        let mut huc3_source = system(&huc3_rom, HardwareModel::CgbD);
        huc3_source.bus.write(0x0000, 0x0A);
        huc3_source.bus.write(0xA000, 0x53);
        set_huc3_address(&mut huc3_source, 0x42);
        huc3_command(&mut huc3_source, 3, 0x0C);
        let huc3_save = export_mapper_save(&huc3_source, huc3_identity, UNIX_EPOCH)
            .unwrap()
            .unwrap();
        let mut huc3_target = system(&huc3_rom, HardwareModel::CgbD);
        import_mapper_save(&mut huc3_target, &huc3_save, huc3_identity).unwrap();
        huc3_target.bus.write(0x0000, 0x0A);
        assert_eq!(huc3_target.bus.read(0xA000), 0x53);
        assert_eq!(read_huc3_rtc_nibble(&mut huc3_target, 0x42), 0x0C);

        let huc1_payload: MapperSavePayload = rmp_serde::from_slice(&huc1_save).unwrap();
        let cross_mapper = rmp_serde::to_vec_named(&MapperSavePayload {
            schema_version: PERSISTENCE_SCHEMA_VERSION,
            rom_identity: huc3_identity,
            cartridge: huc1_payload.cartridge,
        })
        .unwrap();
        assert!(matches!(
            import_mapper_save(&mut huc3_target, &cross_mapper, huc3_identity),
            Err(GbcPersistenceError::InvalidState(_))
        ));
    }

    #[test]
    fn huc3_snapshot_sync_respects_rtc_policy() {
        let rom = rom(0xFE, 0x02, 0);
        let identity = GbcRomIdentity::from_rom(&rom).unwrap();
        let captured_at = UNIX_EPOCH + Duration::from_secs(100);
        let source = system(&rom, HardwareModel::CgbD);

        let system_time_options = rtc_options(RtcSyncPolicy::SystemTime);
        let bytes =
            export_machine_state(&source, identity, system_time_options, captured_at).unwrap();
        let mut synced = system(&rom, HardwareModel::CgbD);
        import_machine_state(
            &mut synced,
            &bytes,
            identity,
            system_time_options,
            captured_at + Duration::from_secs(120),
        )
        .unwrap();
        huc3_command(&mut synced, 6, 0);
        assert_eq!(read_huc3_rtc_nibble(&mut synced, 0), 2);

        let exact_options = rtc_options(RtcSyncPolicy::SaveDataOnly);
        let bytes = export_machine_state(&source, identity, exact_options, captured_at).unwrap();
        let mut exact = system(&rom, HardwareModel::CgbD);
        import_machine_state(
            &mut exact,
            &bytes,
            identity,
            exact_options,
            captured_at + Duration::from_secs(120),
        )
        .unwrap();
        huc3_command(&mut exact, 6, 0);
        assert_eq!(read_huc3_rtc_nibble(&mut exact, 0), 0);
    }
}
