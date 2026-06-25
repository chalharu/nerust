use std::{
    fs,
    path::{Path, PathBuf},
};

use super::prepare_test_dir;
use crate::sidecar::{
    load_mapper_save, resolve_sidecars, write_mapper_save, write_recovery_mapper_save,
};

#[test]
fn resolve_sidecars_appends_to_full_rom_filename() {
    let nes = resolve_sidecars(Path::new("/tmp/game.nes"));
    let fds = resolve_sidecars(Path::new("/tmp/game.fds"));

    assert_eq!(nes.mapper_save_path, PathBuf::from("/tmp/game.nes.sav"));
    assert_eq!(nes.states_dir, PathBuf::from("/tmp/game.nes.states"));
    assert_eq!(fds.mapper_save_path, PathBuf::from("/tmp/game.fds.sav"));
    assert_eq!(fds.states_dir, PathBuf::from("/tmp/game.fds.states"));
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
