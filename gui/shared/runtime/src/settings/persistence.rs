use super::{SettingsError, SettingsPaths};
use crc::{CRC_32_ISO_HDLC, Crc};
use nerust_contract_mirror::MirrorMode;
use nerust_contract_rom::RomIdentity;
use nerust_gui_settings::shared::{DesktopSharedSettings, StoragePolicy};
use nerust_input_schema::SystemId;
use nerust_persistence::sidecar::{SidecarPaths, resolve_sidecars};
use std::fs;
use std::path::Path;

const MAPPER_SAVE_FILE_NAME: &str = "mapper.sav";
const STATES_DIR_NAME: &str = "states";

pub fn resolve_persistence_paths(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: SystemId,
    rom_path: Option<&Path>,
    rom_identity: RomIdentity,
) -> Result<SidecarPaths, SettingsError> {
    resolve_current_persistence_paths(shared, paths, system, rom_path, rom_identity)
}

pub fn resolve_persistence_paths_with_import(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: SystemId,
    rom_path: Option<&Path>,
    rom_identity: RomIdentity,
) -> Result<SidecarPaths, SettingsError> {
    let resolved =
        resolve_current_persistence_paths(shared, paths, system, rom_path, rom_identity)?;
    maybe_auto_import_storage(shared, paths, system, rom_path, rom_identity, &resolved)?;
    Ok(resolved)
}

pub fn system_storage_key(system: SystemId, rom_identity: RomIdentity) -> String {
    let canonical = canonical_rom_identity(system, rom_identity);
    let checksum = Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(canonical.as_bytes());
    format!(
        "m{:04x}-s{:02x}-p{:016x}-c{:016x}-t{:016x}-{:08x}",
        rom_identity.mapper_type,
        rom_identity.sub_mapper_type,
        rom_identity.prg_rom_crc64,
        rom_identity.chr_rom_crc64,
        rom_identity.trainer_crc64,
        checksum
    )
}

pub fn resolve_central_storage_paths(
    root: &Path,
    system: SystemId,
    rom_identity: RomIdentity,
) -> SidecarPaths {
    let base = root
        .join(system_id_slug(system))
        .join(system_storage_key(system, rom_identity));
    SidecarPaths {
        mapper_save_path: base.join(MAPPER_SAVE_FILE_NAME),
        states_dir: base.join(STATES_DIR_NAME),
    }
}

fn resolve_current_persistence_paths(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: SystemId,
    rom_path: Option<&Path>,
    rom_identity: RomIdentity,
) -> Result<SidecarPaths, SettingsError> {
    match shared.persistence.storage_policy {
        StoragePolicy::Sidecar => {
            let rom_path = rom_path.ok_or(SettingsError::PersistenceUnavailable)?;
            Ok(resolve_sidecars(rom_path))
        }
        StoragePolicy::AppSharedData => {
            let Some(paths) = paths else {
                return Err(SettingsError::PersistenceUnavailable);
            };
            Ok(resolve_central_storage_paths(
                &paths.central_storage_root,
                system,
                rom_identity,
            ))
        }
        StoragePolicy::CustomDirectory => {
            let root = shared
                .persistence
                .storage_directory
                .as_deref()
                .ok_or(SettingsError::MissingCustomStorageDirectory)?;
            Ok(resolve_central_storage_paths(root, system, rom_identity))
        }
    }
}

