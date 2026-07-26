use std::{path::Path, rc::Rc, sync::Arc};

use nerust_gui_settings::snapshot::SettingsSnapshot;
use nerust_settings_core::editor::CaptureTarget;

use super::{ValidationState, catalog::FactoryCatalog};

/// Storage path validation error.
#[derive(Debug, Clone)]
pub enum StoragePathError {
    NotDirectory,
    Inaccessible(String),
}

impl std::fmt::Display for StoragePathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoragePathError::NotDirectory => write!(f, "not a directory"),
            StoragePathError::Inaccessible(msg) => write!(f, "{msg}"),
        }
    }
}

/// Storage path validation port — injected by the composition root.
pub trait StoragePathValidator: std::fmt::Debug {
    fn validate(&self, path: &Path) -> Result<(), StoragePathError>;
}

/// Error type for view model operations.
#[derive(Debug, thiserror::Error)]
pub enum ViewModelError {
    #[error("settings mutation is not allowed during property notification")]
    ReentrantMutation,
    #[error("unknown system: {0}")]
    UnknownSystem(String),
    #[error("unknown controller slot: {0}")]
    UnknownSlot(String),
    #[error("unknown controller profile: {0}")]
    UnknownController(String),
    #[error("invalid system settings choice")]
    InvalidSystemChoice,
    #[error("capture target is not available in the current topology")]
    InvalidCaptureTarget,
}

/// No-op validator for use in tests.
#[cfg(test)]
#[derive(Debug)]
pub struct NoopStoragePathValidator;
#[cfg(test)]
impl StoragePathValidator for NoopStoragePathValidator {
    fn validate(&self, _path: &Path) -> Result<(), StoragePathError> {
        Ok(())
    }
}

/// Mutable state for the settings editor.
#[derive(Clone)]
pub struct EditorState {
    pub(crate) draft: SettingsSnapshot,
    pub(crate) capture_target: Option<CaptureTarget>,
    pub validation: ValidationState,
    pub revision: u64,
    pub(crate) catalog: FactoryCatalog,
    pub(crate) supported_sample_rates: Arc<[u32]>,
    pub(crate) storage_validator: Rc<dyn StoragePathValidator>,
    /// Cached snapshot, invalidated when revision advances.
    /// Stored as Arc to document intent for future copy-on-write optimization.
    pub(crate) cached_snapshot: Arc<SettingsSnapshot>,
}
