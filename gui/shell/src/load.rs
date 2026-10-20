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
    /// Typed activation failure: renders once at this boundary, keeping the chain as `source`.
    /// Shaping matches the removed `Activation` spelling so persisted logs do not change.
    #[error("system activation failed: {source}")]
    ActivationFailed {
        #[source]
        source: Box<crate::session::SessionError>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum RomLoaderError {
    #[error("I/O error: {0}")]
    Io(String),
    #[error("ROM detection failed: {0}")]
    Detect(String),
    #[error("load request resolution failed: {0}")]
    Resolve(String),
    /// Typed load failure: renders once at this boundary, keeping the `source` chain.
    /// Shaping matches the removed `Load` spelling so persisted logs and wording do not change.
    #[error("ROM load failed: {source}")]
    LoadFailed {
        #[source]
        source: Box<crate::session::SessionError>,
    },
    /// Typed activation failure at the load boundary: renders once, keeping the chain as `source`.
    /// Shaping matches the removed `Load` spelling wrapping the activation error text.
    #[error("ROM load failed: {source}")]
    ActivationFailed {
        #[source]
        source: Box<SystemActivationError>,
    },
}

impl RomLoaderError {
    /// Typed constructor for session-driven loads: keeps the chain as `source`, renders once.
    pub fn from_session(source: crate::session::SessionError) -> Self {
        Self::LoadFailed {
            source: Box::new(source),
        }
    }
}

impl SystemActivationError {
    /// Typed constructor from a session error; keeps the chain renderable.
    pub fn from_session(source: crate::session::SessionError) -> Self {
        Self::ActivationFailed {
            source: Box::new(source),
        }
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
    #[test]
    fn load_failed_keeps_source_chain_with_compat_text() {
        use super::RomLoaderError;
        use std::error::Error as _;
        let typed = RomLoaderError::from_session(crate::session::SessionError::NoCore);
        // Display matches the removed `Load` wording.
        assert_eq!(
            typed.to_string(),
            "ROM load failed: no emulation core active"
        );
        // The chain is inspectable, not stringified away.
        assert!(typed.source().is_some());
    }
}

#[cfg(test)]
mod activation_tests {
    use crate::load::*;

    #[test]
    fn activation_error_constructs_from_session() {
        let error = SystemActivationError::from_session(crate::session::SessionError::NoCore);
        assert!(matches!(
            error,
            SystemActivationError::ActivationFailed { .. }
        ));
        assert_eq!(
            error.to_string(),
            "system activation failed: no emulation core active"
        );
    }
}
