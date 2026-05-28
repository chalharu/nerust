use nerust_gui_runtime::rom_library::{RomLibrary, RomLibraryPaths};
use nerust_gui_runtime::settings::manager::SettingsManager;
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsPaths};
use nerust_gui_shell::settings::defaults::seed::{
    default_app_state, default_local_settings, default_shared_settings,
};
use std::fs;
use std::path::PathBuf;

const LAST_ROM_ID_FILE_NAME: &str = "last-rom-id";
const SETTINGS_ROOT_DIR_NAME: &str = "settings";
const ROM_LIBRARY_ROOT_DIR_NAME: &str = "rom-library";

pub(crate) struct AndroidStorage {
    pub(crate) settings: SettingsManager,
    pub(crate) rom_library: RomLibrary,
    last_rom_id_file: PathBuf,
}

impl AndroidStorage {
    pub(crate) fn open(root: impl Into<PathBuf>) -> Result<Self, String> {
        let root = root.into();
        let identity = HostBackendIdentity::android_wgpu();
        let settings = SettingsManager::load_or_ephemeral_with_paths(
            SettingsPaths::from_root(root.join(SETTINGS_ROOT_DIR_NAME), &identity),
            default_shared_settings(),
            default_local_settings(),
            default_app_state(),
        );
        let rom_library =
            RomLibrary::open(RomLibraryPaths::new(root.join(ROM_LIBRARY_ROOT_DIR_NAME)))
                .map_err(|error| format!("failed to open Android ROM library: {error}"))?;
        Ok(Self {
            settings,
            rom_library,
            last_rom_id_file: root.join(LAST_ROM_ID_FILE_NAME),
        })
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

    pub(crate) fn save_last_rom_id(&self, id: &str) -> Result<(), String> {
        if let Some(parent) = self.last_rom_id_file.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create Android storage root: {error}"))?;
        }
        fs::write(&self.last_rom_id_file, format!("{id}\n"))
            .map_err(|error| format!("failed to save Android last ROM id: {error}"))
    }
}
