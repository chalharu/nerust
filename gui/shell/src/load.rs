use std::path::Path;

use nerust_core_traits::factory::load::{MediaObject, ResolvedLoadRequest, SystemLoadOptions};
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
    fn default_load_options(&self) -> SystemLoadOptions;
    fn settings_snapshot(&self) -> &SettingsSnapshot;
    fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError>;
    fn resume(&mut self);
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadRequest {
    Auto,
    Explicit { options: SystemLoadOptions },
}

#[cfg(test)]
#[path = "tests/load.rs"]
mod tests;
