use std::{fs, path::Path};

use crc::{CRC_32_ISO_HDLC, Crc};
use nerust_core_traits::identity::{SystemId, SystemIdentity};
use nerust_gui_settings::shared::{DesktopSharedSettings, StoragePolicy};
use nerust_persistence::sidecar::{SidecarPaths, resolve_sidecars};

use super::{SettingsError, SettingsPaths};

const MAPPER_SAVE_FILE_NAME: &str = "mapper.sav";
const STATES_DIR_NAME: &str = "states";
const DOCUMENT_TREE_MARKER: &str = "__nerust_document_tree__";

pub fn resolve_persistence_paths(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: &dyn SystemId,
    rom_path: Option<&Path>,
    identity: &SystemIdentity,
) -> Result<SidecarPaths, SettingsError> {
    resolve_current_persistence_paths(shared, paths, system, rom_path, identity)
}

pub fn resolve_persistence_paths_with_import(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: &dyn SystemId,
    rom_path: Option<&Path>,
    identity: &SystemIdentity,
) -> Result<SidecarPaths, SettingsError> {
    let resolved = resolve_current_persistence_paths(shared, paths, system, rom_path, identity)?;
    maybe_auto_import_storage(shared, paths, system, rom_path, identity, &resolved)?;
    Ok(resolved)
}

pub fn system_storage_key(_system: &dyn SystemId, identity: &SystemIdentity) -> String {
    let checksum = Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(&identity.identity_bytes);
    format!("{:08x}-{:08x}", identity.identity_bytes.len(), checksum)
}

pub fn resolve_central_storage_paths(
    root: &Path,
    system: &dyn SystemId,
    identity: &SystemIdentity,
) -> SidecarPaths {
    let base = root
        .join(system.to_string())
        .join(system_storage_key(system, identity));
    SidecarPaths {
        mapper_save_path: base.join(MAPPER_SAVE_FILE_NAME),
        states_dir: base.join(STATES_DIR_NAME),
    }
}

pub fn resolve_document_tree_storage_paths(
    uri: &str,
    system: &dyn SystemId,
    identity: &SystemIdentity,
) -> SidecarPaths {
    let encoded_uri = uri
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let base = Path::new(DOCUMENT_TREE_MARKER)
        .join(encoded_uri)
        .join(system.to_string())
        .join(system_storage_key(system, identity));
    SidecarPaths {
        mapper_save_path: base.join(MAPPER_SAVE_FILE_NAME),
        states_dir: base.join(STATES_DIR_NAME),
    }
}

pub fn resolve_document_tree_sidecar_paths(
    uri: &str,
    system: &dyn SystemId,
    identity: &SystemIdentity,
) -> SidecarPaths {
    let encoded_uri = uri
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let base = Path::new(DOCUMENT_TREE_MARKER).join(encoded_uri);
    let key = format!("{}-{}", system, system_storage_key(system, identity));
    SidecarPaths {
        mapper_save_path: base.join(format!("{key}.sav")),
        states_dir: base.join(format!("{key}.states")),
    }
}

pub fn decode_document_tree_path(path: &Path) -> Option<(String, String)> {
    let mut components = path.components();
    if components.next()?.as_os_str() != DOCUMENT_TREE_MARKER {
        return None;
    }
    let encoded = components.next()?.as_os_str().to_str()?;
    if encoded.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..encoded.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    let uri = String::from_utf8(bytes).ok()?;
    let relative = components
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?
        .join("/");
    Some((uri, relative))
}

fn resolve_current_persistence_paths(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: &dyn SystemId,
    rom_path: Option<&Path>,
    identity: &SystemIdentity,
) -> Result<SidecarPaths, SettingsError> {
    match shared.persistence.storage_policy {
        StoragePolicy::Sidecar => {
            if let Some(uri) = shared.persistence.storage_document_tree_uri.as_deref() {
                return Ok(resolve_document_tree_sidecar_paths(uri, system, identity));
            }
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
                identity,
            ))
        }
        StoragePolicy::CustomDirectory => {
            if let Some(uri) = shared.persistence.storage_document_tree_uri.as_deref() {
                return Ok(resolve_document_tree_storage_paths(uri, system, identity));
            }
            let root = shared
                .persistence
                .storage_directory
                .as_deref()
                .ok_or(SettingsError::MissingCustomStorageDirectory)?;
            Ok(resolve_central_storage_paths(root, system, identity))
        }
    }
}

