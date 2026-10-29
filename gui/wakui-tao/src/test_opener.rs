//! Headless window opener: fails closed while recording requests.
//! Production code never touches this module.

use std::sync::{Arc, Mutex};
use wakui_core::support::sync::lock_recovered;

/// Recorded window-creation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenerCall {
    Main,
    Alpha,
    Gamma,
    Delta,
    Beta,
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
        *lock_recovered(&opener.main_detail) = detail.to_string();
        opener
    }

    pub(crate) fn calls(&self) -> Vec<OpenerCall> {
        lock_recovered(&self.calls).clone()
    }
}

impl crate::ports::WindowOpener for FakeOpener {
    fn create_main(
        &self,
        _builder: tao::window::WindowBuilder,
    ) -> Result<Arc<tao::window::Window>, String> {
        lock_recovered(&self.calls).push(OpenerCall::Main);
        Err(lock_recovered(&self.main_detail).clone())
    }

    fn create_aux(
        &self,
        window: wakui_core::chrome::window::Window,
        _params: crate::ports::AuxOpenParams,
    ) -> Option<Box<dyn crate::aux_windows::registry::AuxController>> {
        use crate::testkit::TestAux;
        let wakui_core::chrome::window::WindowId::Auxiliary(_) = window.id else {
            return None;
        };
        // Titles are view-side; the headless fake maps them back to calls.
        let call = match window.title.as_str() {
            title if title == TestAux::Alpha.title() => OpenerCall::Alpha,
            title if title == TestAux::Gamma.title() => OpenerCall::Gamma,
            title if title == TestAux::Delta.title() => OpenerCall::Delta,
            title if title == TestAux::Beta.title() => OpenerCall::Beta,
            _ => return None,
        };
        lock_recovered(&self.calls).push(call);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{AuxOpenParams, WindowOpener};
    use crate::testkit::TestAux;

    #[test]
    fn fake_opener_records_each_aux_kind() {
        use crate::aux_windows::widgets::bindings::tests::{mailbox_of, prepare, screen_of};
        let opener = FakeOpener::failing("no display");
        let mut host = crate::event_loop::host::TestHost::new().host;
        let id = prepare(&mut host, TestAux::Beta);
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
        let debug_id = prepare(&mut host, TestAux::Gamma);
        assert!(
            opener
                .create_aux(
                    host.aux.window(debug_id).expect("window").clone(),
                    AuxOpenParams::Controls {
                        id: debug_id,
                        screen: Box::new(screen_of(&host, debug_id)),
                        mailbox: mailbox_of(&host, debug_id),
                    }
                )
                .is_none()
        );
        assert_eq!(opener.calls(), [OpenerCall::Beta, OpenerCall::Gamma]);
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
        use crate::aux_windows::widgets::bindings::tests::{mailbox_of, prepare, screen_of};
        let mut host = crate::event_loop::host::TestHost::new().host;
        let id = prepare(&mut host, TestAux::Alpha);
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
        let ppu_id = prepare(&mut host, TestAux::Delta);
        assert!(
            opener
                .create_aux(
                    host.aux.window(ppu_id).expect("window").clone(),
                    AuxOpenParams::Controls {
                        id: ppu_id,
                        screen: Box::new(screen_of(&host, ppu_id)),
                        mailbox: mailbox_of(&host, ppu_id),
                    }
                )
                .is_none()
        );
        assert_eq!(opener.calls(), [OpenerCall::Alpha, OpenerCall::Delta]);
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
