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

/// A resolved storage key. Document URIs never masquerade as filesystem paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageLocation {
    Native(PathBuf),
    DocumentTree {
        uri: String,
        relative: DocumentRelativePath,
    },
}

/// Validated, slash-separated path inside a document tree (not an OS path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRelativePath(String);

impl DocumentRelativePath {
    pub fn new(relative: impl Into<String>) -> Result<Self, PersistenceError> {
        let relative = relative.into();
        if relative.split('/').any(|segment| {
            segment.is_empty()
                || segment == "."
                || segment == ".."
                || segment.contains(['\\', '\0', ':'])
        }) {
            return Err(PersistenceError::Validation(
                "invalid document-tree relative path".into(),
            ));
        }
        Ok(Self(relative))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl StorageLocation {
    pub fn document_tree(uri: &str, relative: impl Into<String>) -> Result<Self, PersistenceError> {
        let authority = uri
            .strip_prefix("content://")
            .and_then(|rest| rest.split('/').next());
        if authority.is_none_or(str::is_empty)
            || uri.contains('\0')
            || uri.chars().any(char::is_whitespace)
        {
            return Err(PersistenceError::Validation(
                "invalid document-tree URI".into(),
            ));
        }
        Ok(Self::DocumentTree {
            uri: uri.to_owned(),
            relative: DocumentRelativePath::new(relative)?,
        })
    }

    /// Explicit native boundary: unsupported storage fails before any filesystem I/O.
    pub fn native_path(&self) -> Result<&Path, PersistenceError> {
        match self {
            Self::Native(path) => Ok(path),
            Self::DocumentTree { .. } => Err(PersistenceError::Validation(
                "document-tree storage requires a document backend".into(),
            )),
        }
    }

    /// Append a single untrusted entry name, rejecting traversal on either backend.
    pub fn child(&self, name: &str) -> Result<Self, PersistenceError> {
        DocumentRelativePath::new(name)?;
        if name.contains('/') {
            return Err(PersistenceError::Validation(
                "invalid storage entry name".into(),
            ));
        }
        match self {
            Self::Native(path) => Ok(Self::Native(path.join(name))),
            Self::DocumentTree { uri, relative } => {
                Self::document_tree(uri, format!("{}/{name}", relative.as_str()))
            }
        }
    }
}

impl From<PathBuf> for StorageLocation {
    fn from(path: PathBuf) -> Self {
        Self::Native(path)
    }
}

impl From<&Path> for StorageLocation {
    fn from(path: &Path) -> Self {
        Self::Native(path.to_path_buf())
    }
}

impl From<&str> for StorageLocation {
    fn from(path: &str) -> Self {
        Self::Native(path.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarPaths {
    pub mapper_save_path: StorageLocation,
    pub states_dir: StorageLocation,
}

pub fn resolve_sidecars(rom_path: &Path) -> SidecarPaths {
    let rom_name = rom_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("game");
    let base_dir = rom_path.parent().unwrap_or_else(|| Path::new("."));
    SidecarPaths {
        mapper_save_path: base_dir.join(format!("{rom_name}.sav")).into(),
        states_dir: base_dir.join(format!("{rom_name}.states")).into(),
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
