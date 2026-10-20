//! [`RomLoadTarget`] adapter over [`SessionHandle`].
//! Implements the loader-facing trait so `RomLoader`s stay decoupled from the session type.

use nerust_core_traits::{
    factory::load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
    identity::SystemId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;

use crate::{
    load::{RomLoadTarget, RomLoaderError, SystemActivationError},
    session::{SessionHandle, commands::SessionCommand},
};

/// Loader-facing view of a live session. Borrows the session for the
/// duration of one load; frontends keep using the inherent methods.
pub struct SessionLoadAdapter<'a>(pub &'a mut SessionHandle);

impl<'a> SessionLoadAdapter<'a> {
    pub fn new(session: &'a mut SessionHandle) -> Self {
        Self(session)
    }
}

impl RomLoadTarget for SessionLoadAdapter<'_> {
    fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
        self.0.default_load_options()
    }

    fn settings_snapshot(&self) -> &SettingsSnapshot {
        self.0.settings_snapshot()
    }

    fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError> {
        self.0
            .load_resolved(media, resolved)
            .map_err(RomLoaderError::from_session)
    }

    fn resume(&mut self) {
        let _ = SessionHandle::run_command(self.0, SessionCommand::Resume);
    }

    fn set_active_system(&mut self, system_id: &dyn SystemId) -> Result<(), SystemActivationError> {
        self.0.set_active_system(system_id)
    }
}
