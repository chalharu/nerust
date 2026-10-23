use std::{fs, path::Path};

use crc::{CRC_32_ISO_HDLC, Crc};
use nerust_core_traits::identity::{SystemId, SystemIdentity};
use nerust_persistence::sidecar::{SidecarPaths, StorageLocation, resolve_sidecars};
use nerust_settings_core::model::shared::{DesktopSharedSettings, StoragePolicy};

use super::{SettingsError, SettingsPaths};

const MAPPER_SAVE_FILE_NAME: &str = "mapper.sav";
const STATES_DIR_NAME: &str = "states";

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
        mapper_save_path: base.join(MAPPER_SAVE_FILE_NAME).into(),
        states_dir: base.join(STATES_DIR_NAME).into(),
    }
}

pub fn resolve_document_tree_storage_paths(
    uri: &str,
    system: &dyn SystemId,
    identity: &SystemIdentity,
) -> Result<SidecarPaths, SettingsError> {
    let base = format!("{system}/{}", system_storage_key(system, identity));
    Ok(SidecarPaths {
        mapper_save_path: StorageLocation::document_tree(
            uri,
            format!("{base}/{MAPPER_SAVE_FILE_NAME}"),
        )
        .map_err(storage_error)?,
        states_dir: StorageLocation::document_tree(uri, format!("{base}/{STATES_DIR_NAME}"))
            .map_err(storage_error)?,
    })
}

pub fn resolve_document_tree_sidecar_paths(
    uri: &str,
    system: &dyn SystemId,
    identity: &SystemIdentity,
) -> Result<SidecarPaths, SettingsError> {
    let key = format!("{}-{}", system, system_storage_key(system, identity));
    Ok(SidecarPaths {
        mapper_save_path: StorageLocation::document_tree(uri, format!("{key}.sav"))
            .map_err(storage_error)?,
        states_dir: StorageLocation::document_tree(uri, format!("{key}.states"))
            .map_err(storage_error)?,
    })
}

