use crc::{CRC_32_ISO_HDLC, Crc};
use serde_derive::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const ROM_LIBRARY_SCHEMA_VERSION: u32 = 1;
const CATALOG_FILE_NAME: &str = "catalog.yaml";
const ROMS_DIR_NAME: &str = "roms";

#[derive(Debug, thiserror::Error)]
pub enum RomLibraryError {
    #[error("ROM library schema version {found} is newer than supported version {expected}")]
    UnsupportedSchemaVersion { found: u32, expected: u32 },
    #[error("ROM library serialization failed: {0}")]
    Serialize(#[from] serde_yaml::Error),
    #[error("ROM library I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("system time is unavailable: {0}")]
    Clock(#[from] std::time::SystemTimeError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomLibraryPaths {
    pub root: PathBuf,
    pub catalog_file: PathBuf,
    pub roms_dir: PathBuf,
}

impl RomLibraryPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            catalog_file: root.join(CATALOG_FILE_NAME),
            roms_dir: root.join(ROMS_DIR_NAME),
            root,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RomLibraryEntry {
    pub id: String,
    pub display_name: String,
    pub file_name: String,
    pub imported_at_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct RomLibraryDocument {
    schema_version: u32,
    entries: Vec<RomLibraryEntry>,
}

impl Default for RomLibraryDocument {
    fn default() -> Self {
        Self {
            schema_version: ROM_LIBRARY_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct RomLibrary {
    paths: RomLibraryPaths,
    document: RomLibraryDocument,
}

impl RomLibrary {
    pub fn open(paths: RomLibraryPaths) -> Result<Self, RomLibraryError> {
        let document = match fs::read_to_string(&paths.catalog_file) {
            Ok(contents) => {
                let document: RomLibraryDocument = serde_yaml::from_str(&contents)?;
                if document.schema_version > ROM_LIBRARY_SCHEMA_VERSION {
                    return Err(RomLibraryError::UnsupportedSchemaVersion {
                        found: document.schema_version,
                        expected: ROM_LIBRARY_SCHEMA_VERSION,
                    });
                }
                RomLibraryDocument {
                    schema_version: ROM_LIBRARY_SCHEMA_VERSION,
                    ..document
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                RomLibraryDocument::default()
            }
            Err(error) => return Err(error.into()),
        };
        Ok(Self { paths, document })
    }

    pub fn entries(&self) -> &[RomLibraryEntry] {
        &self.document.entries
    }

    pub fn import_bytes(
        &mut self,
        display_name: &str,
        extension: &str,
        bytes: &[u8],
    ) -> Result<RomLibraryEntry, RomLibraryError> {
        let imported_at_unix_seconds = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let checksum = Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(bytes);
        let id = format!("{imported_at_unix_seconds:016x}-{checksum:08x}");
        let file_name = match normalize_extension(extension) {
            Some(extension) => format!("{id}.{extension}"),
            None => id.clone(),
        };
        fs::create_dir_all(&self.paths.roms_dir)?;
        fs::write(self.paths.roms_dir.join(&file_name), bytes)?;

        let entry = RomLibraryEntry {
            id: id.clone(),
            display_name: normalize_display_name(display_name),
            file_name,
            imported_at_unix_seconds,
        };
        self.document.entries.retain(|existing| existing.id != id);
        self.document.entries.insert(0, entry.clone());
        self.save()?;
        Ok(entry)
    }

    pub fn load_bytes(&self, id: &str) -> Result<Option<Vec<u8>>, RomLibraryError> {
        let Some(path) = self.rom_path(id) else {
            return Ok(None);
        };
        match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn rom_path(&self, id: &str) -> Option<PathBuf> {
        self.document
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| self.paths.roms_dir.join(&entry.file_name))
    }

    pub fn remove(&mut self, id: &str) -> Result<bool, RomLibraryError> {
        let Some(index) = self
            .document
            .entries
            .iter()
            .position(|entry| entry.id == id)
        else {
            return Ok(false);
        };
        let entry = self.document.entries.remove(index);
        match fs::remove_file(self.paths.roms_dir.join(&entry.file_name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.save()?;
        Ok(true)
    }

    fn save(&self) -> Result<(), RomLibraryError> {
        fs::create_dir_all(&self.paths.root)?;
        fs::create_dir_all(&self.paths.roms_dir)?;
        fs::write(
            &self.paths.catalog_file,
            serde_yaml::to_string(&self.document)?,
        )?;
        Ok(())
    }
}

fn normalize_display_name(display_name: &str) -> String {
    let trimmed = display_name.trim();
    if trimmed.is_empty() {
        "Imported ROM".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_extension(extension: &str) -> Option<String> {
    let trimmed = extension.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        return None;
    }
    let normalized: String = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect();
    (!normalized.is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::{RomLibrary, RomLibraryPaths};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("nerust-rom-library-{label}-{nonce}"))
    }

    #[test]
    fn import_round_trips_catalog_and_bytes() {
        let root = test_root("import");
        let paths = RomLibraryPaths::new(root.clone());
        let mut library = RomLibrary::open(paths.clone()).unwrap();

        let entry = library
            .import_bytes("Mega Man", ".NES", b"rom-bytes")
            .unwrap();

        assert_eq!(library.entries(), std::slice::from_ref(&entry));
        assert!(entry.file_name.ends_with(".nes"));
        assert_eq!(
            fs::read(paths.roms_dir.join(&entry.file_name)).unwrap(),
            b"rom-bytes"
        );

        let reopened = RomLibrary::open(paths).unwrap();
        assert_eq!(reopened.entries(), std::slice::from_ref(&entry));
        assert_eq!(
            reopened.load_bytes(&entry.id).unwrap(),
            Some(b"rom-bytes".to_vec())
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_bytes_returns_none_for_unknown_id() {
        let root = test_root("unknown-id");
        let paths = RomLibraryPaths::new(root.clone());
        let library = RomLibrary::open(paths).unwrap();
        assert_eq!(library.load_bytes("does-not-exist").unwrap(), None);
        assert_eq!(library.rom_path("does-not-exist"), None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn most_recently_imported_entry_is_first() {
        let root = test_root("import-order");
        let paths = RomLibraryPaths::new(root.clone());
        let mut library = RomLibrary::open(paths.clone()).unwrap();

        let first = library
            .import_bytes("First Game", "nes", b"first-rom")
            .unwrap();
        let second = library
            .import_bytes("Second Game", "nes", b"second-rom")
            .unwrap();

        let entries = library.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, second.id);
        assert_eq!(entries[1].id, first.id);

        // Persisted order must match in-memory order.
        let reopened = RomLibrary::open(paths).unwrap();
        assert_eq!(reopened.entries()[0].id, second.id);
        assert_eq!(reopened.entries()[1].id, first.id);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_returns_false_for_unknown_id() {
        let root = test_root("remove-unknown");
        let paths = RomLibraryPaths::new(root.clone());
        let mut library = RomLibrary::open(paths).unwrap();
        assert!(!library.remove("does-not-exist").unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remove_deletes_catalog_entry_and_payload() {
        let root = test_root("remove");
        let paths = RomLibraryPaths::new(root.clone());
        let mut library = RomLibrary::open(paths.clone()).unwrap();
        let entry = library.import_bytes("", "zip", b"zip-bytes").unwrap();

        assert!(library.remove(&entry.id).unwrap());
        assert!(!paths.roms_dir.join(&entry.file_name).exists());
        assert!(library.entries().is_empty());
        assert_eq!(library.load_bytes(&entry.id).unwrap(), None);

        let reopened = RomLibrary::open(paths).unwrap();
        assert!(reopened.entries().is_empty());

        let _ = fs::remove_dir_all(root);
    }
}
