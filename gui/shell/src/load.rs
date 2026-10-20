use std::path::Path;

use nerust_core_traits::{
    factory::load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
    identity::SystemId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;

#[derive(Debug, thiserror::Error)]
pub enum SystemActivationError {
    #[error("system not registered: {0}")]
    NotRegistered(Box<dyn SystemId>),
    #[error("system activation failed: {0}")]
    Activation(String),
}

#[derive(Debug, thiserror::Error)]
pub enum RomLoaderError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("ROM detection failed: {0}")]
    Detect(String),
    #[error("load request resolution failed: {0}")]
    Resolve(String),
    #[error("ROM load failed: {0}")]
    Load(String),
    /// Typed load failure: renders once at this boundary, keeping the `source` chain.
    /// Shaping matches [`Load`](Self::Load) so persisted logs and wording do not change.
    #[error("ROM load failed: {detail}")]
    LoadFailed {
        detail: String,
        #[source]
        source: Box<crate::session::SessionError>,
    },
}

impl RomLoaderError {
    /// Typed constructor for session-driven loads: renders the text
    /// once at the boundary and keeps the chain as `source`.
    pub fn from_session(source: crate::session::SessionError) -> Self {
        Self::LoadFailed {
            detail: source.to_string(),
            source: Box::new(source),
        }
    }
}

impl SystemActivationError {
    /// Typed constructor from a session error; keeps the chain renderable.
    pub fn from_session(source: crate::session::SessionError) -> Self {
        Self::Activation(source.to_string())
    }
}

/// Target for a ROM load operation.
/// Session operations needed by `RomLoader` implementations, for any session type.
pub trait RomLoadTarget {
    fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>>;
    fn settings_snapshot(&self) -> &SettingsSnapshot;
    fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError>;
    fn resume(&mut self);

    /// Notifies the target of the detected system for the ROM being loaded.
    /// Returns `Err` if the system is not recognised or activation fails.
    fn set_active_system(
        &mut self,
        _system_id: &dyn SystemId,
    ) -> Result<(), SystemActivationError> {
        Ok(())
    }
}

/// Loads a ROM file into a [`RomLoadTarget`] and resumes.
/// Implementors read the disk file, build the `MediaObject`, resolve options, and load.
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
    fn load_failed_keeps_source_chain_with_compat_text() {
        use super::RomLoaderError;
        use std::error::Error as _;
        let typed = RomLoaderError::from_session(crate::session::SessionError::NoCore);
        // Display matches the legacy `Load` wording.
        assert_eq!(
            typed.to_string(),
            "ROM load failed: no emulation core active"
        );
        // The chain is inspectable, not stringified away.
        assert!(typed.source().is_some());
        // Legacy construction still compiles for out-of-crate callers.
        let legacy = RomLoaderError::Load("no ROM in tests".to_string());
        assert!(legacy.to_string().contains("no ROM in tests"));
    }

    #[test]
    fn media_object_tracks_path_extension() {
        let media = MediaObject::new(Some(PathBuf::from("/tmp/test.NES")), vec![1, 2, 3]);

        assert_eq!(media.extension.as_deref(), Some("nes"));
        assert_eq!(media.bytes.as_ref(), [1, 2, 3]);
    }
}

#[cfg(test)]
mod activation_tests {
    use crate::load::*;

    #[test]
    fn activation_error_constructs_from_session() {
        let error = SystemActivationError::from_session(crate::session::SessionError::NoCore);
        assert!(matches!(error, SystemActivationError::Activation(_)));
    }
}