fn storage_error(error: nerust_persistence::error::PersistenceError) -> SettingsError {
    SettingsError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
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
                return resolve_document_tree_sidecar_paths(uri, system, identity);
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
                return resolve_document_tree_storage_paths(uri, system, identity);
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
    if matches!(
        destination.mapper_save_path,
        StorageLocation::DocumentTree { .. }
    ) {
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
    let mapper_exists = paths
        .mapper_save_path
        .native_path()
        .map_err(storage_error)?
        .is_file();
    let states_exist = match fs::read_dir(paths.states_dir.native_path().map_err(storage_error)?) {
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
    let source_mapper = source
        .mapper_save_path
        .native_path()
        .map_err(storage_error)?;
    let destination_mapper = destination
        .mapper_save_path
        .native_path()
        .map_err(storage_error)?;
    let source_states = source.states_dir.native_path().map_err(storage_error)?;
    let destination_states = destination
        .states_dir
        .native_path()
        .map_err(storage_error)?;
    // Roll back a partial migration so the next load retries from a clean
    // state instead of freezing on a half-migrated target (NER-325).
    let mut copied: Vec<std::path::PathBuf> = Vec::new();
    let result = (|| -> Result<(), SettingsError> {
        if source_mapper.is_file() {
            if let Some(parent) = destination_mapper.parent() {
                fs::create_dir_all(parent)?;
            }
            copy_one_tracked(
                &mut copied,
                source_mapper,
                destination_mapper,
                |src, dst| fs::copy(src, dst),
            )?;
        }
        match fs::read_dir(source_states) {
            Ok(entries) => {
                fs::create_dir_all(destination_states)?;
                for entry in entries {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_file() {
                        let target = destination_states.join(entry.file_name());
                        copy_one_tracked(&mut copied, &path, &target, |src, dst| {
                            fs::copy(src, dst)
                        })?;
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    })();
    if result.is_err() {
        for path in copied {
            // A failed cleanup must not mask the migration error, but it
            // must not pass silently either: a leftover target would still
            // block the retry. Already-swept targets are quiet.
            if let Err(error) = fs::remove_file(&path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!(
                    "storage migration rollback could not remove {}: {error}",
                    path.display()
                );
            }
        }
    }
    result
}

/// Copy one migration file with rollback reservation: the target joins the
/// rollback set BEFORE the write, so a copy that fails mid-stream still has
/// its partial target removed (NER-325).
fn copy_one_tracked(
    copied: &mut Vec<std::path::PathBuf>,
    source: &Path,
    target: &Path,
    copy: impl FnOnce(&Path, &Path) -> std::io::Result<u64>,
) -> Result<(), SettingsError> {
    copied.push(target.to_path_buf());
    if let Err(error) = copy(source, target) {
        // Remove the partial target now instead of relying on the outer
        // sweep: the reservation must hold even if the caller never
        // reaches it. The sweep stays as the backstop for earlier files.
        if let Err(cleanup) = fs::remove_file(target) {
            log::warn!(
                "storage migration rollback could not remove {}: {cleanup}",
                target.display()
            );
        }
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use nerust_persistence::sidecar::{StorageLocation, resolve_sidecars};
    use nerust_settings_core::model::shared::StoragePolicy;

    use crate::test::DummySystemId;

    use super::{
        super::{SettingsPaths, test_root, test_shared_defaults, test_system_identity},
        resolve_central_storage_paths, resolve_document_tree_sidecar_paths,
        resolve_document_tree_storage_paths, resolve_persistence_paths_with_import,
        system_storage_key,
    };

    #[test]
    fn central_storage_paths_use_system_and_identity_not_rom_path() {
        let root = PathBuf::from("/base");
        let identity = test_system_identity();
        let first = resolve_central_storage_paths(&root, &DummySystemId, &identity);
        let second = resolve_central_storage_paths(&root, &DummySystemId, &identity);

        assert_eq!(first, second);
        assert!(
            first
                .mapper_save_path
                .native_path()
                .unwrap()
                .ends_with("mapper.sav")
        );
        assert!(first.states_dir.native_path().unwrap().ends_with("states"));
        assert!(!system_storage_key(&DummySystemId, &identity).is_empty());
    }

    /// NER-325: the rollback reservation happens before the write, so a
    /// copy that fails mid-stream still has its partial target removed and
    /// the next load can retry from a clean state.
    #[test]
    fn tracked_copy_removes_partial_target_on_mid_stream_failure() {
        use super::copy_one_tracked;
        let dir = super::super::test_root("tracked-copy-partial");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("7.state");
        let mut copied = Vec::new();
        let error = copy_one_tracked(
            &mut copied,
            std::path::Path::new("source"),
            &target,
            |_, dst| {
                // Simulate a mid-stream failure that leaves a partial target.
                fs::write(dst, vec![0u8; 1024]).unwrap();
                Err(std::io::Error::other("mid-stream failure"))
            },
        )
        .unwrap_err();
        assert!(matches!(error, super::super::SettingsError::Io(_)));
        assert_eq!(copied, vec![target.clone()]);
        assert!(!target.exists(), "partial target must not block retry");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn document_tree_paths_round_trip_uri_and_relative_name() {
        let identity = test_system_identity();
        let paths = resolve_document_tree_storage_paths(
            "content://provider/tree/primary%3ASaves",
            identity.system_id.as_ref(),
            &identity,
        )
        .unwrap();

        let StorageLocation::DocumentTree { uri, relative } = paths.mapper_save_path else {
            panic!("expected document-tree storage");
        };
        assert_eq!(uri.as_str(), "content://provider/tree/primary%3ASaves");
        assert!(
            relative
                .as_str()
                .starts_with(&format!("{}/", identity.system_id))
        );
        assert!(relative.as_str().ends_with("/mapper.sav"));
    }

    #[test]
    fn document_tree_sidecars_are_siblings_in_selected_tree() {
        let identity = test_system_identity();
        let paths = resolve_document_tree_sidecar_paths(
            "content://provider/tree/primary%3ASaves",
            identity.system_id.as_ref(),
            &identity,
        )
        .unwrap();
        let StorageLocation::DocumentTree {
            relative: mapper, ..
        } = paths.mapper_save_path
        else {
            panic!("expected document-tree mapper");
        };
        let StorageLocation::DocumentTree {
            relative: states, ..
        } = paths.states_dir
        else {
            panic!("expected document-tree states");
        };
        let (mapper, states) = (mapper.as_str(), states.as_str());

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
        fs::write(sidecar.mapper_save_path.native_path().unwrap(), b"mapper").unwrap();
        fs::create_dir_all(sidecar.states_dir.native_path().unwrap()).unwrap();
        fs::write(
            sidecar.states_dir.native_path().unwrap().join("slot-1.zip"),
            b"state",
        )
        .unwrap();

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

        assert_eq!(
            fs::read(resolved.mapper_save_path.native_path().unwrap()).unwrap(),
            b"mapper"
        );
        assert_eq!(
            fs::read(
                resolved
                    .states_dir
                    .native_path()
                    .unwrap()
                    .join("slot-1.zip")
            )
            .unwrap(),
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
        fs::write(sidecar.mapper_save_path.native_path().unwrap(), b"sidecar").unwrap();

        let central_root = root.join("central");
        let identity = test_system_identity();
        let central = resolve_central_storage_paths(&central_root, &DummySystemId, &identity);
        fs::create_dir_all(
            central
                .mapper_save_path
                .native_path()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .unwrap();
        fs::write(central.mapper_save_path.native_path().unwrap(), b"central").unwrap();

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

        assert_eq!(
            fs::read(resolved.mapper_save_path.native_path().unwrap()).unwrap(),
            b"central"
        );
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
        fs::create_dir_all(
            custom
                .mapper_save_path
                .native_path()
                .unwrap()
                .parent()
                .unwrap(),
        )
        .unwrap();
        fs::write(custom.mapper_save_path.native_path().unwrap(), b"custom").unwrap();

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

        assert_eq!(
            fs::read(resolved.mapper_save_path.native_path().unwrap()).unwrap(),
            b"custom"
        );
    }

    #[test]
    fn document_tree_policy_resolves_without_native_import_or_fallback() {
        let identity = test_system_identity();
        let uri = "content://provider/tree/primary%3ASaves";
        for policy in [StoragePolicy::Sidecar, StoragePolicy::CustomDirectory] {
            let mut shared = test_shared_defaults();
            shared.persistence.storage_policy = policy;
            shared.persistence.storage_document_tree_uri = Some(uri.into());
            // A native path is neither required nor consulted for a selected tree.
            let resolved = resolve_persistence_paths_with_import(
                &shared,
                None,
                &DummySystemId,
                None,
                &identity,
            )
            .unwrap();
            let expected = if policy == StoragePolicy::Sidecar {
                resolve_document_tree_sidecar_paths(uri, &DummySystemId, &identity)
            } else {
                resolve_document_tree_storage_paths(uri, &DummySystemId, &identity)
            }
            .unwrap();
            assert_eq!(resolved, expected);
            assert!(resolved.mapper_save_path.native_path().is_err());
            assert!(resolved.states_dir.native_path().is_err());
            shared.persistence.storage_document_tree_uri = Some(String::new());
            assert!(
                resolve_persistence_paths_with_import(
                    &shared,
                    None,
                    &DummySystemId,
                    None,
                    &identity,
                )
                .is_err()
            );
        }
    }

    #[test]
    fn failed_migration_rolls_back_and_retries_cleanly() {
        use super::{copy_storage_contents, resolve_central_storage_paths};
        use crate::settings::test_root;

        let identity = crate::settings::test_system_identity();
        let source =
            resolve_central_storage_paths(&test_root("migrate-src"), &DummySystemId, &identity);
        let destination =
            resolve_central_storage_paths(&test_root("migrate-dst"), &DummySystemId, &identity);
        let source_mapper = source.mapper_save_path.native_path().unwrap().to_path_buf();
        let source_state = source.states_dir.native_path().unwrap().join("7.state");
        fs::create_dir_all(source_mapper.parent().unwrap()).unwrap();
        fs::create_dir_all(source_state.parent().unwrap()).unwrap();
        fs::write(&source_mapper, b"mapper").unwrap();
        fs::write(&source_state, b"state").unwrap();
        let destination_mapper = destination
            .mapper_save_path
            .native_path()
            .unwrap()
            .to_path_buf();
        let destination_states = destination.states_dir.native_path().unwrap().to_path_buf();
        // Block the states copy: the mapper copy must roll back.
        fs::create_dir_all(destination_mapper.parent().unwrap()).unwrap();
        fs::write(&destination_states, b"blocker").unwrap();
        assert!(copy_storage_contents(&source, &destination).is_err());
        assert!(
            !destination_mapper.exists(),
            "partial migration must roll back"
        );
        // Unblocked retry migrates everything.
        fs::remove_file(&destination_states).unwrap();
        copy_storage_contents(&source, &destination).unwrap();
        assert_eq!(fs::read(&destination_mapper).unwrap(), b"mapper");
        assert_eq!(
            fs::read(destination_states.join("7.state")).unwrap(),
            b"state"
        );
        let _ = fs::remove_dir_all(test_root("migrate-src"));
        let _ = fs::remove_dir_all(test_root("migrate-dst"));
    }
}
