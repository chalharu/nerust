//! [`RomLoadTarget`] adapter over [`SessionHandle`].
//! Implements the loader-facing trait so `RomLoader`s stay decoupled from the session type.

use nerust_core_traits::identity::SystemId;
use nerust_gui_runtime::settings::SettingsSnapshot;
use nerust_system_traits::factory::load::{DynSystemLoadOptions, MediaObject, ResolvedLoadRequest};

use crate::{
    load::{RomLoadTarget, RomLoaderError, SystemActivationError},
    session::SessionHandle,
};

/// Narrow load-leg implementation behind the loader port.
impl super::ports::LoadPorts for SessionHandle {
    fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
        SessionHandle::default_load_options(self)
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

/// Narrow snapshot-leg implementation behind the loader port.
impl super::ports::SnapshotPort for SessionHandle {
    fn settings_snapshot(&self) -> &SettingsSnapshot {
        SessionHandle::settings_snapshot(self)
    }
}

/// Loader-facing view of a live session over the narrow load legs.
/// Orchestrates one activation plus load plus resume per call.
pub struct SessionLoadAdapter<'a, P: super::ports::LoadPorts + super::ports::SnapshotPort + ?Sized>
{
    port: &'a mut P,
}

impl<'a> SessionLoadAdapter<'a, SessionHandle> {
    /// Concrete constructor for the live session.
    pub fn new(session: &'a mut SessionHandle) -> Self {
        Self { port: session }
    }
}

impl<'a, P: super::ports::LoadPorts + super::ports::SnapshotPort + ?Sized>
    SessionLoadAdapter<'a, P>
{
    /// Narrow-port constructor; tests and frontends inject fakes here.
    pub fn from_port(port: &'a mut P) -> Self {
        Self { port }
    }
}

impl<P: super::ports::LoadPorts + super::ports::SnapshotPort + ?Sized> SessionLoadAdapter<'_, P> {
    /// One orchestrated load: activate the system, load the media, then resume.
    /// Errors short-circuit; the port stays borrowed for the whole sequence.
    pub fn activate_and_load(
        &mut self,
        system_id: &dyn SystemId,
        media: MediaObject,
        resolved: ResolvedLoadRequest,
    ) -> Result<(), RomLoaderError> {
        self.port.set_active_system(system_id).map_err(|error| {
            RomLoaderError::ActivationFailed {
                source: Box::new(error),
            }
        })?;
        self.port.load_resolved(media, resolved)?;
        self.port.resume();
        Ok(())
    }
}

impl<P: super::ports::LoadPorts + super::ports::SnapshotPort + ?Sized> RomLoadTarget
    for SessionLoadAdapter<'_, P>
{
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
    use crate::session::load_adapter::*;
    use crate::session::ports::{LoadPorts, SnapshotPort};

    struct FakePorts {
        calls: Vec<&'static str>,
    }

    impl LoadPorts for FakePorts {
        fn default_load_options(&self) -> Option<Box<dyn DynSystemLoadOptions>> {
            None
        }
        fn load_resolved(
            &mut self,
            _media: MediaObject,
            _resolved: ResolvedLoadRequest,
        ) -> Result<(), RomLoaderError> {
            self.calls.push("load_resolved");
            Ok(())
        }
        fn resume(&mut self) {
            self.calls.push("resume");
        }
        fn set_active_system(
            &mut self,
            _system_id: &dyn SystemId,
        ) -> Result<(), SystemActivationError> {
            self.calls.push("set_active_system");
            Ok(())
        }
    }

    impl SnapshotPort for FakePorts {
        fn settings_snapshot(&self) -> &SettingsSnapshot {
            panic!("no snapshot in fake")
        }
    }

    #[test]
    fn adapter_depends_on_narrow_ports_only() {
        fn bound<T: LoadPorts + SnapshotPort>() {}
        bound::<FakePorts>();
        bound::<super::super::SessionHandle>();
        let mut fake = FakePorts { calls: vec![] };
        let mut adapter = SessionLoadAdapter::from_port(&mut fake);
        adapter.resume();
        assert_eq!(fake.calls, vec!["resume"]);
    }

    #[test]
    fn activate_and_load_sequences_once_against_the_port() {
        use crate::session::test_util::{test_session, test_view};
        let mut session = test_session();
        let media = MediaObject::new(None, vec![]);
        {
            let adapter = SessionLoadAdapter::new(&mut session);
            assert!(adapter.default_load_options().is_some());
            let _ = adapter.settings_snapshot();
        }
        let mut fake = FakePorts { calls: vec![] };
        let mut adapter = SessionLoadAdapter::from_port(&mut fake);
        let id = crate::test_helpers::DummySystemId;
        let resolved = {
            let factory = session.active_factory_cloned().unwrap();
            let options = factory.default_load_options();
            let view = test_view(&session);
            factory.resolve_load_request(&view, options).unwrap()
        };
        assert!(adapter.activate_and_load(&id, media, resolved).is_ok());
        // One orchestrated pass: activate, then load, then resume.
        assert_eq!(
            fake.calls,
            vec!["set_active_system", "load_resolved", "resume"]
        );
    }

    #[test]
    fn session_ports_impl_delegates_without_recursion() {
        use crate::session::test_util::{test_session, test_view};
        let mut session = test_session();
        assert!(LoadPorts::default_load_options(&session).is_some());
        let _ = SnapshotPort::settings_snapshot(&session);
        LoadPorts::resume(&mut session);
        let id = crate::test_helpers::DummySystemId;
        assert!(LoadPorts::set_active_system(&mut session, &id).is_ok());
        let resolved = {
            let factory = session.active_factory_cloned().unwrap();
            let options = factory.default_load_options();
            let view = test_view(&session);
            factory.resolve_load_request(&view, options).unwrap()
        };
        LoadPorts::load_resolved(&mut session, MediaObject::new(None, vec![]), resolved)
            .expect("mock core accepts its resolved load request");
        assert!(session.tick_metrics().loaded);
    }
}
