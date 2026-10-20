//! [`RomLoadTarget`] adapter over [`SessionHandle`].
//! Implements the loader-facing trait so `RomLoader`s stay decoupled from the session type.

use nerust_core_traits::{
    factory::load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest},
    identity::SystemId,
};
use nerust_gui_runtime::settings::SettingsSnapshot;

use crate::{
    load::{RomLoadTarget, RomLoaderError, SystemActivationError},
    session::SessionHandle,
};

/// SessionPorts implementation behind the narrow loader port.
impl super::ports::SessionPorts for SessionHandle {
    fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
        SessionHandle::default_load_options(self)
    }

    fn settings_snapshot(&self) -> &SettingsSnapshot {
        SessionHandle::settings_snapshot(self)
    }

    fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError> {
        SessionHandle::load_resolved(self, media, resolved).map_err(RomLoaderError::from_session)
    }

    fn resume(&mut self) {
        let _ = SessionHandle::run_command(self, crate::session::commands::SessionCommand::Resume);
    }

    fn set_active_system(&mut self, system_id: &dyn SystemId) -> Result<(), SystemActivationError> {
        SessionHandle::set_active_system(self, system_id)
    }
}

/// Loader-facing view of a live session. Borrows the port for the
/// duration of one load; frontends keep using the inherent methods.
pub struct SessionLoadAdapter<'a> {
    port: &'a mut dyn super::ports::SessionPorts,
}

impl<'a> SessionLoadAdapter<'a> {
    /// Concrete constructor for the live session.
    pub fn new(session: &'a mut SessionHandle) -> Self {
        Self { port: session }
    }

    /// Narrow-port constructor; tests and frontends inject fakes here.
    pub fn from_port(port: &'a mut dyn super::ports::SessionPorts) -> Self {
        Self { port }
    }
}

impl RomLoadTarget for SessionLoadAdapter<'_> {
    fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
        self.port.default_load_options()
    }

    fn settings_snapshot(&self) -> &SettingsSnapshot {
        self.port.settings_snapshot()
    }

    fn load_resolved(
        &mut self,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError> {
        self.port.load_resolved(media, resolved)
    }

    fn resume(&mut self) {
        self.port.resume();
    }

    fn set_active_system(&mut self, system_id: &dyn SystemId) -> Result<(), SystemActivationError> {
        self.port.set_active_system(system_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakePorts {
        resumed: bool,
    }

    impl super::super::ports::SessionPorts for FakePorts {
        fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
            None
        }
        fn settings_snapshot(&self) -> &SettingsSnapshot {
            panic!("no snapshot in fake")
        }
        fn load_resolved(
            &mut self,
            _media: MediaObject,
            _resolved: ResolvedLoadRequest,
        ) -> Result<(), RomLoaderError> {
            Ok(())
        }
        fn resume(&mut self) {
            self.resumed = true;
        }
        fn set_active_system(
            &mut self,
            _system_id: &dyn SystemId,
        ) -> Result<(), SystemActivationError> {
            Ok(())
        }
    }

    #[test]
    fn adapter_depends_on_narrow_ports_only() {
        let mut fake = FakePorts { resumed: false };
        let mut adapter = SessionLoadAdapter::from_port(&mut fake);
        adapter.resume();
        assert!(fake.resumed);
    }
}
