//! Headless window opener: fails closed while recording requests.
//! Production code never touches this module.

use std::sync::{Arc, Mutex};

/// Recorded window-creation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenerCall {
    Main,
    Settings,
    Debug,
    Ppu,
    History,
}

/// `WindowOpener` that fails every creation closed and records calls. Aux handles are unbuildable
/// headless (they embed `TaoWindow`), so only the failure/skip paths are reachable — the honest maximum.
#[derive(Debug, Default)]
pub(crate) struct FakeOpener {
    calls: Mutex<Vec<OpenerCall>>,
    main_detail: Mutex<String>,
}

impl FakeOpener {
    pub(crate) fn failing(detail: &str) -> Self {
        let opener = Self::default();
        *crate::aux::bridge::lock_recovered(&opener.main_detail) = detail.to_string();
        opener
    }

    pub(crate) fn calls(&self) -> Vec<OpenerCall> {
        crate::aux::bridge::lock_recovered(&self.calls).clone()
    }
}

impl crate::ports::WindowOpener for FakeOpener {
    fn create_main(
        &self,
        _builder: tao::window::WindowBuilder,
    ) -> Result<Arc<tao::window::Window>, String> {
        crate::aux::bridge::lock_recovered(&self.calls).push(OpenerCall::Main);
        Err(crate::aux::bridge::lock_recovered(&self.main_detail).clone())
    }

    fn create_aux(
        &self,
        window: nerust_frontend_host::window::Window,
        _params: crate::ports::AuxOpenParams,
    ) -> Option<Box<dyn crate::aux::registry::AuxController>> {
        use crate::aux::focus::AuxKind;
        let nerust_frontend_host::window::WindowId::Auxiliary(kind) = window.id else {
            return None;
        };
        let call = match kind {
            AuxKind::Settings => OpenerCall::Settings,
            AuxKind::Debug => OpenerCall::Debug,
            AuxKind::Ppu => OpenerCall::Ppu,
            AuxKind::History => OpenerCall::History,
        };
        crate::aux::bridge::lock_recovered(&self.calls).push(call);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aux::focus::AuxKind;
    use crate::ports::{AuxOpenParams, WindowOpener};

    #[test]
    fn fake_opener_records_each_aux_kind() {
        use crate::aux::widgets::bindings::tests::{mailbox_of, prepare, screen_of};
        let opener = FakeOpener::failing("no display");
        let mut host = crate::runtime::host::TestHost::new().host;
        let id = prepare(&mut host, AuxKind::History);
        assert!(
            opener
                .create_aux(
                    host.aux.window(id).expect("window").clone(),
                    AuxOpenParams::Controls {
                        id,
                        screen: Box::new(screen_of(&host, id)),
                        mailbox: mailbox_of(&host, id),
                    }
                )
                .is_none()
        );
        assert!(
            opener
                .create_aux(
                    nerust_frontend_host::window::Window::auxiliary(AuxKind::Debug),
                    AuxOpenParams::Debug { position: None }
                )
                .is_none()
        );
        assert_eq!(opener.calls(), [OpenerCall::History, OpenerCall::Debug]);
    }

    #[test]
    fn fake_main_records_and_fails_closed() {
        use crate::ports::WindowOpener as _;
        let opener = FakeOpener::failing("no display");
        let builder = tao::window::WindowBuilder::new().with_title("test");
        assert!(opener.create_main(builder).is_err());
        assert_eq!(opener.calls(), [OpenerCall::Main]);
    }

    #[test]
    fn fake_opener_records_remaining_aux_kinds() {
        let opener = FakeOpener::failing("no display");
        use crate::aux::widgets::bindings::tests::{mailbox_of, prepare, screen_of};
        let mut host = crate::runtime::host::TestHost::new().host;
        let id = prepare(&mut host, AuxKind::Settings);
        assert!(
            opener
                .create_aux(
                    host.aux.window(id).expect("window").clone(),
                    AuxOpenParams::Controls {
                        id,
                        screen: Box::new(screen_of(&host, id)),
                        mailbox: mailbox_of(&host, id),
                    }
                )
                .is_none()
        );
        assert!(
            opener
                .create_aux(
                    nerust_frontend_host::window::Window::auxiliary(AuxKind::Ppu),
                    AuxOpenParams::Ppu { position: None }
                )
                .is_none()
        );
        assert_eq!(opener.calls(), [OpenerCall::Settings, OpenerCall::Ppu]);
    }

    #[test]
    fn fake_main_surfaces_the_failing_detail() {
        use crate::ports::WindowOpener as _;
        let opener = FakeOpener::failing("headless has no display");
        let builder = tao::window::WindowBuilder::new().with_title("test");
        let error = opener.create_main(builder).expect_err("fails closed");
        assert_eq!(error, "headless has no display");
    }
}