fn maybe_auto_import_storage(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: SystemId,
    rom_path: Option<&Path>,
    rom_identity: RomIdentity,
    destination: &SidecarPaths,
) -> Result<(), SettingsError> {
    let Some(rom_path) = rom_path else {
        return Ok(());
    };
    if !storage_is_empty(destination)? {
        return Ok(());
    }

    match shared.persistence.storage_policy {
        StoragePolicy::Sidecar => {
            if let Some(paths) = paths {
                let app_shared = resolve_central_storage_paths(
                    &paths.central_storage_root,
                    system,
                    rom_identity,
                );
                if !storage_is_empty(&app_shared)? {
                    copy_storage_contents(&app_shared, destination)?;
                    return Ok(());
                }
            }
            if let Some(root) = shared.persistence.storage_directory.as_deref() {
                let custom = resolve_central_storage_paths(root, system, rom_identity);
                if !storage_is_empty(&custom)? {
                    copy_storage_contents(&custom, destination)?;
                }
            }
        }
        StoragePolicy::AppSharedData | StoragePolicy::CustomDirectory => {
            let sidecar = resolve_sidecars(rom_path);
            if !storage_is_empty(&sidecar)? {
                copy_storage_contents(&sidecar, destination)?;
                return Ok(());
            }
            match shared.persistence.storage_policy {
                StoragePolicy::AppSharedData => {
                    if let Some(root) = shared.persistence.storage_directory.as_deref() {
                        let custom = resolve_central_storage_paths(root, system, rom_identity);
                        if !storage_is_empty(&custom)? {
                            copy_storage_contents(&custom, destination)?;
                        }
                    }
                }
                StoragePolicy::CustomDirectory => {
                    if let Some(paths) = paths {
                        let app_shared = resolve_central_storage_paths(
                            &paths.central_storage_root,
                            system,
                            rom_identity,
                        );
                        if !storage_is_empty(&app_shared)? {
                            copy_storage_contents(&app_shared, destination)?;
                        }
                    }
                }
                StoragePolicy::Sidecar => {}
            }
        }
    }
    Ok(())
}

fn canonical_rom_identity(system: SystemId, rom_identity: RomIdentity) -> String {
    format!(
        "v1|{}|{:04x}|{:02x}|{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{:016x}|{:016x}|{:016x}",
        system_id_slug(system),
        rom_identity.mapper_type,
        rom_identity.sub_mapper_type,
        rom_identity.format,
        rom_identity.has_battery,
        rom_identity.trainer_len,
        rom_identity.prg_rom_len,
        rom_identity.chr_rom_len,
        rom_identity.prg_ram_len,
        rom_identity.save_prg_ram_len,
        rom_identity.chr_ram_len,
        rom_identity.save_chr_ram_len,
        mirror_mode_slug(rom_identity.mirror_mode),
        rom_identity.prg_rom_crc64,
        rom_identity.chr_rom_crc64,
        rom_identity.trainer_crc64,
    )
}

fn mirror_mode_slug(mode: MirrorMode) -> String {
    match mode {
        MirrorMode::Horizontal => "horizontal".to_string(),
        MirrorMode::Vertical => "vertical".to_string(),
        MirrorMode::Single0 => "single0".to_string(),
        MirrorMode::Single1 => "single1".to_string(),
        MirrorMode::Four => "four".to_string(),
        MirrorMode::Custom(lut) => {
            let mut text = "custom".to_string();
            for value in lut {
                text.push_str(format!("-{value}").as_str());
            }
            text
        }
    }
}

fn system_id_slug(system: SystemId) -> &'static str {
    match system {
        SystemId::Nes => "nes",
        SystemId::Snes => "snes",
        SystemId::Ps1 => "ps1",
        SystemId::MegaDrive => "megadrive",
    }
}

fn storage_is_empty(paths: &SidecarPaths) -> Result<bool, SettingsError> {
    let mapper_exists = paths.mapper_save_path.is_file();
    let states_exist = match fs::read_dir(&paths.states_dir) {
        Ok(mut entries) => entries.next().is_some(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    Ok(!mapper_exists && !states_exist)
}

fn copy_storage_contents(
    source: &SidecarPaths,
    destination: &SidecarPaths,
) -> Result<(), SettingsError> {
    if source.mapper_save_path.is_file() {
        if let Some(parent) = destination.mapper_save_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _ = fs::copy(&source.mapper_save_path, &destination.mapper_save_path)?;
    }
    match fs::read_dir(&source.states_dir) {
        Ok(entries) => {
            fs::create_dir_all(&destination.states_dir)?;
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    let _ = fs::copy(&path, destination.states_dir.join(entry.file_name()))?;
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
