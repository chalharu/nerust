use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use crate::{
    error::PersistenceError,
    fs_ops::{create_unique_file, read_limited, sync_parent_dir, write_atomic},
};

const MAX_MAPPER_SAVE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarPaths {
    pub mapper_save_path: PathBuf,
    pub states_dir: PathBuf,
}

pub fn resolve_sidecars(rom_path: &Path) -> SidecarPaths {
    let rom_name = rom_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("game");
    let base_dir = rom_path.parent().unwrap_or_else(|| Path::new("."));
    SidecarPaths {
        mapper_save_path: base_dir.join(format!("{rom_name}.sav")),
        states_dir: base_dir.join(format!("{rom_name}.states")),
    }
}

pub fn load_mapper_save(path: &Path) -> Result<Option<Vec<u8>>, PersistenceError> {
    match File::open(path) {
        Ok(mut file) => Ok(Some(read_limited(
            &mut file,
            MAX_MAPPER_SAVE_BYTES,
            "mapper save",
        )?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn write_mapper_save(path: &Path, bytes: &[u8]) -> Result<(), PersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    write_atomic(path, bytes)
}

pub fn write_recovery_mapper_save(
    original_path: &Path,
    bytes: &[u8],
) -> Result<PathBuf, PersistenceError> {
    let parent = original_path.parent().unwrap_or_else(|| Path::new("."));
    let base_name = original_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sidecar");
    let (mut file, recovery_path) = create_unique_file(
        |nonce| parent.join(format!("{base_name}.recovered.{nonce}")),
        "failed to create unique mapper save recovery path",
    )?;
    let write_result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&recovery_path);
        return Err(error.into());
    }
    sync_parent_dir(&recovery_path)?;
    Ok(recovery_path)
}