fn maybe_auto_import_storage(
    shared: &DesktopSharedSettings,
    paths: Option<&SettingsPaths>,
    system: &dyn SystemId,
    rom_path: Option<&Path>,
    identity: &SystemIdentity,
    destination: &SidecarPaths,
) -> Result<(), SettingsError> {
    if decode_document_tree_path(&destination.mapper_save_path).is_some() {
        return Ok(());
    }
    let Some(rom_path) = rom_path else {
        return Ok(());
    };
    if !storage_is_empty(destination)? {
        return Ok(());
    }

    match shared.persistence.storage_policy {
        StoragePolicy::Sidecar => {
            if let Some(paths) = paths {
                let app_shared =
                    resolve_central_storage_paths(&paths.central_storage_root, system, identity);
                if !storage_is_empty(&app_shared)? {
                    copy_storage_contents(&app_shared, destination)?;
                    return Ok(());
                }
            }
            if let Some(root) = shared.persistence.storage_directory.as_deref() {
                let custom = resolve_central_storage_paths(root, system, identity);
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
                        let custom = resolve_central_storage_paths(root, system, identity);
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
                            identity,
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

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use nerust_gui_settings::shared::StoragePolicy;
    use nerust_persistence::sidecar::resolve_sidecars;

    use crate::test::DummySystemId;

    use super::{
        super::{SettingsPaths, test_root, test_shared_defaults, test_system_identity},
        decode_document_tree_path, resolve_central_storage_paths,
        resolve_document_tree_sidecar_paths, resolve_document_tree_storage_paths,
        resolve_persistence_paths_with_import, system_storage_key,
    };

    #[test]
    fn central_storage_paths_use_system_and_identity_not_rom_path() {
        let root = PathBuf::from("/base");
        let identity = test_system_identity();
        let first = resolve_central_storage_paths(&root, &DummySystemId, &identity);
        let second = resolve_central_storage_paths(&root, &DummySystemId, &identity);

        assert_eq!(first, second);
        assert!(first.mapper_save_path.ends_with("mapper.sav"));
        assert!(first.states_dir.ends_with("states"));
        assert!(!system_storage_key(&DummySystemId, &identity).is_empty());
    }

    #[test]
    fn document_tree_paths_round_trip_uri_and_relative_name() {
        let identity = test_system_identity();
        let paths = resolve_document_tree_storage_paths(
            "content://provider/tree/primary%3ASaves",
            identity.system_id.as_ref(),
            &identity,
        );

        let (uri, relative) = decode_document_tree_path(&paths.mapper_save_path).unwrap();
        assert_eq!(uri, "content://provider/tree/primary%3ASaves");
        assert!(relative.starts_with(&format!("{}/", identity.system_id)));
        assert!(relative.ends_with("/mapper.sav"));
    }

    #[test]
    fn document_tree_sidecars_are_siblings_in_selected_tree() {
        let identity = test_system_identity();
        let paths = resolve_document_tree_sidecar_paths(
            "content://provider/tree/primary%3ASaves",
            identity.system_id.as_ref(),
            &identity,
        );
        let (_, mapper) = decode_document_tree_path(&paths.mapper_save_path).unwrap();
        let (_, states) = decode_document_tree_path(&paths.states_dir).unwrap();

        assert!(!mapper.contains('/'));
        assert!(!states.contains('/'));
        assert!(mapper.ends_with(".sav"));
        assert!(states.ends_with(".states"));
    }

    #[test]
    fn sidecar_imports_into_empty_central_storage() {
        let root = test_root("import-sidecar-to-central");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let rom_path = root.join("game.nes");
        fs::write(&rom_path, [0_u8; 4]).unwrap();
        let sidecar = resolve_sidecars(&rom_path);
        fs::write(&sidecar.mapper_save_path, b"mapper").unwrap();
        fs::create_dir_all(&sidecar.states_dir).unwrap();
        fs::write(sidecar.states_dir.join("slot-1.zip"), b"state").unwrap();

        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::CustomDirectory;
        shared.persistence.storage_directory = Some(root.join("central"));

        let identity = test_system_identity();
        let resolved = resolve_persistence_paths_with_import(
            &shared,
            None,
            &DummySystemId,
            Some(&rom_path),
            &identity,
        )
        .unwrap();

        assert_eq!(fs::read(&resolved.mapper_save_path).unwrap(), b"mapper");
        assert_eq!(
            fs::read(resolved.states_dir.join("slot-1.zip")).unwrap(),
            b"state"
        );
    }

    #[test]
    fn central_storage_wins_when_destination_is_not_empty() {
        let root = test_root("central-destination-wins");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let rom_path = root.join("game.nes");
        fs::write(&rom_path, [0_u8; 4]).unwrap();
        let sidecar = resolve_sidecars(&rom_path);
        fs::write(&sidecar.mapper_save_path, b"sidecar").unwrap();

        let central_root = root.join("central");
        let identity = test_system_identity();
        let central = resolve_central_storage_paths(&central_root, &DummySystemId, &identity);
        fs::create_dir_all(central.mapper_save_path.parent().unwrap()).unwrap();
        fs::write(&central.mapper_save_path, b"central").unwrap();

        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::CustomDirectory;
        shared.persistence.storage_directory = Some(central_root);

        let identity = test_system_identity();
        let resolved = resolve_persistence_paths_with_import(
            &shared,
            None,
            &DummySystemId,
            Some(&rom_path),
            &identity,
        )
        .unwrap();

        assert_eq!(fs::read(&resolved.mapper_save_path).unwrap(), b"central");
    }

    #[test]
    fn app_shared_data_imports_from_custom_storage_when_sidecar_is_empty() {
        let root = test_root("app-shared-imports-custom");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let rom_path = root.join("game.nes");
        fs::write(&rom_path, [0_u8; 4]).unwrap();
        let custom_root = root.join("custom");
        let identity = test_system_identity();
        let custom = resolve_central_storage_paths(&custom_root, &DummySystemId, &identity);
        fs::create_dir_all(custom.mapper_save_path.parent().unwrap()).unwrap();
        fs::write(&custom.mapper_save_path, b"custom").unwrap();

        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::AppSharedData;
        shared.persistence.storage_directory = Some(custom_root);
        let paths = SettingsPaths {
            settings_file: root.join("config/settings.yaml"),
            central_storage_root: root.join("data/persistence"),
        };

        let resolved = resolve_persistence_paths_with_import(
            &shared,
            Some(&paths),
            &DummySystemId,
            Some(&rom_path),
            &test_system_identity(),
        )
        .unwrap();

        assert_eq!(fs::read(&resolved.mapper_save_path).unwrap(), b"custom");
    }
}
