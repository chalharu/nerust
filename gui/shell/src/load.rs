use std::path::Path;

use nerust_core_traits::{
    factory::load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
    identity::SystemId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;

#[derive(Debug, thiserror::Error)]
pub enum RomLoaderError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("load request resolution failed: {0}")]
    Resolve(String),
    #[error("ROM load failed: {0}")]
    Load(String),
}

/// Target for a ROM load operation.
///
/// Abstracts the session operations needed by `RomLoader` implementations,
/// allowing them to work with any type (not just `SessionHandle`).
pub trait RomLoadTarget {
    fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>>;
    fn settings_snapshot(&self) -> &SettingsSnapshot;
    fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError>;
    fn resume(&mut self);

    /// Notifies the target of the detected system after a successful load.
    /// Default implementation is a no-op for backward compatibility.
    fn set_active_system(&mut self, _system_id: SystemId) {}
}

/// Loads and resolves a ROM file into a [`RomLoadTarget`].
///
/// Implementations handle:
/// - Reading the file from disk
/// - Creating a `MediaObject` from the file contents
/// - Resolving system-specific load options (e.g., MMC3 IRQ variant)
/// - Calling `target.load_resolved()` to start emulation
/// - Calling `SessionCommand::Resume` after successful load
pub trait RomLoader {
    fn load_rom(
        &mut self,
        path: &Path,
        target: &mut dyn RomLoadTarget,
    ) -> Result<(), RomLoaderError>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoadRequest {
    Auto,
    Explicit {
        options: Box<dyn DynSystemLoadOptions>,
    },
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use nerust_core_traits::factory::load::MediaObject;

    #[test]
    fn media_object_tracks_path_extension() {
        let media = MediaObject::new(Some(PathBuf::from("/tmp/test.NES")), vec![1, 2, 3]);

        assert_eq!(media.extension.as_deref(), Some("nes"));
        assert_eq!(media.bytes.as_ref(), [1, 2, 3]);
    }
}
