use nerust_gui_runtime::rom_library::{RomLibrary, RomLibraryPaths};
use nerust_gui_runtime::settings::{HostBackendIdentity, SettingsManager, SettingsPaths};
use nerust_gui_shell::settings::defaults::seed::{
    default_app_state, default_local_settings, default_shared_settings,
};
use std::path::PathBuf;

const SETTINGS_ROOT_DIR_NAME: &str = "settings";
const ROM_LIBRARY_ROOT_DIR_NAME: &str = "rom-library";

pub(crate) struct AndroidStorage {
    pub(crate) settings: SettingsManager,
    pub(crate) rom_library: RomLibrary,
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
        })
    }
}
