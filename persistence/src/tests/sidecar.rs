use std::{
    fs,
    path::{Path, PathBuf},
};

use super::prepare_test_dir;
use crate::sidecar::{
    DocumentRelativePath, StorageLocation, load_mapper_save, resolve_sidecars, write_mapper_save,
    write_recovery_mapper_save,
};

#[test]
fn resolve_sidecars_appends_to_full_rom_filename() {
    let nes = resolve_sidecars(Path::new("/tmp/game.nes"));
    let fds = resolve_sidecars(Path::new("/tmp/game.fds"));

    assert_eq!(
        nes.mapper_save_path,
        PathBuf::from("/tmp/game.nes.sav").into()
    );
    assert_eq!(nes.states_dir, PathBuf::from("/tmp/game.nes.states").into());
    assert_eq!(
        fds.mapper_save_path,
        PathBuf::from("/tmp/game.fds.sav").into()
    );
    assert_eq!(fds.states_dir, PathBuf::from("/tmp/game.fds.states").into());
    assert_ne!(nes.mapper_save_path, fds.mapper_save_path);
    assert_ne!(nes.states_dir, fds.states_dir);
}

#[test]
fn mapper_save_sidecar_and_recovery_paths_preserve_bytes() {
    let dir = prepare_test_dir("mapper-save-sidecars");
    let sidecar = dir.join("game.nes.sav");

    write_mapper_save(&sidecar, b"primary").unwrap();
    assert_eq!(
        load_mapper_save(&sidecar).unwrap(),
        Some(b"primary".to_vec())
    );

    let recovery = write_recovery_mapper_save(&sidecar, b"recovered").unwrap();
    assert_ne!(recovery, sidecar);
    assert_eq!(
        load_mapper_save(&sidecar).unwrap(),
        Some(b"primary".to_vec())
    );
    assert_eq!(fs::read(recovery).unwrap(), b"recovered");
}

#[test]
fn storage_locations_keep_native_and_document_keys_distinct() {
    let native = StorageLocation::from(Path::new("/tmp/saves"));
    assert_eq!(
        native.child("7.state").unwrap().native_path().unwrap(),
        Path::new("/tmp/saves/7.state")
    );
    let uri = "content://provider/tree/primary%3A日本語";
    let tree = StorageLocation::document_tree(uri, "nes/id/states").unwrap();
    assert!(tree.native_path().is_err());
    let child = tree.child("7.state").unwrap();
    assert_eq!(
        child,
        StorageLocation::DocumentTree {
            uri: uri.into(),
            relative: DocumentRelativePath::new("nes/id/states/7.state").unwrap(),
        }
    );
}

#[test]
fn document_relative_paths_and_child_names_reject_traversal() {
    for invalid in [
        "",
        "/root",
        "../save",
        "nes/../save",
        "nes//save",
        "nes/./save",
        "nes/",
        "nes\\save",
        "C:/save",
        "save\0file",
    ] {
        assert!(
            StorageLocation::document_tree("content://provider/tree/saves", invalid).is_err(),
            "{invalid:?}"
        );
    }
    assert!(StorageLocation::document_tree("", "save").is_err());
    for dir in [
        StorageLocation::from("/tmp/saves"),
        StorageLocation::document_tree("content://provider/tree/saves", "states").unwrap(),
    ] {
        for invalid in [
            "",
            "..",
            ".",
            "../7.state",
            "/7.state",
            "sub/7.state",
            "sub\\7.state",
            "7.state\0",
        ] {
            assert!(dir.child(invalid).is_err(), "{invalid:?}");
        }
    }
}
