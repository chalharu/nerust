use std::{fs, path::PathBuf};

use nerust_gui_runtime::rom_library::{RomLibrary, RomLibraryPaths};

const LAST_ROM_ID_FILE_NAME: &str = "last-rom-id";
const LAST_MEDIA_REFERENCE_FILE_NAME: &str = "last-media.json";
const LAST_MEDIA_REFERENCE_SCHEMA_VERSION: u32 = 1;
const RESTORE_PENDING_FILE_NAME: &str = ".restore_pending";
const ROM_LIBRARY_ROOT_DIR_NAME: &str = "rom-library";
const STORAGE_POLICY_MIGRATION_FILE_NAME: &str = ".storage-policy-v1";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LastMediaReference {
    schema_version: u32,
    pub(crate) uri: String,
    pub(crate) display_name: String,
    pub(crate) system_id: String,
}

impl LastMediaReference {
    pub(crate) fn new(uri: String, display_name: String, system_id: String) -> Self {
        Self {
            schema_version: LAST_MEDIA_REFERENCE_SCHEMA_VERSION,
            uri,
            display_name,
            system_id,
        }
    }
}

pub(crate) struct AndroidStorage {
    pub(crate) rom_library: RomLibrary,
    last_rom_id_file: PathBuf,
    last_media_reference_file: PathBuf,
    restore_pending_file: PathBuf,
    storage_policy_migration_file: PathBuf,
}

impl AndroidStorage {
    pub(crate) fn open(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        let rom_library =
            RomLibrary::open(RomLibraryPaths::new(root.join(ROM_LIBRARY_ROOT_DIR_NAME)))
                .map_err(|error| format!("failed to open Android ROM library: {error}"))?;
        Ok(Self {
            rom_library,
            last_rom_id_file: root.join(LAST_ROM_ID_FILE_NAME),
            last_media_reference_file: root.join(LAST_MEDIA_REFERENCE_FILE_NAME),
            restore_pending_file: root.join(RESTORE_PENDING_FILE_NAME),
            storage_policy_migration_file: root.join(STORAGE_POLICY_MIGRATION_FILE_NAME),
        })
    }

    pub(crate) fn load_last_media_reference(&self) -> Result<Option<LastMediaReference>, String> {
        let bytes = match fs::read(&self.last_media_reference_file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("failed to read Android last media: {error}")),
        };
        let reference: LastMediaReference = serde_json::from_slice(&bytes)
            .map_err(|error| format!("failed to parse Android last media: {error}"))?;
        if reference.schema_version != LAST_MEDIA_REFERENCE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported Android last media schema version {}",
                reference.schema_version
            ));
        }
        Ok(Some(reference))
    }

    pub(crate) fn save_last_media_reference(
        &self,
        reference: &LastMediaReference,
    ) -> Result<(), String> {
        let parent = self
            .last_media_reference_file
            .parent()
            .ok_or_else(|| "Android last media path has no parent".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create Android storage root: {error}"))?;
        let temporary = self.last_media_reference_file.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(reference)
            .map_err(|error| format!("failed to serialize Android last media: {error}"))?;
        fs::write(&temporary, bytes)
            .map_err(|error| format!("failed to stage Android last media: {error}"))?;
        fs::rename(&temporary, &self.last_media_reference_file)
            .map_err(|error| format!("failed to commit Android last media: {error}"))
    }

    pub(crate) fn storage_policy_migration_completed(&self) -> bool {
        self.storage_policy_migration_file.is_file()
    }

    pub(crate) fn complete_storage_policy_migration(&self) -> Result<(), String> {
        if let Some(parent) = self.storage_policy_migration_file.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create Android storage root: {error}"))?;
        }
        fs::write(&self.storage_policy_migration_file, b"app_shared_data\n")
            .map_err(|error| format!("failed to save Android storage migration marker: {error}"))
    }

    pub(crate) fn has_restore_pending(&self) -> bool {
        self.restore_pending_file.exists()
    }

    pub(crate) fn touch_restore_pending(&self) {
        if let Some(parent) = self.restore_pending_file.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            log::warn!("failed to create restore pending dir: {error}");
        }
        if let Err(error) = fs::write(&self.restore_pending_file, []) {
            log::warn!("failed to write restore pending file: {error}");
        }
    }

    pub(crate) fn clear_restore_pending(&self) {
        if let Err(error) = fs::remove_file(&self.restore_pending_file)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("failed to clear restore pending file: {error}");
        }
    }

    pub(crate) fn load_last_rom_id(&self) -> Result<Option<String>, String> {
        match fs::read_to_string(&self.last_rom_id_file) {
            Ok(contents) => {
                let id = contents.trim();
                Ok((!id.is_empty()).then(|| id.to_string()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("failed to read Android last ROM id: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temporary_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!("nerust-android-{name}-{unique}"))
    }

    #[test]
    fn last_media_reference_round_trips_without_rom_bytes() {
        let root = temporary_root("last-media");
        let storage = AndroidStorage::open(&root).unwrap();
        let reference = LastMediaReference::new(
            "content://provider/document/42".to_string(),
            "Pokemon Crystal.gbc".to_string(),
            "gbc".to_string(),
        );

        storage.save_last_media_reference(&reference).unwrap();

        assert_eq!(
            storage.load_last_media_reference().unwrap(),
            Some(reference)
        );
        let serialized = fs::read_to_string(root.join(LAST_MEDIA_REFERENCE_FILE_NAME)).unwrap();
        assert!(!serialized.contains("romBytes"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn storage_policy_marker_is_explicit() {
        let root = temporary_root("storage-policy");
        let storage = AndroidStorage::open(&root).unwrap();
        assert!(!storage.storage_policy_migration_completed());

        storage.complete_storage_policy_migration().unwrap();

        assert!(storage.storage_policy_migration_completed());
        fs::remove_dir_all(root).unwrap();
    }
}
