use crate::error::PersistenceError;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), PersistenceError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temp_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sidecar");
    let (mut file, temp_path) = create_temp_file(parent, temp_name)?;
    let write_result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error.into());
    }
    if let Err(error) = replace_path(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    sync_parent_dir(path)?;
    Ok(())
}

pub(crate) fn read_limited(
    reader: &mut impl Read,
    max_len: usize,
    label: &str,
) -> Result<Vec<u8>, PersistenceError> {
    let mut bytes = Vec::new();
    reader.take(max_len as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max_len {
        return Err(PersistenceError::Validation(format!(
            "{label} entry exceeds {max_len} bytes"
        )));
    }
    Ok(bytes)
}

pub(crate) fn sync_parent_dir(path: &Path) -> Result<(), PersistenceError> {
    sync_parent_dir_impl(path)
}

pub(crate) fn temp_nonce() -> String {
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    format!(
        "{}.{}.{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed),
    )
}

fn create_temp_file(
    parent: &Path,
    base_name: &str,
) -> Result<(std::fs::File, PathBuf), PersistenceError> {
    create_unique_file(
        |nonce| parent.join(format!(".{base_name}.{}.tmp", nonce)),
        "failed to create unique temporary sidecar",
    )
}

pub(crate) fn create_unique_file(
    mut path_for_nonce: impl FnMut(&str) -> PathBuf,
    failure_message: &str,
) -> Result<(std::fs::File, PathBuf), PersistenceError> {
    for _ in 0..1024 {
        let nonce = temp_nonce();
        let temp_path = path_for_nonce(&nonce);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((file, temp_path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(PersistenceError::Validation(failure_message.into()))
}

#[cfg(not(windows))]
fn replace_path(from: &Path, to: &Path) -> Result<(), PersistenceError> {
    fs::rename(from, to)?;
    Ok(())
}

#[cfg(windows)]
fn replace_path(from: &Path, to: &Path) -> Result<(), PersistenceError> {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(iter::once(0)).collect();
    let from_wide: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let replaced = unsafe {
        ReplaceFileW(
            to_wide.as_ptr(),
            from_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced != 0 {
        Ok(())
    } else {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::NotFound {
            fs::rename(from, to)?;
            Ok(())
        } else {
            Err(error.into())
        }
    }
}

#[cfg(not(windows))]
fn sync_parent_dir_impl(path: &Path) -> Result<(), PersistenceError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(windows)]
fn sync_parent_dir_impl(_path: &Path) -> Result<(), PersistenceError> {
    Ok(())
}
